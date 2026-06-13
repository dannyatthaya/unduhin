//! Thin async wrapper around the `yt-dlp` external binary.
//!
//! Two entry points:
//!
//! - [`probe`] — invokes `yt-dlp --dump-single-json` against a URL,
//!   waits for the JSON, and lifts it into a [`ProbeResult`]. Short
//!   default timeout (3 s) so a pasted direct-file URL isn't slowed
//!   down on its way to the engine path.
//! - [`download`] — invokes `yt-dlp` with `--progress-template` and
//!   streams parsed [`progress::Tick`] events onto the supplied engine
//!   broadcast channel, so a yt-dlp download is indistinguishable from
//!   an engine download to consumers (UI / DB persistence).
//!
//! The binary path is supplied by the caller — `core::tooling` resolves
//! it from settings / managed dir / system PATH, this module just spawns
//! the process.
//!
//! Tests in this module focus on the JSON / progress-line parsers; the
//! subprocess wiring is exercised manually and through integration
//! tests that ship a stub yt-dlp binary.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use engine::{CancellationToken, ProgressEvent};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::time::timeout;

pub mod progress;
mod wire;

/// Outcome of probing a URL with yt-dlp's metadata extractors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// The URL yt-dlp resolved (may differ from the user-supplied input
    /// after the extractor canonicalizes it).
    pub url: String,
    /// Lower-cased extractor key (`"youtube"`, `"vimeo"`, `"twitter"`, …).
    pub extractor: String,
    pub title: String,
    pub uploader: Option<String>,
    pub duration_secs: Option<u32>,
    pub thumbnail_url: Option<String>,
    pub is_live: bool,
    pub age_limit: Option<u32>,
    pub formats: Vec<Format>,
    /// yt-dlp format selector the UI should preselect for "Best video+audio".
    /// `None` when no usable video format is exposed.
    pub recommended_video_audio: Option<String>,
    /// yt-dlp format selector for "Audio only (best)". `None` when no
    /// audio-only format is exposed.
    pub recommended_audio_only: Option<String>,
}

/// One downloadable format yt-dlp exposed for the URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Format {
    pub format_id: String,
    pub ext: String,
    /// `"1920x1080"`, `"audio only"`, or `None` when yt-dlp didn't report.
    pub resolution: Option<String>,
    pub fps: Option<u32>,
    /// `"avc1.640028"`, `"none"`, or `None`.
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub filesize_bytes: Option<u64>,
    pub tbr_kbps: Option<f64>,
    pub note: Option<String>,
}

/// Persisted on the download row so the queue worker can re-spawn yt-dlp
/// after a restart without re-probing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub extractor: String,
    /// The yt-dlp format selector. Either a single `format_id` (e.g.
    /// `"140"`), a `+`-combined pair (`"137+140"`), or yt-dlp's selector
    /// DSL (`"bv*+ba/b"`). When the string contains `+`, ffmpeg is needed
    /// to merge the streams.
    pub format_selector: String,
    pub title: String,
    pub original_url: String,
    /// Set when `format_selector` contains `+` (i.e. separate video and
    /// audio streams that must be muxed).
    pub needs_ffmpeg: bool,
}

impl MediaInfo {
    pub fn needs_ffmpeg_for(selector: &str) -> bool {
        selector.contains('+')
    }
}

/// Result of a successful [`download`] call.
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    /// Total bytes written to disk (best-effort, from the last progress
    /// tick — yt-dlp doesn't report this authoritatively).
    pub bytes: u64,
    /// The actual on-disk path yt-dlp wrote the file to, including the
    /// extension it picked after `%(ext)s` expansion / post-mux. `None`
    /// only if the `--print after_move:` line never arrived (older
    /// yt-dlp, or extractor that bypasses the move step).
    pub final_path: Option<PathBuf>,
}

