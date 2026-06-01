//! Top-level downloader: orchestrates probe, segment splitting,
//! preallocation, and resume. The actual transfer (worker queue +
//! ticker + per-segment telemetry) lives in [`crate::transfer`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::RANGE;
use reqwest::{Client, Response, StatusCode};
use tokio::fs::OpenOptions;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::error::{EngineError, Result};
use crate::http::{build_client, probe, RemoteInfo};
use crate::meta::Meta;
use crate::progress::{ProgressEvent, DEFAULT_CHANNEL_CAPACITY};
use crate::retry::Backoff;
use crate::segment::{split, Segment};
use crate::transfer::{run_transfer, Control};

/// Tunable inputs for a fresh download.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub url: Url,
    /// Final on-disk path. If a directory is passed, the engine errors —
    /// the CLI is responsible for joining a filename hint.
    pub output: PathBuf,
    pub segments: usize,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub backoff: Backoff,
    /// Override the User-Agent header on every request. `None` keeps the
    /// engine's compiled-in default; empty strings should be normalized to
    /// `None` by the caller.
    pub user_agent: Option<String>,
    /// Additional request headers replayed on every HEAD / ranged GET via
    /// reqwest's `default_headers`. Populated by captures
    /// (Cookie / Referer / observed `webRequest` headers); empty for
    /// CLI / Add-URL flows. Names on
    /// [`crate::http::HEADER_DROP_LIST`](crate::http) (case-insensitive)
    /// are silently dropped so a captured `Range` or `Host` cannot
    /// corrupt the segment loop.
    pub headers: Vec<(String, String)>,
    /// Broadcast channel capacity; only used when a sender is built internally.
    pub channel_capacity: usize,
}

impl DownloadOptions {
    pub fn new(url: Url, output: PathBuf) -> Self {
        Self {
            url,
            output,
            segments: 8,
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(60),
            backoff: Backoff::default(),
            user_agent: None,
            headers: Vec::new(),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

/// Result of a completed download.
#[derive(Debug, Clone)]
pub struct DownloadSummary {
    pub url: Url,
    pub output: PathBuf,
    pub bytes: u64,
    pub segments: usize,
    pub resumed: bool,
    /// `Content-Type` advertised by the server on the probe response, if
    /// any. Carried through so the queue's completion gate can refuse to
    /// mark an HTML landing page (served in place of the real file by
    /// one-click hosts) as a successful download.
    pub content_type: Option<String>,
}

/// Start a brand-new download. If a sidecar already exists for the output
/// path, it is overwritten — call [`resume_at`] to continue an existing
/// transfer instead.
pub async fn download(
    opts: DownloadOptions,
    cancel: CancellationToken,
    tx: Option<broadcast::Sender<ProgressEvent>>,
) -> Result<DownloadSummary> {
    download_with_control(opts, cancel, tx, None).await
}

/// Same as [`download`] but accepts a control receiver for live
/// re-segmentation. Pass `None` for the basic non-controllable path.
pub async fn download_with_control(
    opts: DownloadOptions,
    cancel: CancellationToken,
    tx: Option<broadcast::Sender<ProgressEvent>>,
    control_rx: Option<mpsc::Receiver<Control>>,
) -> Result<DownloadSummary> {
    if opts.output.is_dir() {
        return Err(EngineError::other(format!(
            "output path is a directory: {}",
            opts.output.display()
        )));
    }
    if let Some(parent) = opts.output.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| EngineError::io(Some(parent.to_path_buf()), e))?;
        }
    }

    let client = build_client(
        opts.connect_timeout,
        opts.read_timeout,
        opts.user_agent.as_deref(),
        &opts.headers,
    )?;

    // One plain GET — no HEAD — that doubles as the metadata probe and, for
    // single-stream downloads, the body source. A browser makes exactly one
    // request; so do we. This stops a throwaway HEAD from spending a
    // one-time / session-bound token (common with one-click file hosts)
    // before the real fetch, which previously left the transfer empty.
    let (info, resp) = initial_get(&client, &opts.url).await?;
    let meta_path = Meta::sidecar_path(&opts.output);

    // Only range-capable hosts with a known length earn the parallel
    // segmented treatment. Confirm advertised range support is real (some
    // servers send `Accept-Ranges: bytes` then ignore `Range`). That verify
    // request only ever hits range-capable hosts — which by definition
    // tolerate multiple requests — never one-time-token hosts.
    let wants_segments =
        info.accept_ranges && info.content_length.is_some() && opts.segments > 1;
    let truly_segmented = wants_segments && verify_range_support(&client, &opts.url).await?;

