//! End-to-end tests against a local hyper test server.
//!
//! These cover three scenarios:
//! - a full download whose sha256 matches the source bytes,
//! - resume after the server cuts a segment mid-stream,
//! - graceful fallback to single-stream when the server ignores Range and
//!   returns 200 instead of 206.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use engine::{
    download, download_with_control, resume_at, Backoff, CancellationToken, Control,
    DownloadOptions, ProgressEvent, SegmentRuntimeState, DEFAULT_CHANNEL_CAPACITY,
};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::net::TcpListener;
use url::Url;

/// 1 MiB of pseudo-random bytes.
fn payload() -> Vec<u8> {
    // Deterministic so we can assert exact hashes if needed.
    let mut buf = Vec::with_capacity(1 << 20);
    let mut x: u32 = 0x1234_5678;
    while buf.len() < (1 << 20) {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf.truncate(1 << 20);
    buf
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

#[derive(Clone, Copy, Debug)]
enum ServerMode {
    /// Range-aware, healthy.
    RangeOk,
    /// Range header sent but the server returns 200 with the full body.
    IgnoreRangeReturn200,
    /// Range-aware, but the FIRST request to each segment is cut short after
    /// `cut_after` bytes by closing the body early. Subsequent requests to
    /// the same range succeed normally — this is what resume should recover
    /// from.
    CutFirstRequestPerSegment { cut_after: usize },
    /// Range-aware, but each non-HEAD response is delayed by `delay_ms`
    /// before any bytes are sent — gives a cancellation test enough
    /// wall-clock to actually fire mid-transfer.
    Slow { delay_ms: u64 },
    /// One-time / session-bound link: the FIRST request (any method)
    /// returns the full body; every later request returns an empty 200.
    /// No `Accept-Ranges` — a single-stream, browser-like host. Models
    /// one-click file hosts (e.g. fuckingfast.co) where a HEAD probe
    /// would spend the token before the real GET.
    SingleUseToken,
    /// Always 404 — exercises the non-success mapping in `initial_get`
    /// before any file is created on disk.
    NotFound,
    /// Range-capable, but refuses ranged GETs whose start offset is > 0
    /// with 403 — mimics a per-file concurrent-connection cap (pixeldrain
    /// behind Cloudflare): the verify probe (`bytes=0-0`) and a single
    /// connection succeed, but the parallel offset segments are rejected.
    RejectOffsetRanges,
}

#[derive(Clone)]
struct ServerState {
    payload: Arc<Vec<u8>>,
    mode: ServerMode,
    requests: Arc<AtomicUsize>,
    // For cut mode: track segments we've already cut. Keyed by the
    // requested range's `end` byte — that stays constant across a
    // segment's retries while `start` advances with each successful
    // partial read.
    failed_segments: Arc<tokio::sync::Mutex<std::collections::HashSet<u64>>>,
}

const ETAG_VALUE: &str = "\"unduhin-test-etag\"";
const LAST_MODIFIED_VALUE: &str = "Wed, 21 Oct 2026 07:28:00 GMT";

async fn handle(
    req: Request<Incoming>,
    state: ServerState,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let req_no = state.requests.fetch_add(1, Ordering::SeqCst);
    let total = state.payload.len() as u64;
    let mut resp = Response::builder()
        .header(ETAG, ETAG_VALUE)
        .header(LAST_MODIFIED, LAST_MODIFIED_VALUE);

    // Range-aware modes advertise range support; `IgnoreRangeReturn200`
    // lies about it (Range requests get full-body 200s). `SingleUseToken`
    // and `NotFound` are single-stream and deliberately omit
    // `Accept-Ranges`.
    if !matches!(state.mode, ServerMode::SingleUseToken | ServerMode::NotFound) {
        resp = resp.header(ACCEPT_RANGES, "bytes");
    }

    if req.method() == Method::HEAD {
        let r = resp
            .status(StatusCode::OK)
            .header(CONTENT_LENGTH, total)
            .body(Full::new(Bytes::new()))
            .unwrap();
        return Ok(r);
    }

    if let ServerMode::Slow { delay_ms } = state.mode {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    let range_hdr = req
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    match (range_hdr.as_deref(), state.mode) {
        (_, ServerMode::NotFound) => {
            let r = resp
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::new()))
                .unwrap();
            Ok(r)
        }
        (_, ServerMode::SingleUseToken) => {
            // The token is spent by the first request (whatever the
            // method). A second request — e.g. a transfer GET after a HEAD
            // probe spent the token — sees an empty 200, which the
            // completion gate would reject as 0 bytes.
            let body = if req_no == 0 {
                Bytes::from(state.payload.as_ref().clone())
            } else {
                Bytes::new()
            };
            // The real filename lives only in the GET's Content-Disposition
            // (fuckingfast.co's shape): the URL path is an opaque token, so
            // this header is the engine's one chance to learn the name.
            let r = resp
                .status(StatusCode::OK)
                .header(CONTENT_LENGTH, body.len() as u64)
                .header(CONTENT_DISPOSITION, "attachment; filename=\"real-movie.mkv\"")
                .body(Full::new(body))
                .unwrap();
            Ok(r)
        }
        (Some(r), ServerMode::RejectOffsetRanges) => {
            let Some((start, end)) = parse_range(r, total) else {
                let r = resp
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                return Ok(r);
            };
            if start > 0 {
                // Concurrent offset segment — refuse it. The first segment
                // (start 0) and the verify probe (bytes=0-0) are allowed.
                let r = resp
                    .status(StatusCode::FORBIDDEN)
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                return Ok(r);
            }
            let slice = state.payload[start as usize..=end as usize].to_vec();
            let r = resp
                .status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_LENGTH, slice.len() as u64)
                .header(CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total))
                .body(Full::new(Bytes::from(slice)))
                .unwrap();
            Ok(r)
        }
        (Some(r), ServerMode::IgnoreRangeReturn200) => {
            // Server silently returns the whole body. The engine should
            // detect this during pre-flight verification and fall back to
            // single-stream mode (one segment).
            let _ = r;
            let r = resp
                .status(StatusCode::OK)
                .header(CONTENT_LENGTH, total)
                .body(Full::new(Bytes::from(state.payload.as_ref().clone())))
                .unwrap();
            Ok(r)
        }
        (Some(r), ServerMode::RangeOk)
        | (Some(r), ServerMode::CutFirstRequestPerSegment { .. })
        | (Some(r), ServerMode::Slow { .. }) => {
            let Some((start, end)) = parse_range(r, total) else {
                let r = resp
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                return Ok(r);
            };
            let slice = state.payload[start as usize..=end as usize].to_vec();

            let trimmed = if let ServerMode::CutFirstRequestPerSegment { cut_after } = state.mode {
                let mut fr = state.failed_segments.lock().await;
                if !fr.contains(&end) && slice.len() > cut_after {
                    fr.insert(end);
                    slice[..cut_after].to_vec()
                } else {
                    slice
                }
            } else {
                slice
            };

            let r = resp
                .status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_LENGTH, trimmed.len() as u64)
                .header(CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total))
                .body(Full::new(Bytes::from(trimmed)))
                .unwrap();
            Ok(r)
        }
        (None, _) => {
            // No Range header: full response.
            let r = resp
                .status(StatusCode::OK)
                .header(CONTENT_LENGTH, total)
                .body(Full::new(Bytes::from(state.payload.as_ref().clone())))
                .unwrap();
            Ok(r)
        }
    }
}

fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let rest = value.strip_prefix("bytes=")?;
    let (s, e) = rest.split_once('-')?;
    let start: u64 = s.parse().ok()?;
    let end: u64 = if e.is_empty() {
        total.saturating_sub(1)
    } else {
        e.parse().ok()?
    };
    if start > end || end >= total {
        return None;
    }
    Some((start, end))
}

struct TestServer {
    addr: SocketAddr,
    state: ServerState,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start(mode: ServerMode) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = ServerState {
            payload: Arc::new(payload()),
            mode,
            requests: Arc::new(AtomicUsize::new(0)),
            failed_segments: Arc::new(tokio::sync::Mutex::new(Default::default())),
        };
        let st = state.clone();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, _)) => {
                                let st = st.clone();
                                tokio::spawn(async move {
                                    let io = TokioIo::new(stream);
                                    let svc = service_fn(move |req| {
                                        let st = st.clone();
                                        async move { handle(req, st).await }
                                    });
                                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });
        Ok(Self {
            addr,
            state,
            shutdown: tx,
            handle,
        })
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{}", self.addr, path)).unwrap()
    }

    fn payload(&self) -> &[u8] {
        &self.state.payload
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }
}

fn opts_for(url: Url, output: PathBuf, segments: usize) -> DownloadOptions {
    let mut o = DownloadOptions::new(url, output);
    o.segments = segments;
    o.connect_timeout = Duration::from_secs(5);
    o.read_timeout = Duration::from_secs(5);
    o.backoff = Backoff {
        base: Duration::from_millis(20),
        cap: Duration::from_millis(100),
        max_attempts: 5,
    };
    o
}