/// Inputs to [`download`].
#[derive(Debug, Clone)]
pub struct YtdlpJob {
    pub url: String,
    pub format_selector: String,
    pub output_dir: PathBuf,
    /// yt-dlp `--output` template, e.g. `"%(title)s.%(ext)s"`. Resolved
    /// relative to `output_dir`.
    pub output_template: String,
    pub binary_path: PathBuf,
    /// Passed to yt-dlp via `--ffmpeg-location`; required when
    /// `format_selector` involves separate video+audio streams.
    pub ffmpeg_path: Option<PathBuf>,
    pub user_agent: Option<String>,
    /// Additional request headers forwarded to yt-dlp via `--add-header
    /// "Name:Value"`. Populated by browser captures
    /// (Cookie / Referer / observed `webRequest` headers). Names on
    /// [`engine::http::HEADER_DROP_LIST`] are silently dropped to mirror
    /// the engine's sanitization — captured `Range` or `Host` would
    /// break yt-dlp's own segment loop.
    pub extra_headers: Vec<(String, String)>,
    /// Global download speed cap in bytes/sec, passed to yt-dlp as
    /// `--limit-rate`. `None` or `0` means unlimited. Read from
    /// `global_speed_limit_bps` at spawn time (yt-dlp is a subprocess, so this
    /// is fixed for the run — unlike the HTTP engine's live token bucket).
    pub limit_rate_bps: Option<u64>,
    /// When `true`, pass `--extractor-args "generic:impersonate"` so the
    /// generic extractor mimics a real browser's TLS/HTTP fingerprint
    /// (curl_cffi). Defeats Cloudflare's anti-bot 403 on browser-captured
    /// HLS/DASH and pasted stream URLs — a TLS-handshake block that header
    /// forwarding cannot fix. Scoped to the generic extractor, so
    /// site-specific extractors are unaffected. Read from the
    /// `ytdlp_impersonate` setting at spawn time.
    pub impersonate: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum YtdlpError {
    #[error("yt-dlp not installed — install from Settings → Media")]
    NotInstalled,
    #[error("probe timed out after {0:?}")]
    Timeout(Duration),
    #[error("URL not recognized by any yt-dlp extractor")]
    Unsupported,
    #[error("DRM-protected content cannot be downloaded")]
    Drm,
    #[error(
        "YouTube blocked this request as suspicious (\"confirm you're not a bot\"). \
         Try updating yt-dlp in Settings → Media; if that doesn't help, the site is \
         rate-limiting from your network or needs a signed-in cookie jar (not yet supported)."
    )]
    BotChallenge,
    #[error("sign-in required for this URL: {0}")]
    AuthRequired(String),
    #[error("ffmpeg is required for the selected format but is not installed")]
    FfmpegMissing,
    #[error("yt-dlp exited {code} — {message}")]
    Process { code: i32, message: String },
    #[error("failed to parse yt-dlp output: {0}")]
    Parse(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Run `yt-dlp --dump-single-json` against `url`. Returns a [`ProbeResult`]
/// when the URL is recognized, or a typed error variant otherwise.
///
/// `timeout_duration` caps the whole probe (subprocess spawn + stdout
/// drain). Default callers should pass a few seconds — the user is
/// waiting on the formats dialog when this is called.
pub async fn probe(
    url: &str,
    binary_path: &Path,
    timeout_duration: Duration,
) -> Result<ProbeResult, YtdlpError> {
    if !binary_exists(binary_path).await {
        return Err(YtdlpError::NotInstalled);
    }

    let url_string = url.to_string();
    let binary = binary_path.to_path_buf();
    let task = async move {
        let mut cmd = Command::new(&binary);
        cmd.arg("--dump-single-json")
            .arg("--no-warnings")
            .arg("--no-playlist")
            .arg("--no-call-home")
            .arg("--skip-download")
            .arg(&url_string)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            // Hide the console window the child would otherwise pop up
            // when launched from a windowed Tauri parent. 0x08000000 =
            // CREATE_NO_WINDOW.
            cmd.creation_flags(0x0800_0000);
        }
        let output = cmd.output().await?;
        if !output.status.success() {
            return Err(classify_exit(&output.stderr, output.status.code()));
        }
        let raw: wire::RawInfo =
            serde_json::from_slice(&output.stdout).map_err(|e| YtdlpError::Parse(e.to_string()))?;
        let probe = raw.into_probe(&url_string);
        // yt-dlp's generic extractor matches almost any HTTP(S) URL and
        // returns metadata scraped from OG/HTML tags, so an ordinary web
        // page comes back as a "successful" probe with no downloadable
        // formats. Treat that as not-media (`Unsupported`) so callers fall
        // back to a plain HTTP download instead of opening the media/format
        // dialog for every pasted link. A real media site always reports at
        // least one format, so this never suppresses genuine matches.
        if probe.extractor == "generic" && probe.formats.is_empty() {
            return Err(YtdlpError::Unsupported);
        }
        Ok(probe)
    };

    match timeout(timeout_duration, task).await {
        Ok(result) => result,
        Err(_) => Err(YtdlpError::Timeout(timeout_duration)),
    }
}