    if truly_segmented {
        // Slow-start: begin with ONE connection over the whole file and let
        // the transfer ramp up toward `opts.segments`, backing off if the
        // host refuses a new connection. Starting at one (then adding,
        // confirm-before-commit) avoids the all-at-once burst that trips
        // per-IP connection caps (pixeldrain/Cloudflare). Drop the initial
        // non-range body; worker 0 issues its own ranged GET.
        drop(resp);
        let total = info.content_length.unwrap_or(0);
        let meta = Meta::new(
            opts.url.to_string(),
            opts.output.clone(),
            info.content_length,
            info.etag.clone(),
            info.last_modified.clone(),
            true,
            vec![Segment {
                index: 0,
                start: 0,
                end: total,
            }],
        );
        meta.save(&meta_path).await?;
        preallocate(&opts.output, meta.total_bytes).await?;
        let content_type = info.content_type.clone();
        // A child token: a worker failure inside `run_transfer` calls
        // `cancel()`, and token clones share state — using the parent
        // directly would cancel the fallback before it starts. The child is
        // still cancelled if the *user* cancels the parent.
        match run_transfer(
            client.clone(),
            opts.clone(),
            meta,
            meta_path.clone(),
            cancel.child_token(),
            tx.clone(),
            false,
            control_rx,
            content_type,
            None,
            Some(opts.segments),
        )
        .await
        {
            Err(ref e) if looks_like_concurrency_limit(e) => {
                // Even the first ranged connection was refused (some hosts
                // 403 every Range yet serve one plain GET, like a browser).
                // Fall back to single-stream; `preallocate` truncates any
                // partial bytes.
                tracing::warn!(
                    error = %e,
                    "segmented transfer rejected; retrying single-stream"
                );
                return single_stream_download(&client, opts, &meta_path, cancel, tx).await;
            }
            other => return other,
        }
    }

    // Single-stream: chosen up front (one-click host / unknown length / no
    // ranges) or because the server lied about range support. Either way we
    // can stream the body we already opened — exactly one GET total.
    let meta = build_initial_meta(&opts, &info, false);
    meta.save(&meta_path).await?;
    preallocate(&opts.output, meta.total_bytes).await?;
    let content_type = info.content_type.clone();
    run_transfer(
        client,
        opts,
        meta,
        meta_path,
        cancel,
        tx,
        false,
        control_rx,
        content_type,
        Some(resp),
        None,
    )
    .await
}

/// Download `opts` over a single connection, fetching the body with one
/// fresh `GET`. Used both as the chosen path for non-segmentable hosts and
/// as the fallback when a segmented attempt is rejected for opening too
/// many connections. Live re-segmentation control is intentionally dropped
/// here — a single stream has nothing to re-segment.
async fn single_stream_download(
    client: &Client,
    opts: DownloadOptions,
    meta_path: &Path,
    cancel: CancellationToken,
    tx: Option<broadcast::Sender<ProgressEvent>>,
) -> Result<DownloadSummary> {
    let (info, resp) = initial_get(client, &opts.url).await?;
    let meta = build_initial_meta(&opts, &info, false);
    meta.save(meta_path).await?;
    preallocate(&opts.output, meta.total_bytes).await?;
    let content_type = info.content_type.clone();
    run_transfer(
        client.clone(),
        opts,
        meta,
        meta_path.to_path_buf(),
        cancel,
        tx,
        false,
        None,
        content_type,
        Some(resp),
        None,
    )
    .await
}

/// Whether a *segmented* transfer failure looks like the host rejecting
/// concurrent connections (rather than a genuine fetch error). Such hosts
/// (e.g. pixeldrain behind Cloudflare) serve a single connection fine, so
/// the caller retries single-stream. Missing-resource statuses (404/410)
/// are deliberately excluded — one connection won't conjure the file.
fn looks_like_concurrency_limit(err: &EngineError) -> bool {
    match err {
        EngineError::TerminalStatus { status } | EngineError::TransientStatus { status } => {
            matches!(*status, 403 | 429 | 503 | 509)
        }
        // A transient status that burned the whole retry budget under the
        // parallel burst — the gentler single connection is worth a try.
        EngineError::RetryExhausted { .. } => true,
        _ => false,
    }
}