#[tokio::test]
async fn full_download_matches_sha256() -> Result<()> {
    let server = TestServer::start(ServerMode::RangeOk).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("file.bin");
    let opts = opts_for(server.url("/file.bin"), out.clone(), 4);

    let summary = download(opts, CancellationToken::new(), None).await?;
    assert_eq!(summary.bytes, server.payload().len() as u64);
    // Slow-start: a tiny local file finishes before the ramp adds any
    // connections, so it stays at one segment. Multi-segment assembly is
    // covered by `re_segments_*` and the ramp tests.
    assert!(summary.segments >= 1);
    assert!(!summary.resumed);

    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));

    // Sidecar should be gone on successful completion.
    assert!(!engine::Meta::sidecar_path(&out).exists());

    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn resume_after_midstream_disconnect() -> Result<()> {
    // Cut the FIRST request to every segment after 4 KiB, so each segment
    // has to be retried at least once. Engine's worker retry loop should
    // handle this without resume_at being involved.
    let server = TestServer::start(ServerMode::CutFirstRequestPerSegment {
        cut_after: 4 * 1024,
    })
    .await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("flaky.bin");
    let opts = opts_for(server.url("/flaky.bin"), out.clone(), 4);

    let summary = download(opts, CancellationToken::new(), None).await?;
    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));
    assert_eq!(summary.bytes, server.payload().len() as u64);
    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn resume_across_explicit_restart() -> Result<()> {
    // Start a download against a slow server, then cancel before
    // any segment finishes. The sidecar must be left behind so resume_at
    // can pick up the partial state.
    let server = TestServer::start(ServerMode::Slow { delay_ms: 200 }).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("resumable.bin");

    {
        let opts = opts_for(server.url("/r.bin"), out.clone(), 4);
        let cancel = CancellationToken::new();
        let cancel_for_kill = cancel.clone();
        let kill = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_for_kill.cancel();
        });
        let _ = download(opts, cancel, None).await;
        let _ = kill.await;
    }

    let meta_path = engine::Meta::sidecar_path(&out);
    assert!(meta_path.exists(), "expected sidecar after cancel");

    // Resume from the sidecar.
    let summary = resume_at(
        meta_path.clone(),
        Backoff {
            base: Duration::from_millis(20),
            cap: Duration::from_millis(100),
            max_attempts: 5,
        },
        Duration::from_secs(5),
        Duration::from_secs(5),
        None,
        Vec::new(),
        CancellationToken::new(),
        None,
    )
    .await?;
    assert!(summary.resumed);

    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));
    assert!(!meta_path.exists());
    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn segment_progress_carries_speed_state_and_eventual_done() -> Result<()> {
    // Slow mode delays every response by 50 ms so the ticker (250 ms
    // cadence) has time to observe non-zero per-worker speeds before
    // the transfer completes.
    let server = TestServer::start(ServerMode::Slow { delay_ms: 50 }).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("telemetry.bin");
    let opts = opts_for(server.url("/telemetry.bin"), out.clone(), 4);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<ProgressEvent>(DEFAULT_CHANNEL_CAPACITY);
    let collector = tokio::spawn(async move {
        let mut events = Vec::new();
        loop {
            match rx.recv().await {
                Ok(ev) => events.push(ev),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
        events
    });

    let summary = download(opts, CancellationToken::new(), Some(tx.clone())).await?;
    drop(tx);
    let events = collector.await?;

    // Regression check: byte-exact assembly under the new worker-queue
    // model. Catches any drift introduced by the refactor.
    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));
    assert!(summary.segments >= 1);

    let segment_events: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::SegmentProgress {
                index,
                speed_bps,
                state,
                bytes_downloaded,
                segment_total,
            } => Some((
                *index,
                *speed_bps,
                *state,
                *bytes_downloaded,
                *segment_total,
            )),
            _ => None,
        })
        .collect();

    assert!(
        !segment_events.is_empty(),
        "expected at least one SegmentProgress event"
    );

    // The new wire shape must carry plausible per-segment values:
    // speed_bps is finite and non-negative, total matches the planned
    // segment size, and bytes_downloaded never exceeds total.
    for (idx, speed, _state, bd, total) in &segment_events {
        assert!(
            speed.is_finite() && *speed >= 0.0,
            "segment {idx} speed not finite/non-negative: {speed}",
        );
        assert!(
            *bd <= *total,
            "segment {idx} bytes_downloaded {bd} exceeds segment_total {total}",
        );
    }

    // Every segment index must eventually emit Done. The ticker may
    // race against worker completion, so run_transfer also emits a
    // synthetic final Done per segment — this assertion locks both
    // paths in.
    let mut done_seen = vec![false; summary.segments];
    for (idx, _, state, _, _) in &segment_events {
        if matches!(state, SegmentRuntimeState::Done) {
            if let Some(slot) = done_seen.get_mut(*idx) {
                *slot = true;
            }
        }
    }
    assert!(
        done_seen.iter().all(|b| *b),
        "expected Done for all {} segments, got {done_seen:?}",
        summary.segments
    );

    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn re_segments_mid_flight_preserves_sha256() -> Result<()> {
    // Slow mode with a longer per-response delay gives the test enough
    // wall-clock to fire two SetSegments commands while the transfer is
    // still in flight. The server applies the delay once per request, so
    // each round-trip is ~500 ms and a 4 -> 8 split that re-issues a
    // Range request adds another ~500 ms.
    let server = TestServer::start(ServerMode::Slow { delay_ms: 500 }).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("reseg.bin");
    let opts = opts_for(server.url("/reseg.bin"), out.clone(), 4);

    let (ctrl_tx, ctrl_rx) = tokio::sync::mpsc::channel::<Control>(8);
    let (ev_tx, _ev_rx) =
        tokio::sync::broadcast::channel::<ProgressEvent>(DEFAULT_CHANNEL_CAPACITY);

    // Fire SetSegments on a fixed timeline rather than tying it to Tick
    // events — the ticker may not have fired yet when the body arrives.
    let ctrl_for_driver = ctrl_tx.clone();
    let driver = tokio::spawn(async move {
        // Wait for the initial probe + verify_range_support + first body
        // dispatch to land before re-segmenting.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut grew = false;
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        if ctrl_for_driver
            .send(Control::SetSegments { n: 8, ack: ack_tx })
            .await
            .is_ok()
        {
            let _ = ack_rx.await;
            grew = true;
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut shrunk = false;
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        if ctrl_for_driver
            .send(Control::SetSegments { n: 4, ack: ack_tx })
            .await
            .is_ok()
        {
            let _ = ack_rx.await;
            shrunk = true;
        }
        (grew, shrunk)
    });

    let summary =
        download_with_control(opts, CancellationToken::new(), Some(ev_tx), Some(ctrl_rx)).await?;
    drop(ctrl_tx);
    let (grew, _shrunk) = driver.await?;

    let total_bytes = server.payload().len() as u64;

    // Byte-exact assembly is the load-bearing assertion.
    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));
    assert_eq!(summary.bytes, total_bytes);
    // No-delete-on-join policy: meta.segments grows monotonically; the
    // summary's `segments` field reflects the final meta length, which
    // equals the high watermark reached during split.
    assert!(
        summary.segments >= 4,
        "expected at least 4 segments in summary (no-delete policy), got {}",
        summary.segments
    );
    // Sanity: the grow command actually fired. (We can't always observe
    // the shrink because the transfer may complete before the second
    // timer fires; that's OK — the SHA assertion is the truth, and the
    // grow path is the harder one to validate.)
    assert!(grew, "expected at least the 4 -> 8 split to fire");

    server.stop().await;
    Ok(())
}