/// Spawn yt-dlp to actually fetch a previously-probed URL. Progress is
/// emitted onto `progress_tx` in the same [`ProgressEvent`] shape the
/// engine uses, so the queue's pump task forwards it identically.
pub async fn download(
    job: YtdlpJob,
    cancel: CancellationToken,
    progress_tx: Option<broadcast::Sender<ProgressEvent>>,
) -> Result<DownloadOutcome, YtdlpError> {
    if !binary_exists(&job.binary_path).await {
        return Err(YtdlpError::NotInstalled);
    }
    if MediaInfo::needs_ffmpeg_for(&job.format_selector) {
        match job.ffmpeg_path.as_deref() {
            Some(p) if binary_exists(p).await => {}
            _ => return Err(YtdlpError::FfmpegMissing),
        }
    }

    tokio::fs::create_dir_all(&job.output_dir).await?;

    let output_arg = job.output_dir.join(&job.output_template);

    let mut cmd = Command::new(&job.binary_path);
    cmd.arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--newline")
        .arg("--no-overwrites")
        .arg("--continue")
        // yt-dlp throttles the progress hook internally; with no
        // `--progress-delta` it skips intermediate ticks entirely for
        // fast HTTP downloads, presenting as a 0 % → 100 % jump in the
        // UI. 0.5 s gives a smooth bar without spamming the event bus.
        .arg("--progress-delta")
        .arg("0.5")
        // `--print` (used below for `after_move:`) implicitly turns on
        // `--quiet`, which silences progress output entirely. Re-enable
        // it explicitly — without this, `--progress-template` lines
        // never reach stdout and the bar stays at 0 % until completion.
        .arg("--progress")
        .arg("--format")
        .arg(&job.format_selector)
        // Prefer `.mp4` for muxed output when the codecs allow it.
        // yt-dlp transparently falls back to `.mkv` for codec combos
        // that can't live in mp4 (VP9/opus, AV1, etc), so the final
        // extension still varies — `after_move:` keeps the DB in sync.
        .arg("--merge-output-format")
        .arg("mp4")
        // `total_bytes` is null for DASH/HLS sources (YouTube, most live-derived
        // streams) — those expose `total_bytes_estimate` instead. Without the
        // fallback the progress bar stays at 0% until the Completed event lands
        // because every tick reports an unknown total. yt-dlp's
        // `field,alt_field` syntax picks the first non-null value.
        .arg("--progress-template")
        .arg("%(progress.downloaded_bytes)s|%(progress.total_bytes,total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s")
        // Tag-prefix the printed path so we can recognize the line in
        // stdout without colliding with progress ticks or banners. yt-dlp
        // expands the literal prefix before the placeholder.
        .arg("--print")
        .arg("after_move:unduhin-final-path:%(filepath)s")
        .arg("--output")
        .arg(&output_arg);
    if let Some(ffmpeg) = job.ffmpeg_path.as_deref() {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    // Global speed cap. yt-dlp's `--limit-rate` accepts a raw bytes/sec
    // integer; `0`/`None` means no flag (unlimited).
    if let Some(bps) = job.limit_rate_bps.filter(|b| *b > 0) {
        cmd.arg("--limit-rate").arg(bps.to_string());
    }
    // Browser impersonation. With no explicit target yt-dlp auto-selects an
    // available impersonation client (curl_cffi) for the generic extractor,
    // matching a real browser's TLS/HTTP fingerprint so Cloudflare's anti-bot
    // challenge stops returning 403. Scoped to `generic:` — site-specific
    // extractors keep their own request logic. If the bundled yt-dlp has no
    // impersonation support it logs a warning and continues unimpersonated,
    // so leaving this on is safe.
    if job.impersonate {
        cmd.arg("--extractor-args").arg("generic:impersonate");
    }
    if let Some(ua) = job.user_agent.as_deref() {
        cmd.arg("--user-agent").arg(ua);
    }
    for (name, value) in sanitize_extra_headers(&job.extra_headers) {
        // yt-dlp accepts `--add-header NAME:VALUE`. The name is already
        // ASCII-validated by `sanitize_extra_headers`; the value cannot
        // contain CR/LF since that would terminate the arg.
        cmd.arg("--add-header").arg(format!("{name}:{value}"));
    }
    cmd.arg(&job.url)
        // yt-dlp.exe on Windows is a PyInstaller-frozen Python program.
        // When its stdout is piped (not a TTY), Python defaults to
        // block-buffering — progress lines accumulate in the buffer and
        // are flushed in one big chunk near the end, which presents as a
        // 0 % → 100 % jump in the UI. yt-dlp itself calls flush() after
        // each line, but PYTHONUNBUFFERED=1 disables the underlying
        // buffer so even partial writes reach us immediately. Cheap and
        // harmless on non-frozen yt-dlp builds (it's a Python-stdlib env
        // var, not a yt-dlp setting).
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000);
    }

    tracing::debug!(
        url = %job.url,
        format = %job.format_selector,
        "ytdlp: spawning"
    );

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| YtdlpError::Process {
        code: -1,
        message: "stdout missing".into(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| YtdlpError::Process {
        code: -1,
        message: "stderr missing".into(),
    })?;

    // Drain stderr concurrently — yt-dlp can block on a full pipe.
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut buf = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut reader = BufReader::new(stdout).lines();
    let mut last_total: Option<u64> = None;
    let mut last_downloaded: u64 = 0;
    let mut started_emitted = false;
    let mut final_path: Option<PathBuf> = None;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = stderr_task.await;
                return Err(YtdlpError::Process {
                    code: -1,
                    message: "cancelled".into(),
                });
            }
            line = reader.next_line() => {
                let Ok(line) = line else { break };
                let Some(line) = line else { break };
                tracing::trace!(line = %line, "ytdlp: stdout");
                if let Some(p) = parse_final_path(&line) {
                    final_path = Some(p);
                    continue;
                }
                let Some(tick) = progress::parse_line(&line) else { continue };
                tracing::debug!(
                    downloaded = tick.downloaded,
                    total = ?tick.total,
                    speed = ?tick.speed_bps,
                    eta = ?tick.eta,
                    "ytdlp: tick"
                );
                if !started_emitted {
                    emit(progress_tx.as_ref(), ProgressEvent::Started {
                        total: tick.total,
                        segments: 1,
                        resumed_bytes: 0,
                    });
                    started_emitted = true;
                }
                last_downloaded = tick.downloaded;
                if tick.total.is_some() { last_total = tick.total; }
                emit(progress_tx.as_ref(), ProgressEvent::Tick {
                    downloaded: tick.downloaded,
                    total: tick.total.or(last_total),
                    speed_bps: tick.speed_bps.unwrap_or(0.0),
                    eta: tick.eta,
                });
            }
        }
    }

    let status = child.wait().await?;
    let stderr_buf = stderr_task.await.unwrap_or_default();
    tracing::info!(
        exit_code = ?status.code(),
        last_downloaded,
        ?last_total,
        ?final_path,
        "ytdlp: child exited"
    );

    if status.success() {
        // yt-dlp's `--progress-template` only fires during HTTP transfer.
        // When the final file is the result of a post-mux (audio+video
        // merged into .mkv), the merged file's size doesn't appear in
        // any progress tick. Prefer the actual on-disk size of the file
        // yt-dlp printed via `after_move:` so the UI doesn't show 0 B
        // for completed merges. Fall back to whatever progress did
        // capture if the metadata read fails for any reason.
        let progress_bytes = last_total.unwrap_or(last_downloaded);
        let bytes = match final_path.as_deref() {
            Some(p) => tokio::fs::metadata(p)
                .await
                .map(|m| m.len())
                .unwrap_or(progress_bytes),
            None => progress_bytes,
        };
        tracing::info!(bytes, "ytdlp: emitting Completed");
        emit(progress_tx.as_ref(), ProgressEvent::Completed { bytes });
        Ok(DownloadOutcome { bytes, final_path })
    } else {
        Err(classify_exit(stderr_buf.as_bytes(), status.code()))
    }
}

