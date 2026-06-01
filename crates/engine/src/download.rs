//! Top-level downloader: orchestrates probe, segment splitting,
//! preallocation, and resume. The actual transfer (worker queue +
//! ticker + per-segment telemetry) lives in [`crate::transfer`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::RANGE;
use reqwest::{Client, StatusCode};
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
    let info = probe(&client, &opts.url).await?;

    let truly_supports_ranges =
        if info.accept_ranges && info.content_length.is_some() && opts.segments > 1 {
            verify_range_support(&client, &opts.url).await?
        } else {
            false
        };

    let meta = build_initial_meta(&opts, &info, truly_supports_ranges);
    let meta_path = Meta::sidecar_path(&opts.output);
    meta.save(&meta_path).await?;

    preallocate(&opts.output, meta.total_bytes).await?;

    run_transfer(client, opts, meta, meta_path, cancel, tx, false, control_rx).await
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

    run_transfer(client, opts, meta, meta_path, cancel, tx, true, control_rx).await
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
