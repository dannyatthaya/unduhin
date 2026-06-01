//! End-to-end tests for `unduhin-core`.
//!
//! These cover the four core scenarios:
//! - migrations apply cleanly on a fresh database,
//! - the queue respects `max_concurrent_downloads` under load,
//! - pause-then-resume across a `Core` shutdown actually resumes,
//! - auto-categorization picks the right category by extension.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use chrono::Timelike;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, RANGE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use unduhin_core::settings::settings_keys;
use unduhin_core::{
    AddDownload, CategorySelector, Core, CoreEvent, DownloadFilter, DownloadSource, NewSchedule,
    ScheduleKind, SettingValue, Status,
};
use url::Url;

// Local hyper test server (same shape as the engine integration tests).

fn payload(size: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(size);
    let mut x: u32 = 0xCAFE_BABE;
    while buf.len() < size {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf.truncate(size);
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

/// Range-aware test server that streams the response in `chunk_size`-byte
/// pieces and sleeps `delay_ms` between each. Using a streaming body is
/// the only reliable way to pause mid-flight on localhost; a one-shot
/// body finishes before the first tick can fire. Set `delay_ms = 0` for
/// the "as fast as possible" case.
#[derive(Clone, Copy, Debug)]
struct ServerMode {
    delay_ms: u64,
    chunk_size: usize,
}

#[derive(Clone)]
struct ServerState {
    payload: Arc<Vec<u8>>,
    mode: ServerMode,
    requests: Arc<AtomicUsize>,
}

type Body = BoxBody<Bytes, Infallible>;

fn full_body(bytes: Bytes) -> Body {
    Full::new(bytes).map_err(|_| unreachable!()).boxed()
}

fn streamed_body(bytes: Vec<u8>, chunk_size: usize, delay: Duration) -> Body {
    let chunks: Vec<Bytes> = bytes
        .chunks(chunk_size.max(1))
        .map(Bytes::copy_from_slice)
        .collect();
    let stream = futures::stream::unfold(chunks.into_iter(), move |mut it| async move {
        let next = it.next()?;
        tokio::time::sleep(delay).await;
        Some((Ok::<_, Infallible>(Frame::data(next)), it))
    });
    StreamBody::new(stream).boxed()
}

async fn handle(
    req: Request<Incoming>,
    state: ServerState,
) -> std::result::Result<Response<Body>, Infallible> {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let total = state.payload.len() as u64;

    // Bug-repro paths (KNOWN_BUGS #2/#3): one-click file hosts answer the
    // captured bare URL with an empty body or an HTML landing page instead
    // of the real file. Both HEAD (the engine's probe) and GET must report
    // the same thing, so these branches ignore the method and return a
    // uniform response.
    match req.uri().path() {
        // 200 OK, Content-Length: 0, no body — the "expired token" case.
        "/empty" => {
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_LENGTH, 0u64)
                .body(full_body(Bytes::new()))
                .unwrap());
        }
        // 200 OK with an HTML interstitial where a file was expected.
        "/landing" => {
            let html =
                b"<!doctype html><html><body>click to download</body></html>".to_vec();
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/html; charset=utf-8")
                .header(CONTENT_LENGTH, html.len() as u64)
                .body(full_body(Bytes::from(html)))
                .unwrap());
        }
        _ => {}
    }

    let resp = Response::builder()
        .header(ETAG, "\"unduhin-core-test\"")
        .header(LAST_MODIFIED, "Wed, 21 Oct 2026 07:28:00 GMT")
        .header(ACCEPT_RANGES, "bytes");

    if req.method() == Method::HEAD {
        let r = resp
            .status(StatusCode::OK)
            .header(CONTENT_LENGTH, total)
            .body(full_body(Bytes::new()))
            .unwrap();
        return Ok(r);
    }

    let range_hdr = req
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let (status, slice) = if let Some(r) = range_hdr.as_deref() {
        let Some((start, end)) = parse_range(r, total) else {
            return Ok(resp
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .body(full_body(Bytes::new()))
                .unwrap());
        };
        let slice = state.payload[start as usize..=end as usize].to_vec();
        let resp = resp
            .header(CONTENT_LENGTH, slice.len() as u64)
            .header(CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total));
        (resp.status(StatusCode::PARTIAL_CONTENT), slice)
    } else {
        let resp = resp.header(CONTENT_LENGTH, total);
        (resp.status(StatusCode::OK), state.payload.as_ref().clone())
    };

    let body = streamed_body(
        slice,
        state.mode.chunk_size,
        Duration::from_millis(state.mode.delay_ms),
    );
    Ok(status.body(body).unwrap())
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
    payload: Arc<Vec<u8>>,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start(mode: ServerMode, size: usize) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let payload = Arc::new(payload(size));
        let state = ServerState {
            payload: payload.clone(),
            mode,
            requests: Arc::new(AtomicUsize::new(0)),
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
            payload,
            shutdown: tx,
            handle,
        })
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{}", self.addr, path)).unwrap()
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }
}