/// A captured browser request header (`X-Foo: bar`) flows
/// through `build_client`'s `extra_headers` and reaches the server on the
/// HEAD probe. This is the only end-to-end coverage for the captured-
/// headers path until the native host + pipe server land in 8b/8c.
#[tokio::test]
async fn build_client_forwards_extra_headers_on_probe() -> Result<()> {
    let observed = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        String,
        String,
    >::new()));
    let observed_for_handler = observed.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accept = listener.accept() => {
                    let Ok((stream, _)) = accept else { break };
                    let observed = observed_for_handler.clone();
                    tokio::spawn(async move {
                        let io = TokioIo::new(stream);
                        let svc = service_fn(move |req: Request<Incoming>| {
                            let observed = observed.clone();
                            async move {
                                let mut snap = observed.lock().await;
                                for (name, value) in req.headers() {
                                    if let Ok(v) = value.to_str() {
                                        snap.insert(name.as_str().to_string(), v.to_string());
                                    }
                                }
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(StatusCode::OK)
                                        .header(CONTENT_LENGTH, 0)
                                        .header(ACCEPT_RANGES, "bytes")
                                        .body(Full::new(Bytes::new()))
                                        .unwrap(),
                                )
                            }
                        });
                        let _ = http1::Builder::new().serve_connection(io, svc).await;
                    });
                }
            }
        }
    });

    let extra = vec![
        ("X-Foo".to_string(), "bar".to_string()),
        ("Cookie".to_string(), "session=abc".to_string()),
        // Must be dropped by sanitize_headers — confirms the drop-list
        // wins even when callers pass disallowed names.
        ("Range".to_string(), "bytes=0-1".to_string()),
    ];
    let client = engine::http::build_client(
        Duration::from_secs(5),
        Duration::from_secs(5),
        Some("unduhin-test/1.0"),
        &extra,
    )?;
    let url: Url = format!("http://{addr}/probe").parse().unwrap();
    let info = engine::probe(&client, &url).await?;

    // Probe succeeded — server saw the request.
    assert!(info.accept_ranges);

    let snap = observed.lock().await;
    assert_eq!(snap.get("x-foo").map(String::as_str), Some("bar"));
    assert_eq!(snap.get("cookie").map(String::as_str), Some("session=abc"),);
    // Drop-list applied — captured Range did not leak into the HEAD probe.
    assert!(
        !snap.contains_key("range"),
        "captured Range header must be filtered by sanitize_headers, saw {:?}",
        snap.get("range")
    );
    drop(snap);

    let _ = shutdown_tx.send(());
    let _ = server.await;
    Ok(())
}