/// Filter a captured header list against the engine's drop-list and
/// reject values containing control bytes (CR/LF would terminate the
/// `--add-header` argument or break yt-dlp's own header parser). The
/// drop-list matches the engine's so an HTTP download and a yt-dlp
/// download share the same surface area.
fn sanitize_extra_headers(pairs: &[(String, String)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            if engine::http::HEADER_DROP_LIST.iter().any(|d| *d == lower) {
                tracing::trace!(header = %name, "ytdlp: dropping per-request header");
                return None;
            }
            // Reject names with whitespace or control bytes and values
            // with CR/LF — these break the `--add-header NAME:VALUE`
            // argument shape.
            if name.is_empty() || name.bytes().any(|b| b <= 0x20 || b == b':' || b == 0x7f) {
                tracing::warn!(header = %name, "ytdlp: invalid header name; skipping");
                return None;
            }
            if value.bytes().any(|b| b == b'\r' || b == b'\n') {
                tracing::warn!(header = %name, "ytdlp: invalid header value; skipping");
                return None;
            }
            Some((name.clone(), value.clone()))
        })
        .collect()
}

/// yt-dlp emits one tagged line per successfully-moved file with the
/// shape `unduhin-final-path:<absolute path>`. We strip the tag and
/// return the path; everything else returns `None`.
fn parse_final_path(line: &str) -> Option<PathBuf> {
    line.trim()
        .strip_prefix("unduhin-final-path:")
        .map(|p| PathBuf::from(p.trim()))
}