// Helpers

async fn fresh_core(dir: &tempfile::TempDir) -> Result<(Core, PathBuf)> {
    let db_path = dir.path().join("test.db");
    let core = Core::open(&db_path).await?;
    Ok((core, db_path))
}

async fn wait_for_status(core: &Core, id: i64, target: Status, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let rec = core.get_download(id).await?;
        if rec.status == target {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let rec = core.get_download(id).await?;
    anyhow::bail!(
        "timed out waiting for status {target:?}; current = {:?} ({} bytes / {:?})",
        rec.status,
        rec.downloaded_bytes,
        rec.total_bytes,
    );
}

// Tests

#[tokio::test]
async fn migrations_apply_on_fresh_db() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;

    // Seeded categories should be present.
    let cats = core.list_categories().await?;
    let names: Vec<_> = cats.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "Documents",
        "Music",
        "Video",
        "Compressed",
        "Programs",
        "Other",
    ] {
        assert!(
            names.contains(&expected),
            "missing seeded category {expected}"
        );
    }

    // Seeded settings. The migration bumps this from 3 to 4 on
    // fresh installs (only when the row still holds the original seed).
    let max = core
        .get_setting(settings_keys::MAX_CONCURRENT_DOWNLOADS)
        .await?
        .expect("max_concurrent_downloads should be seeded");
    assert_eq!(max.as_u64(), Some(4));

    Ok(())
}

#[tokio::test]
async fn auto_categorize_by_extension() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;
    let out_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&out_dir)?;

    let cases = &[
        ("song.MP3", "Music"),
        ("movie.mkv", "Video"),
        ("archive.zip", "Compressed"),
        ("notes.pdf", "Documents"),
        ("setup.exe", "Programs"),
        ("weird.xyz123", "Other"),
        ("noext", "Other"),
    ];

    for (filename, expected_cat) in cases {
        let id = core
            .add_download(AddDownload {
                url: Url::parse("https://example.com/x").unwrap(),
                filename: Some((*filename).to_string()),
                output_path: Some(out_dir.join(filename)),
                category: None,
                priority: 0,
                segments: Some(1),
                media_info: None,
                headers: None,
                source: DownloadSource::Manual,
            })
            .await?;
        let rec = core.get_download(id).await?;
        let cat = core
            .get_category(rec.category_id.expect("category should be auto-assigned"))
            .await?;
        assert_eq!(
            cat.name, *expected_cat,
            "filename {filename:?} expected category {expected_cat}, got {}",
            cat.name
        );
    }

    Ok(())
}

#[tokio::test]
async fn settings_round_trip() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;

    core.set_setting(
        settings_keys::MAX_CONCURRENT_DOWNLOADS,
        SettingValue::from_u64(5),
    )
    .await?;
    let v = core
        .get_setting(settings_keys::MAX_CONCURRENT_DOWNLOADS)
        .await?
        .unwrap();
    assert_eq!(v.as_u64(), Some(5));

    let custom_key = "test_custom_key";
    core.set_setting(custom_key, SettingValue::from_string("hello"))
        .await?;
    let v = core.get_setting(custom_key).await?.unwrap();
    assert_eq!(v.as_str(), Some("hello"));

    let all = core.all_settings().await?;
    assert!(all.contains_key(settings_keys::MAX_CONCURRENT_DOWNLOADS));
    assert!(all.contains_key(custom_key));

    Ok(())
}