/// Issue a single plain `GET` (no HEAD, no `Range`) and parse the metadata
/// we care about from the response, returning the live response so the
/// caller can stream its body (single-stream) or drop it and start ranged
/// GETs (segmented). Non-success statuses map through the same
/// [`crate::http::map_status_error`] as [`probe`], before any file is
/// created on disk.
async fn initial_get(client: &Client, url: &Url) -> Result<(RemoteInfo, Response)> {
    let resp = client.get(url.clone()).send().await?;
    let status = resp.status();
    if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
        return Err(crate::http::map_status_error(status.as_u16()));
    }
    let info = crate::http::parse_remote_info(url, &resp);
    Ok((info, resp))
}

/// Resume a download from a sidecar metadata file. Validates ETag /
/// Last-Modified before continuing; if either has changed, the engine
/// returns [`EngineError::RemoteChanged`] and the caller can restart from
/// scratch.
#[allow(clippy::too_many_arguments)]
pub async fn resume_at(
    meta_path: PathBuf,
    backoff: Backoff,
    connect_timeout: Duration,
    read_timeout: Duration,
    user_agent: Option<String>,
    headers: Vec<(String, String)>,
    cancel: CancellationToken,
    tx: Option<broadcast::Sender<ProgressEvent>>,
) -> Result<DownloadSummary> {
    resume_at_with_control(
        meta_path,
        backoff,
        connect_timeout,
        read_timeout,
        user_agent,
        headers,
        cancel,
        tx,
        None,
    )
    .await
}

/// Same as [`resume_at`] but accepts a control receiver for live
/// re-segmentation.
#[allow(clippy::too_many_arguments)]
pub async fn resume_at_with_control(
    meta_path: PathBuf,
    backoff: Backoff,
    connect_timeout: Duration,
    read_timeout: Duration,
    user_agent: Option<String>,
    headers: Vec<(String, String)>,
    cancel: CancellationToken,
    tx: Option<broadcast::Sender<ProgressEvent>>,
    control_rx: Option<mpsc::Receiver<Control>>,
) -> Result<DownloadSummary> {
    let meta = Meta::load(&meta_path).await?;
    let url: Url = meta
        .url
        .parse()
        .map_err(|_| EngineError::InvalidUrl(meta.url.clone()))?;

    let client = build_client(
        connect_timeout,
        read_timeout,
        user_agent.as_deref(),
        &headers,
    )?;
    let info = probe(&client, &url).await?;

    if !meta.matches_remote(
        info.etag.as_deref(),
        info.last_modified.as_deref(),
        info.content_length,
    ) {
        return Err(EngineError::RemoteChanged);
    }

    let opts = DownloadOptions {
        url,
        output: meta.output_path.clone(),
        segments: meta.segments.len(),
        connect_timeout,
        read_timeout,
        backoff,
        user_agent,
        headers,
        channel_capacity: DEFAULT_CHANNEL_CAPACITY,
    };

    if !opts.output.exists() {
        preallocate(&opts.output, meta.total_bytes).await?;
    }

    let content_type = info.content_type.clone();
    run_transfer(
        client, opts, meta, meta_path, cancel, tx, true, control_rx, content_type, None, None,
    )
    .await
}

/// Send a 1-byte ranged GET to confirm the server actually honors ranges
/// (some servers advertise `Accept-Ranges: bytes` but return 200 with the
/// entire body anyway).
async fn verify_range_support(client: &Client, url: &Url) -> Result<bool> {
    let resp = client
        .get(url.clone())
        .header(RANGE, "bytes=0-0")
        .send()
        .await?;
    Ok(resp.status() == StatusCode::PARTIAL_CONTENT)
}

fn build_initial_meta(opts: &DownloadOptions, info: &RemoteInfo, ranges: bool) -> Meta {
    let segments = if ranges {
        split(info.content_length.unwrap_or(0), opts.segments)
    } else {
        let end = info.content_length.unwrap_or(0);
        vec![Segment {
            index: 0,
            start: 0,
            end,
        }]
    };
    Meta::new(
        opts.url.to_string(),
        opts.output.clone(),
        info.content_length,
        info.etag.clone(),
        info.last_modified.clone(),
        ranges,
        segments,
    )
}

async fn preallocate(path: &Path, total: Option<u64>) -> Result<()> {
    let f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(|e| EngineError::io(Some(path.to_path_buf()), e))?;
    if let Some(total) = total {
        if total > 0 {
            f.set_len(total)
                .await
                .map_err(|e| EngineError::io(Some(path.to_path_buf()), e))?;
        }
    }
    drop(f);
    Ok(())
}