/// Best-effort guess at the on-disk file when yt-dlp didn't emit an
/// `after_move:` line (older builds, exotic extractors). Tries `stored`
/// as-is first; otherwise scans the parent dir for a file whose stem
/// matches `stored`'s stem, picking the most recently modified non-temp
/// candidate.
pub(crate) async fn resolve_final_path_fallback(stored: &Path) -> Option<PathBuf> {
    if tokio::fs::metadata(stored).await.is_ok() {
        return Some(stored.to_path_buf());
    }
    let parent = stored.parent()?;
    let stem = stored.file_stem()?.to_string_lossy().into_owned();
    let mut dir = tokio::fs::read_dir(parent).await.ok()?;
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    while let Ok(Some(entry)) = dir.next_entry().await {
        let path = entry.path();
        let Some(entry_stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if entry_stem != stem {
            continue;
        }
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        // yt-dlp's intermediate scratch files. Skip them — picking one
        // here would record a half-downloaded segment as the final file.
        if matches!(ext.as_str(), "part" | "ytdl" | "temp" | "tmp") {
            continue;
        }
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        match &best {
            Some((_, t)) if *t >= mtime => {}
            _ => best = Some((path, mtime)),
        }
    }
    best.map(|(p, _)| p)
}

async fn binary_exists(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

fn emit(tx: Option<&broadcast::Sender<ProgressEvent>>, ev: ProgressEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(ev);
    }
}

/// Map a non-zero exit + stderr to a typed [`YtdlpError`]. yt-dlp prints
/// human-readable diagnostics that we partially key off; everything that
/// doesn't match falls through to the generic `Process` variant carrying
/// yt-dlp's own last line so the UI doesn't lie about *why* it failed.
///
/// Order matters: the bot-challenge check must precede the generic
/// "sign in" check because YouTube's anti-bot prompt also contains the
/// substring "sign in" and we don't want to mis-label it as an age gate.
fn classify_exit(stderr: &[u8], code: Option<i32>) -> YtdlpError {
    let text = String::from_utf8_lossy(stderr).to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("unsupported url") || lower.contains("no video formats found") {
        return YtdlpError::Unsupported;
    }
    if lower.contains("drm") || lower.contains("protected content") {
        return YtdlpError::Drm;
    }
    // YouTube anti-bot challenge — distinct from an actual age gate.
    if lower.contains("confirm you're not a bot")
        || lower.contains("confirm you’re not a bot")
        || lower.contains("not a bot")
    {
        return YtdlpError::BotChallenge;
    }
    if lower.contains("age restricted")
        || lower.contains("age-restricted")
        || lower.contains("confirm your age")
        || lower.contains("login required")
    {
        let detail = pick_error_line(&text);
        return YtdlpError::AuthRequired(detail);
    }
    if lower.contains("ffmpeg") && lower.contains("not found") {
        return YtdlpError::FfmpegMissing;
    }
    let message = pick_error_line(&text);
    YtdlpError::Process {
        code: code.unwrap_or(-1),
        message: if message.is_empty() {
            "yt-dlp exited non-zero".into()
        } else {
            message
        },
    }
}

/// Pull yt-dlp's most informative line out of stderr. yt-dlp prefixes
/// real errors with `ERROR:`; we prefer that, falling back to the last
/// non-empty line if no prefix is present.
fn pick_error_line(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| lines.last().copied().unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_extra_headers_drops_engine_droplist_case_insensitively() {
        let pairs = vec![
            ("RANGE".into(), "bytes=0-".into()),
            ("Host".into(), "x".into()),
            ("Cookie".into(), "a=b".into()),
            ("Referer".into(), "https://x/".into()),
        ];
        let filtered = sanitize_extra_headers(&pairs);
        let names: Vec<&str> = filtered.iter().map(|(n, _)| n.as_str()).collect();
        assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("range")));
        assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("host")));
        assert!(names.contains(&"Cookie"));
        assert!(names.contains(&"Referer"));
    }

    #[test]
    fn sanitize_extra_headers_rejects_crlf_in_value() {
        let pairs = vec![
            ("X-Bad".into(), "x\r\ninjected".into()),
            ("X-Ok".into(), "x".into()),
        ];
        let filtered = sanitize_extra_headers(&pairs);
        let names: Vec<&str> = filtered.iter().map(|(n, _)| n.as_str()).collect();
        assert!(!names.contains(&"X-Bad"));
        assert!(names.contains(&"X-Ok"));
    }

    #[test]
    fn sanitize_extra_headers_rejects_invalid_name_chars() {
        let pairs = vec![
            ("Bad Name".into(), "v".into()),
            ("name:colon".into(), "v".into()),
            ("Good".into(), "v".into()),
        ];
        let names: Vec<String> = sanitize_extra_headers(&pairs)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["Good".to_string()]);
    }

    #[test]
    fn needs_ffmpeg_detection() {
        assert!(!MediaInfo::needs_ffmpeg_for("140"));
        assert!(!MediaInfo::needs_ffmpeg_for("best"));
        assert!(MediaInfo::needs_ffmpeg_for("137+140"));
        assert!(MediaInfo::needs_ffmpeg_for("bv*+ba/b"));
    }

    #[test]
    fn classify_drm() {
        let e = classify_exit(b"ERROR: This is DRM-protected content.\n", Some(1));
        matches!(e, YtdlpError::Drm);
    }

    #[test]
    fn classify_unsupported() {
        let e = classify_exit(b"ERROR: Unsupported URL: foo://bar\n", Some(1));
        matches!(e, YtdlpError::Unsupported);
    }

    #[test]
    fn classify_age_gate() {
        let e = classify_exit(b"ERROR: Sign in to confirm your age.\n", Some(1));
        assert!(matches!(e, YtdlpError::AuthRequired(_)));
    }

    #[test]
    fn classify_bot_challenge_over_age_gate() {
        // YouTube's bot-detection prompt mentions "sign in" too — we
        // must not misroute it to the age-gate variant.
        let e = classify_exit(
            b"ERROR: [youtube] xyz: Sign in to confirm you're not a bot. Use --cookies-from-browser.\n",
            Some(1),
        );
        assert!(matches!(e, YtdlpError::BotChallenge));
    }

    #[test]
    fn classify_falls_through_to_process_with_raw_yt_dlp_line() {
        let e = classify_exit(b"ERROR: Some unknown failure\n", Some(2));
        match e {
            YtdlpError::Process { code, message } => {
                assert_eq!(code, 2);
                assert_eq!(message, "Some unknown failure");
            }
            _ => panic!("expected Process variant"),
        }
    }

    #[test]
    fn pick_error_line_prefers_error_prefix() {
        let text = "[youtube] some banner\nERROR: the real reason\n[debug] noise";
        assert_eq!(pick_error_line(text), "the real reason");
    }

    #[test]
    fn parse_final_path_strips_tag() {
        let ok = parse_final_path("unduhin-final-path:C:\\Users\\me\\Downloads\\video.mkv");
        assert_eq!(
            ok.as_deref(),
            Some(std::path::Path::new("C:\\Users\\me\\Downloads\\video.mkv"))
        );
        assert!(parse_final_path("[download] something else").is_none());
        assert!(parse_final_path("1024|2048|512|10").is_none());
        // Surrounding whitespace tolerated (yt-dlp shouldn't add any, but be safe).
        assert_eq!(
            parse_final_path("  unduhin-final-path:/tmp/x.mp4  ").as_deref(),
            Some(std::path::Path::new("/tmp/x.mp4"))
        );
    }

    #[tokio::test]
    async fn fallback_returns_stored_path_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("video.mp4");
        tokio::fs::write(&p, b"x").await.unwrap();
        let resolved = resolve_final_path_fallback(&p).await;
        assert_eq!(resolved.as_deref(), Some(p.as_path()));
    }

    #[tokio::test]
    async fn fallback_finds_renamed_extension_by_stem() {
        // Mirrors yt-dlp picking `.mkv` after mux when the row was
        // queued expecting `.mp4`: the original path doesn't exist, but
        // a sibling with the same stem does. Pick it up.
        let dir = tempfile::tempdir().unwrap();
        let queued = dir.path().join("Some Video.mp4");
        let actual = dir.path().join("Some Video.mkv");
        tokio::fs::write(&actual, b"x").await.unwrap();
        let resolved = resolve_final_path_fallback(&queued).await;
        assert_eq!(resolved.as_deref(), Some(actual.as_path()));
    }

    #[tokio::test]
    async fn fallback_skips_part_and_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let queued = dir.path().join("Some Video.mp4");
        let part = dir.path().join("Some Video.part");
        tokio::fs::write(&part, b"x").await.unwrap();
        let resolved = resolve_final_path_fallback(&queued).await;
        assert!(resolved.is_none(), "expected None, got {resolved:?}");
    }
}