#[tokio::test]
async fn queue_respects_concurrency_limit() -> Result<()> {
    let server = TestServer::start(
        ServerMode {
            delay_ms: 30,
            chunk_size: 16 * 1024,
        },
        256 * 1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;

    core.set_setting(
        settings_keys::MAX_CONCURRENT_DOWNLOADS,
        SettingValue::from_u64(2),
    )
    .await?;

    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    // Add 5 downloads; concurrency is 2, so at most 2 should be active at once.
    let mut ids = Vec::new();
    for i in 0..5 {
        let id = core
            .add_download(AddDownload {
                url: server.url(&format!("/file-{i}.bin")),
                filename: Some(format!("file-{i}.bin")),
                output_path: Some(out_dir.join(format!("file-{i}.bin"))),
                category: Some(CategorySelector::Name("Other".into())),
                priority: 0,
                segments: Some(1),
                media_info: None,
                headers: None,
                source: DownloadSource::Manual,
            })
            .await?;
        ids.push(id);
    }

    core.start().await?;

    // Sample the active count a few times while the downloads are in flight.
    let mut max_active = 0usize;
    let probe_deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < probe_deadline {
        let active = core
            .list_downloads(DownloadFilter {
                status: Some(Status::Active),
                category_id: None,
            })
            .await?
            .len();
        if active > max_active {
            max_active = active;
        }
        if max_active >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(max_active >= 1, "queue never started any download");
    assert!(
        max_active <= 2,
        "queue exceeded concurrency limit: saw {max_active} active at once"
    );

    // Wait for all to complete.
    for id in &ids {
        wait_for_status(&core, *id, Status::Completed, Duration::from_secs(30)).await?;
    }

    // Every output file should match the server payload.
    for i in 0..5usize {
        let bytes = std::fs::read(out_dir.join(format!("file-{i}.bin")))?;
        assert_eq!(sha256_hex(&bytes), sha256_hex(&server.payload));
    }

    core.shutdown().await?;
    server.stop().await;
    Ok(())
}

#[tokio::test]
async fn pause_resume_survives_core_restart() -> Result<()> {
    // Chunked-slow server: each 16 KiB chunk waits 60 ms before going out.
    // A 2 MiB payload then takes ~128 chunks × 60 ms ≈ 7.5 s, plenty of
    // wall-clock to fire a pause mid-flight.
    let server = TestServer::start(
        ServerMode {
            delay_ms: 60,
            chunk_size: 16 * 1024,
        },
        2 * 1024 * 1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("test.db");
    let out = dir.path().join("file.bin");

    let id = {
        let core = Core::open(&db_path).await?;
        let mut events = core.subscribe();
        core.start().await?;
        let id = core
            .add_download(AddDownload {
                url: server.url("/file.bin"),
                filename: Some("file.bin".into()),
                output_path: Some(out.clone()),
                category: Some(CategorySelector::Name("Other".into())),
                priority: 0,
                segments: Some(4),
                media_info: None,
                headers: None,
                source: DownloadSource::Manual,
            })
            .await?;

        // Wait for the worker to start producing progress.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut saw_progress = false;
        while std::time::Instant::now() < deadline {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(CoreEvent::ProgressUpdate { downloaded, .. }) if downloaded > 0 => {
                        saw_progress = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
        assert!(saw_progress, "never saw progress before pausing");

        core.pause(id).await?;
        wait_for_status(&core, id, Status::Paused, Duration::from_secs(5)).await?;

        // Snapshot how much we got — should be > 0 but < full size.
        let rec = core.get_download(id).await?;
        assert!(rec.downloaded_bytes > 0, "paused with zero bytes");
        assert!(
            rec.downloaded_bytes < server.payload.len() as u64,
            "paused after already completed"
        );

        core.shutdown().await?;
        id
    };

    // Sidecar should be on disk for resume.
    let meta_path = engine::Meta::sidecar_path(&out);
    assert!(meta_path.exists(), "expected sidecar after pause+shutdown");

    // Brand-new Core against the same DB.
    {
        let core = Core::open(&db_path).await?;
        core.start().await?;
        core.resume(id).await?;
        wait_for_status(&core, id, Status::Completed, Duration::from_secs(30)).await?;

        let bytes = std::fs::read(&out)?;
        assert_eq!(sha256_hex(&bytes), sha256_hex(&server.payload));
        core.shutdown().await?;
    }

    server.stop().await;
    Ok(())
}

/// `CoreEvent::QueueEmptied` fires exactly once after the active worker
/// set drains from non-empty to empty, and the 1-second debounce holds.
#[tokio::test]
async fn queue_emptied_fires_once_per_drain() -> Result<()> {
    let server = TestServer::start(
        ServerMode {
            delay_ms: 0,
            chunk_size: 64 * 1024,
        },
        128 * 1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;
    let mut events = core.subscribe();

    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    // Queue two downloads and let them complete back-to-back.
    let mut ids = Vec::new();
    for i in 0..2 {
        let id = core
            .add_download(AddDownload {
                url: server.url(&format!("/qe-{i}.bin")),
                filename: Some(format!("qe-{i}.bin")),
                output_path: Some(out_dir.join(format!("qe-{i}.bin"))),
                category: Some(CategorySelector::Name("Other".into())),
                priority: 0,
                segments: Some(1),
                media_info: None,
                headers: None,
                source: DownloadSource::Manual,
            })
            .await?;
        ids.push(id);
    }
    core.start().await?;

    for id in &ids {
        wait_for_status(&core, *id, Status::Completed, Duration::from_secs(30)).await?;
    }

    // The debounce is 1 s; the manager ticks at 500 ms. After both rows
    // complete we wait long enough for at least two ticks past the
    // threshold to observe both the emission and the disarm.
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    let mut count = 0usize;
    while std::time::Instant::now() < deadline {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(CoreEvent::QueueEmptied) => count += 1,
                Ok(_) => {}
                Err(_) => break,
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    assert_eq!(
        count, 1,
        "expected QueueEmptied to fire exactly once per drain, saw {count}"
    );

    core.shutdown().await?;
    server.stop().await;
    Ok(())
}

/// A brief gap between two downloads must not produce a second
/// `QueueEmptied` — the 1-second debounce holds the emission until a
/// stable empty observation crosses the threshold.
#[tokio::test]
async fn queue_emptied_does_not_fire_during_brief_gap() -> Result<()> {
    let server = TestServer::start(
        ServerMode {
            delay_ms: 0,
            chunk_size: 64 * 1024,
        },
        128 * 1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;
    let mut events = core.subscribe();

    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    // First download — let it finish.
    let id1 = core
        .add_download(AddDownload {
            url: server.url("/gap-1.bin"),
            filename: Some("gap-1.bin".into()),
            output_path: Some(out_dir.join("gap-1.bin")),
            category: Some(CategorySelector::Name("Other".into())),
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
        })
        .await?;
    core.start().await?;
    wait_for_status(&core, id1, Status::Completed, Duration::from_secs(30)).await?;

    // Within the 1-second debounce window, enqueue a second download so
    // the active set goes empty → non-empty before the emit can fire.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let id2 = core
        .add_download(AddDownload {
            url: server.url("/gap-2.bin"),
            filename: Some("gap-2.bin".into()),
            output_path: Some(out_dir.join("gap-2.bin")),
            category: Some(CategorySelector::Name("Other".into())),
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
        })
        .await?;
    wait_for_status(&core, id2, Status::Completed, Duration::from_secs(30)).await?;

    // After the second drain there should be exactly one QueueEmptied —
    // not two, because the brief gap mid-sequence was absorbed by the
    // debounce.
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    let mut count = 0usize;
    while std::time::Instant::now() < deadline {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(CoreEvent::QueueEmptied) => count += 1,
                Ok(_) => {}
                Err(_) => break,
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    assert_eq!(
        count, 1,
        "expected exactly one QueueEmptied across the drain → refill → drain sequence, saw {count}"
    );

    core.shutdown().await?;
    server.stop().await;
    Ok(())
}

/// KNOWN_BUGS #2: a one-click host whose captured URL resolves to a
/// 0-byte body server-side must NOT land as `Completed` at 0 B (silent
/// data loss). The queue's completion gate has to turn it into `Failed`.
#[tokio::test]
async fn empty_body_download_fails_instead_of_completing() -> Result<()> {
    let server = TestServer::start(
        ServerMode {
            delay_ms: 0,
            chunk_size: 64 * 1024,
        },
        1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    let id = core
        .add_download(AddDownload {
            url: server.url("/empty"),
            filename: Some("movie.mkv".into()),
            output_path: Some(out_dir.join("movie.mkv")),
            category: Some(CategorySelector::Name("Other".into())),
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
        })
        .await?;
    core.start().await?;

    wait_for_status(&core, id, Status::Failed, Duration::from_secs(15)).await?;
    let rec = core.get_download(id).await?;
    assert_eq!(rec.status, Status::Failed);
    assert!(
        rec.error.as_deref().unwrap_or("").contains("empty"),
        "expected an empty-response error, got: {:?}",
        rec.error
    );
    // The bogus 0-byte artefact must not be left on disk.
    assert!(
        !out_dir.join("movie.mkv").exists(),
        "0-byte file should have been removed"
    );

    core.shutdown().await?;
    server.stop().await;
    Ok(())
}

/// KNOWN_BUGS #3: the same class of failure when the host returns an HTML
/// landing page (non-zero body, `Content-Type: text/html`) in place of the
/// requested file. Must be `Failed`, not `Completed`.
#[tokio::test]
async fn html_landing_page_download_fails_instead_of_completing() -> Result<()> {
    let server = TestServer::start(
        ServerMode {
            delay_ms: 0,
            chunk_size: 64 * 1024,
        },
        1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    let id = core
        .add_download(AddDownload {
            url: server.url("/landing"),
            filename: Some("movie.mkv".into()),
            output_path: Some(out_dir.join("movie.mkv")),
            category: Some(CategorySelector::Name("Other".into())),
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
        })
        .await?;
    core.start().await?;

    wait_for_status(&core, id, Status::Failed, Duration::from_secs(15)).await?;
    let rec = core.get_download(id).await?;
    assert_eq!(rec.status, Status::Failed);
    assert!(
        rec.error.as_deref().unwrap_or("").contains("HTML"),
        "expected an HTML-response error, got: {:?}",
        rec.error
    );

    core.shutdown().await?;
    server.stop().await;
    Ok(())
}

/// A download with a future `start_at` schedule stays queued
/// until the scheduled time, then claims on the next tick. We schedule
/// for `now + 1.5s`, assert the row is still `Queued` at +500ms, then
/// wait for it to flip to `Completed`.
#[tokio::test]
async fn start_at_schedule_defers_until_due() -> Result<()> {
    let server = TestServer::start(
        ServerMode {
            delay_ms: 0,
            chunk_size: 64 * 1024,
        },
        64 * 1024,
    )
    .await?;
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir)?;

    let id = core
        .add_download(AddDownload {
            url: server.url("/sched-1.bin"),
            filename: Some("sched-1.bin".into()),
            output_path: Some(out_dir.join("sched-1.bin")),
            category: Some(CategorySelector::Name("Other".into())),
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
        })
        .await?;

    let when = (chrono::Utc::now() + chrono::Duration::milliseconds(1500)).to_rfc3339();
    core.add_schedule(NewSchedule {
        kind: ScheduleKind::StartAt,
        download_id: Some(id),
        start_iso: Some(when),
        end_iso: None,
        days_mask: None,
        active: Some(true),
    })
    .await?;

    core.start().await?;

    // At t+500ms the schedule still gates the row.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        core.get_download(id).await?.status,
        Status::Queued,
        "row should still be queued before its scheduled time"
    );

    // Wait through the gate; the row should run to completion.
    wait_for_status(&core, id, Status::Completed, Duration::from_secs(10)).await?;

    core.shutdown().await?;
    server.stop().await;
    Ok(())
}

/// A global `quiet_hours` row covering "now" is reflected in
/// `Core::quiet_hours_state()`. We construct a window starting one
/// minute ago and ending one minute from now, then assert
/// `active == true` and a populated `until`.
#[tokio::test]
async fn quiet_hours_state_reflects_active_window() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (core, _) = fresh_core(&dir).await?;

    // Build an HH:MM window that comfortably brackets "now" in the
    // process's local timezone. We use ±1 minute around the current
    // local hour:minute and a days_mask of "every day" so the test
    // doesn't flake at midnight.
    let now = chrono::Local::now();
    let pad = |n: u32| format!("{n:02}");
    let start_min = now.naive_local().time();
    let start = format!("{}:{}", pad(start_min.hour()), pad(start_min.minute()));
    let end_min = (now + chrono::Duration::minutes(1)).naive_local().time();
    let end = format!("{}:{}", pad(end_min.hour()), pad(end_min.minute()));

    // Subtle: when the current minute is :59 the end wraps to :00 and
    // the same-day branch in `quiet_hours_active` requires start <= end
    // for a non-wrap window. The test sidesteps this rare edge by
    // skipping the assertion when we'd land on the wrap boundary —
    // the schedule.rs unit tests cover the wrap case directly.
    if start_min.hour() == 23 && start_min.minute() >= 58 {
        return Ok(());
    }

    core.add_schedule(NewSchedule {
        kind: ScheduleKind::QuietHours,
        download_id: None,
        start_iso: Some(start),
        end_iso: Some(end),
        days_mask: Some(127),
        active: Some(true),
    })
    .await?;

    let state = core.quiet_hours_state().await;
    assert!(state.active, "quiet hours should be reported as active");
    assert!(
        state.until.is_some(),
        "active quiet hours should report a non-null `until`"
    );

    core.shutdown().await?;
    Ok(())
}