#[tokio::test]
async fn falls_back_to_single_stream_on_200() -> Result<()> {
    let server = TestServer::start(ServerMode::IgnoreRangeReturn200).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("fallback.bin");
    let opts = opts_for(server.url("/f.bin"), out.clone(), 8);

    let summary = download(opts, CancellationToken::new(), None).await?;
    assert_eq!(summary.bytes, server.payload().len() as u64);
    // Pre-flight verification should have collapsed to a single segment.
    assert_eq!(summary.segments, 1);

    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));
    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn single_use_token_downloads_in_one_get() -> Result<()> {
    // The regression test for one-click / one-time-link hosts: the engine
    // must fetch with a single browser-like GET, never a HEAD probe that
    // would spend the token and leave the real GET empty.
    let server = TestServer::start(ServerMode::SingleUseToken).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("token.bin");
    let opts = opts_for(server.url("/dl/token"), out.clone(), 8);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<ProgressEvent>(DEFAULT_CHANNEL_CAPACITY);
    let collector = tokio::spawn(async move {
        let mut events = Vec::new();
        loop {
            match rx.recv().await {
                Ok(ev) => events.push(ev),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
        events
    });

    let summary = download(opts, CancellationToken::new(), Some(tx.clone())).await?;
    drop(tx);
    let events = collector.await?;
    assert_eq!(summary.bytes, server.payload().len() as u64);
    // No Accept-Ranges → single-stream, streamed from the initial GET.
    assert_eq!(summary.segments, 1);
    // The filename the engine learned from the download response's
    // Content-Disposition rides out on the summary so the queue can rename
    // the slug-named file. This is the fix for the DDL filename symptom.
    assert_eq!(summary.filename_hint.as_deref(), Some("real-movie.mkv"));
    // And it is surfaced *mid-flight* as a `FilenameLearned` event (right
    // after the headers) so the UI can show the real name while downloading.
    let learned: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::FilenameLearned { hint } => Some(hint.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        learned,
        vec!["real-movie.mkv"],
        "expected one mid-flight FilenameLearned event with the real name"
    );

    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));

    // The crux: exactly one request. A HEAD-first probe would be request
    // #1 (spending the token) and the transfer GET request #2 (empty body
    // → 0 bytes → completion-gate failure).
    assert_eq!(
        server.state.requests.load(Ordering::SeqCst),
        1,
        "expected exactly one request (no HEAD probe)"
    );

    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn offset_range_rejection_still_completes() -> Result<()> {
    // The pixeldrain/Cloudflare shape: range-capable, the verify probe
    // (bytes=0-0) and a single connection are fine, but offset ranges are
    // 403'd (per-file connection cap). Slow-start covers this for free: the
    // first connection covers the whole file from offset 0, and any ramp
    // probe for an offset range is refused, so the download completes on a
    // single connection (byte-exact) rather than failing.
    let server = TestServer::start(ServerMode::RejectOffsetRanges).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("rejected.bin");
    let opts = opts_for(server.url("/r.bin"), out.clone(), 8);

    let summary = download(opts, CancellationToken::new(), None).await?;
    assert_eq!(summary.bytes, server.payload().len() as u64);
    assert_eq!(summary.segments, 1, "completes on a single connection");

    let bytes = std::fs::read(&out)?;
    assert_eq!(sha256_hex(&bytes), sha256_hex(server.payload()));
    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn not_found_maps_to_error_without_creating_file() -> Result<()> {
    let server = TestServer::start(ServerMode::NotFound).await?;
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("missing.bin");
    let opts = opts_for(server.url("/missing.bin"), out.clone(), 4);

    let result = download(opts, CancellationToken::new(), None).await;
    assert!(result.is_err(), "404 must fail the download");
    assert!(
        !out.exists(),
        "no output file should be created when the initial GET fails"
    );

    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn captured_user_agent_wins_over_engine_default() -> Result<()> {
    // Tier 2B: with no explicit UA override, the browser's captured
    // User-Agent (forwarded via extra_headers) must be the one actually
    // sent — not the engine's compiled-in default — and sent exactly once.
    // This keeps UA-bound anti-bot cookies (e.g. cf_clearance) valid.
    let observed = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let observed_for_handler = observed.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accept = listener.accept() => {
                    let Ok((stream, _)) = accept else { break };
                    let observed = observed_for_handler.clone();
                    tokio::spawn(async move {
                        let io = TokioIo::new(stream);
                        let svc = service_fn(move |req: Request<Incoming>| {
                            let observed = observed.clone();
                            async move {
                                let mut snap = observed.lock().await;
                                for v in req.headers().get_all(hyper::header::USER_AGENT) {
                                    if let Ok(s) = v.to_str() {
                                        snap.push(s.to_string());
                                    }
                                }
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(StatusCode::OK)
                                        .header(CONTENT_LENGTH, 0)
                                        .header(ACCEPT_RANGES, "bytes")
                                        .body(Full::new(Bytes::new()))
                                        .unwrap(),
                                )
                            }
                        });
                        let _ = http1::Builder::new().serve_connection(io, svc).await;
                    });
                }
            }
        }
    });

    let browser_ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120";
    let extra = vec![("User-Agent".to_string(), browser_ua.to_string())];
    // `user_agent` param is None — the captured header must win.
    let client =
        engine::http::build_client(Duration::from_secs(5), Duration::from_secs(5), None, &extra)?;
    let url: Url = format!("http://{addr}/probe").parse().unwrap();
    let _ = engine::probe(&client, &url).await?;

    let snap = observed.lock().await;
    assert_eq!(
        snap.as_slice(),
        std::slice::from_ref(&browser_ua.to_string()),
        "exactly one User-Agent, the captured browser one (got {snap:?})"
    );
    drop(snap);

    let _ = shutdown_tx.send(());
    let _ = server.await;
    Ok(())
}
