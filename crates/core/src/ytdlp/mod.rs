//! Thin async wrapper around the `yt-dlp` external binary.
//!
//! Three entry points:
//!
//! - [`probe`] — invokes `yt-dlp --dump-single-json` against a URL,
//!   waits for the JSON, and lifts it into a [`ProbeResult`]. Short
//!   default timeout (3 s) so a pasted direct-file URL isn't slowed
//!   down on its way to the engine path.
//! - [`probe_media_formats`] — same subprocess call as `probe` (shares
//!   its internals via `probe_raw`), but lifts the result into the
//!   browser extension-facing [`crate::wire::MediaFormat`] list instead.
//!   Internal-only / pipe-only: never exposed as a Tauri command.
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use engine::{CancellationToken, ProgressEvent};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::time::timeout;

mod process_tree;
pub mod progress;
mod wire;

use process_tree::ProcessTreeGuard;

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
    /// Where the finished file lands. Passed as `--paths home:`, never
    /// baked into `--output` — see [`download`] for why that distinction
    /// is load-bearing.
    pub output_dir: PathBuf,
    /// Scratch directory for yt-dlp's `.part`, `.ytdl`, and per-fragment
    /// files, passed as `--paths temp:`. Must be on the same volume as
    /// `output_dir` (see [`crate::ytdlp::scratch_dir_for`]) — yt-dlp's
    /// final temp→home step is a `shutil.move`, an instant rename within a
    /// volume but a full byte copy across one.
    ///
    /// Keyed per download and *not* cleaned up on cancel: a paused
    /// download resumes out of exactly this directory via `--continue`.
    pub temp_dir: PathBuf,
    /// yt-dlp `--output` template, e.g. `"%(title)s.%(ext)s"`. Must stay
    /// **relative** — yt-dlp ignores `--paths` entirely when the output
    /// template carries an absolute path.
    pub output_template: String,
    pub binary_path: PathBuf,
    /// Passed to yt-dlp via `--ffmpeg-location`; required when
    /// `format_selector` involves separate video+audio streams.
    pub ffmpeg_path: Option<PathBuf>,
    pub user_agent: Option<String>,
    /// Additional request headers forwarded to yt-dlp as `--add-header
    /// "Name:Value"` in a private options file, never on the command line
    /// (see `RequestOptionsFile`). Populated by browser captures
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
    /// When `true` AND [`impersonation_available`] confirms the binary has
    /// at least one impersonation target, `download()` passes the GLOBAL
    /// `--impersonate` flag so every request yt-dlp makes — including the
    /// m3u8 manifest and segment fetches that follow the initial page
    /// load, not just the generic extractor's own webpage fetch — mimics a
    /// real browser's TLS/HTTP fingerprint (curl_cffi). That's what
    /// defeats Cloudflare's anti-bot 403 on browser-captured HLS/DASH and
    /// pasted stream URLs; header forwarding alone can't, because the
    /// block is on the TLS handshake. Read from the `ytdlp_impersonate`
    /// setting at spawn time.
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

/// Whether `probe_raw` should even attempt the (async, subprocess-spawning)
/// impersonation-availability check. Pure/sync so it's unit-testable
/// without a real yt-dlp binary — the actual availability check still has
/// to happen separately since it requires spawning a process.
///
/// Gated on the user's `ytdlp_impersonate` setting alone, deliberately
/// matching `download()`, which gates on the identical setting via
/// `job.impersonate` (see `queue.rs`'s `YtdlpJob` construction). Probe and
/// download must agree: a user who turned the setting off gets an
/// unimpersonated probe, full stop, and one who left it on gets the same
/// treatment at probe time that the subsequent download will use.
///
/// Notably NOT gated on referrer presence. An earlier version required a
/// referrer too, on the reasoning that impersonation without one still
/// 403s a Cloudflare-fronted host. That's true of *that* host, but it made
/// the flag unreachable for the app's own paste-a-URL flow, which has no
/// referring page — so pasting a link from a host gated on TLS
/// fingerprint alone (no Referer check) failed at the probe step even
/// though the download that followed would have impersonated fine.
/// Referer forwarding and impersonation are independent defences; apply
/// each whenever it's available rather than making one contingent on the
/// other.
fn wants_impersonation_probe(impersonate: bool) -> bool {
    impersonate
}

/// Spawn `yt-dlp --dump-single-json` against `url` and return the parsed
/// (but not yet lifted) payload. Shared by [`probe`] (→ [`ProbeResult`])
/// and [`probe_media_formats`] (→ [`crate::wire::MediaFormat`]) so the
/// subprocess wiring — timeout, referrer/impersonation gating — isn't
/// duplicated between the two public entry points.
///
/// `impersonate` is the caller-resolved value of the user's
/// `ytdlp_impersonate` setting — see [`wants_impersonation_probe`] for why
/// it's a required, explicit input rather than something this function
/// infers from `referrer`.
///
/// `timeout_duration` caps the probe subprocess (spawn + stdout drain).
/// Default callers should pass a few seconds — the user is waiting on the
/// formats dialog when this is called. The impersonation-availability
/// check that precedes it is bounded separately by
/// [`IMPERSONATE_PROBE_TIMEOUT`] and does not draw on this budget.
async fn probe_raw(
    url: &str,
    binary_path: &Path,
    timeout_duration: Duration,
    referrer: Option<&str>,
    impersonate: bool,
) -> Result<wire::RawInfo, YtdlpError> {
    if !binary_exists(binary_path).await {
        return Err(YtdlpError::NotInstalled);
    }

    // The availability probe is itself a subprocess spawn, so skip it
    // entirely when the setting already says no.
    //
    // Resolved *outside* `timeout_duration`, which covers the probe
    // itself. It used to run inside, and since it carries its own
    // `IMPERSONATE_PROBE_TIMEOUT` (3s) — the same order as the default
    // `ytdlp_probe_timeout_ms` — the first probe against a given binary
    // could spend the caller's entire budget on
    // `--list-impersonate-targets` and then time out having never probed
    // anything at all. The result is cached per binary fingerprint, so
    // this is a first-call-per-binary cost either way; what changed is
    // that it no longer eats the probe's own allowance.
    let should_impersonate = if wants_impersonation_probe(impersonate) {
        impersonation_available(binary_path).await
    } else {
        false
    };

    let url_string = url.to_string();
    let referrer_string = referrer.map(|r| r.to_string());
    let binary = binary_path.to_path_buf();
    let task = async move {
        let mut cmd = Command::new(&binary);
        cmd.arg("--dump-single-json")
            .arg("--no-warnings")
            .arg("--no-playlist")
            .arg("--no-call-home")
            .arg("--skip-download");
        if let Some(referrer) = referrer_string.as_deref() {
            cmd.arg("--referer").arg(referrer);
        }
        for arg in impersonate_args(should_impersonate) {
            cmd.arg(arg);
        }
        cmd.arg(&url_string)
            // When the timeout below fires, this future is dropped
            // mid-await; `kill_on_drop` is what turns that drop into an
            // actual process kill rather than leaving a yt-dlp running in
            // the background, still doing network I/O for an answer
            // nobody will read.
            .kill_on_drop(true)
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
        Ok(raw)
    };

    match timeout(timeout_duration, task).await {
        Ok(result) => result,
        Err(_) => Err(YtdlpError::Timeout(timeout_duration)),
    }
}

/// Run `yt-dlp --dump-single-json` against `url`. Returns a [`ProbeResult`]
/// when the URL is recognized, or a typed error variant otherwise.
///
/// `referrer` is `None` for every existing caller today (pasted-URL probe
/// has no browser context to source one from). `impersonate` should be
/// the caller's resolved `ytdlp_impersonate` setting — see
/// [`wants_impersonation_probe`] for why it's required explicitly rather
/// than inferred from `referrer`.
pub async fn probe(
    url: &str,
    binary_path: &Path,
    timeout_duration: Duration,
    referrer: Option<&str>,
    impersonate: bool,
) -> Result<ProbeResult, YtdlpError> {
    let raw = probe_raw(url, binary_path, timeout_duration, referrer, impersonate).await?;
    let probe = raw.into_probe(url);
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
}

/// Probe `url` the same way [`probe`] does, but return the browser
/// extension-facing [`crate::wire::MediaFormat`] list instead of the
/// internal [`ProbeResult`]/[`Format`] shape. A separate entry point
/// rather than an extra field on `ProbeResult` — see [`crate::wire::MediaFormat`]'s
/// doc comment for why the two shapes are kept apart (internal-only
/// mapping: no Tauri command, no CDN URL reaching the desktop frontend).
///
/// Used by the pipe server's `Inbound::ProbeMedia` handler so a user
/// pasting a Cloudflare-fronted stream URL into the *extension* gets the
/// same 403 fix Phase 1 gave the desktop download path.
///
/// Unlike [`probe`], this does NOT apply the "generic extractor + no
/// formats ⇒ `Unsupported`" fallback: a yt-dlp exit that's genuinely a
/// failure (unsupported URL, timeout, process error, …) still propagates
/// as an `Err` here exactly like it does from `probe`, but a *successful*
/// probe with an empty (or entirely audio-only / URL-less, post-filter)
/// format list just yields `Ok(vec![])` — there's no `ProbeResult` for
/// callers to fall back to inspecting here, so there's nothing to gain by
/// forcing that case into a typed error instead of an empty list.
///
/// `impersonate` should be the caller's resolved `ytdlp_impersonate`
/// setting — see [`wants_impersonation_probe`] for why it's required
/// explicitly rather than inferred from `referrer`.
pub async fn probe_media_formats(
    url: &str,
    binary_path: &Path,
    timeout_duration: Duration,
    referrer: Option<&str>,
    impersonate: bool,
) -> Result<Vec<crate::wire::MediaFormat>, YtdlpError> {
    let raw = probe_raw(url, binary_path, timeout_duration, referrer, impersonate).await?;
    Ok(raw.into_media_formats())
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
    tokio::fs::create_dir_all(&job.temp_dir).await?;

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
        // Six pipe-delimited fields, parsed by `progress::parse_line`.
        //
        // `total_bytes` is null for DASH/HLS sources (YouTube, most
        // live-derived streams) — those expose `total_bytes_estimate`
        // instead, so yt-dlp's `field,alt_field` syntax picks whichever is
        // non-null. **Both alternatives need the `progress.` prefix**: the
        // template's root namespace holds only `info` and `progress`, so a
        // bare `total_bytes_estimate` resolves against the root, finds
        // nothing, and silently yields `NA` forever. That typo is why the
        // bar sat empty for the entire run on every extension-captured HLS
        // stream — the intended fallback was never actually reachable.
        //
        // `fragment_index` / `fragment_count` are the second line of
        // defence: a live-derived manifest reports neither byte total, but
        // fragment counts still let `Tick::effective_total` estimate one.
        .arg("--progress-template")
        .arg(
            "%(progress.downloaded_bytes)s\
             |%(progress.total_bytes,progress.total_bytes_estimate)s\
             |%(progress.speed)s\
             |%(progress.eta)s\
             |%(progress.fragment_index)s\
             |%(progress.fragment_count)s",
        )
        // Post-processing (the ffmpeg merge / remux / fixup) has its own
        // progress-hook namespace. Subscribing to it is how we know the
        // row genuinely reached the merge phase.
        //
        // The pump used to *infer* this from a drop in the byte counter,
        // which is wrong for anything fragmented: a retried fragment or a
        // resumed run regresses the counter mid-download, stranding a
        // still-downloading HLS row on "Merging audio + video…" — a state
        // that (before this change) also had no pause button.
        .arg("--progress-template")
        .arg(format!("postprocess:{POSTPROCESS_TAG}%(progress.status)s"))
        // Tag-prefix the printed path so we can recognize the line in
        // stdout without colliding with progress ticks or banners. yt-dlp
        // expands the literal prefix before the placeholder.
        .arg("--print")
        .arg("after_move:unduhin-final-path:%(filepath)s")
        // Scratch (`.part`, `.ytdl`, `.part-Frag<N>.part`) goes to
        // `temp:`, the finished file to `home:`. Previously everything
        // landed in the user's download folder, where the churn of
        // per-fragment temp files looked like the app was malfunctioning.
        //
        // **`--output` must stay relative.** yt-dlp ignores `--paths`
        // outright when the output template contains an absolute path, so
        // moving the directory into `--paths home:` and the bare
        // `<stem>.%(ext)s` into `--output` is a single indivisible change.
        .arg("--paths")
        .arg(format!("home:{}", job.output_dir.display()))
        .arg("--paths")
        .arg(format!("temp:{}", job.temp_dir.display()))
        .arg("--output")
        .arg(&job.output_template);
    if let Some(ffmpeg) = job.ffmpeg_path.as_deref() {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    // Global speed cap. yt-dlp's `--limit-rate` accepts a raw bytes/sec
    // integer; `0`/`None` means no flag (unlimited).
    if let Some(bps) = job.limit_rate_bps.filter(|b| *b > 0) {
        cmd.arg("--limit-rate").arg(bps.to_string());
    }
    // Browser impersonation via the GLOBAL `--impersonate` flag — NOT
    // `--extractor-args "generic:impersonate"`, which only impersonates the
    // generic extractor's own webpage fetch. The m3u8 manifest and segment
    // requests that follow still went out on yt-dlp's normal networking
    // stack and got 403'd by Cloudflare's bot management; verified A/B
    // against a real Cloudflare-fronted URL with identical `--referer`.
    //
    // Unlike the extractor-args form, the global flag is NOT a safe no-op
    // when the binary lacks impersonation support: yt-dlp raises
    // `YoutubeDLError` inside `YoutubeDL.__init__` — before any download
    // starts — when `--impersonate` is passed but no `curl_cffi`-backed
    // request handler is registered (verified against yt-dlp source: the
    // `impersonate` param import into `YoutubeDL.__init__` calls
    // `_impersonate_target_available`, which is `False` when no
    // `ImpersonateRequestHandler` exists, and that raises immediately).
    // `impersonation_available()` runs `--list-impersonate-targets` first
    // and only this branch passes the flag — required because
    // `ytdlp_binary_path` (settings.rs) is user-settable, so the bundled
    // build having curl_cffi is no guarantee for every run.
    //
    // No explicit target (`--impersonate ""`) — auto-selects among
    // whatever's available rather than hardcoding a client/version that
    // ages out; verified this parses correctly as two separate args (the
    // empty string doesn't get swallowed as a value for a later flag).
    let should_impersonate = job.impersonate && impersonation_available(&job.binary_path).await;
    for arg in impersonate_args(should_impersonate) {
        cmd.arg(arg);
    }
    // The captured request context (cookies included) reaches yt-dlp
    // through a private options file, never argv: any process can read
    // another's command line (`ps`, Task Manager), and on macOS that
    // includes other users' processes. Deleted when this function returns,
    // however it returns.
    let request_args = request_args(job.user_agent.as_deref(), &job.extra_headers);
    let _request_file = if request_args.is_empty() {
        None
    } else {
        let file = RequestOptionsFile::write(&job.temp_dir, &request_args)?;
        cmd.arg("--config-locations").arg(file.path());
        Some(file)
    };
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
        // Backstop for the paths this function can't reach: if the future
        // is dropped rather than cancelled cooperatively, tokio kills the
        // child. `ProcessTreeGuard`'s own `Drop` widens that to the whole
        // tree. `download()` was the only one of this module's three spawn
        // sites missing this.
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(unix)]
    {
        // Make the child the leader of a new process group, so its pid is
        // also its pgid and `ProcessTreeGuard` can reap the PyInstaller
        // re-exec and ffmpeg with one `killpg`. This is load-bearing for
        // safety, not just for completeness: without it the child stays in
        // *our* group, and the guard would signal the app itself.
        cmd.process_group(0);
    }

    tracing::debug!(
        url = %job.url,
        format = %job.format_selector,
        "ytdlp: spawning"
    );

    let mut child = cmd.spawn()?;
    // Adopt the tree *before* touching the pipes. yt-dlp.exe is a
    // PyInstaller one-file build whose bootloader runs the real downloader
    // as its own child, and yt-dlp spawns ffmpeg on top of that — killing
    // only the PID we spawned leaves a live downloader writing to the
    // output path. See `process_tree` for the full story.
    let tree = ProcessTreeGuard::adopt(&child);
    let stdout = child.stdout.take().ok_or_else(|| YtdlpError::Process {
        code: -1,
        message: "stdout missing".into(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| YtdlpError::Process {
        code: -1,
        message: "stderr missing".into(),
    })?;

    // Drain stderr concurrently — yt-dlp can block on a full pipe.
    //
    // This is also where post-processing is detected. yt-dlp writes
    // `postprocess:` progress ticks to **stderr**, not stdout (verified
    // against the real binary: a `--remux-video` run put the download
    // ticks and the `after_move:` line on stdout and all four
    // `started`/`finished` post-processor ticks on stderr). Watching for
    // the tag on stdout looks right and silently never fires.
    //
    // Every line still accumulates into the buffer `classify_exit` reads
    // — the tag lines are inert there.
    let pp_tx = progress_tx.clone();
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut buf = String::new();
        // yt-dlp ticks this hook for every post-processor it runs, and
        // twice each (`started`, `finished`); the row only enters
        // `Muxing` once.
        let mut postprocess_emitted = false;
        while let Ok(Some(line)) = reader.next_line().await {
            if !postprocess_emitted && is_postprocess_line(&line) {
                tracing::debug!("ytdlp: post-processing started");
                emit(pp_tx.as_ref(), ProgressEvent::PostProcessing);
                postprocess_emitted = true;
            }
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
                // Kill the tree first, then the direct child as a fallback
                // for the (logged) case where the job object couldn't be
                // created. Order matters: until every descendant is gone
                // they hold inherited duplicates of our stdout/stderr write
                // handles, and the drain below never sees EOF.
                tree.terminate();
                let _ = child.start_kill();
                // Bounded on purpose. `Core::remove` awaits this worker
                // synchronously before it deletes anything on disk, so an
                // unbounded wait here is an unkillable UI hang — which is
                // precisely how "I deleted the row and nothing happened"
                // used to present. If something escaped the job object,
                // give up on the drain and let `kill_on_drop` and the
                // guard's `Drop` finish the job.
                if timeout(CANCEL_DRAIN_TIMEOUT, async {
                    let _ = child.wait().await;
                    let _ = stderr_task.await;
                })
                .await
                .is_err()
                {
                    tracing::warn!(
                        timeout = ?CANCEL_DRAIN_TIMEOUT,
                        "ytdlp: child did not drain after cancel; abandoning pipes"
                    );
                }
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
                let total = tick.effective_total();
                tracing::debug!(
                    downloaded = tick.downloaded,
                    total = ?total,
                    speed = ?tick.speed_bps,
                    eta = ?tick.eta,
                    fragment = ?tick.fragment_index,
                    fragments = ?tick.fragment_count,
                    "ytdlp: tick"
                );
                if !started_emitted {
                    emit(progress_tx.as_ref(), ProgressEvent::Started {
                        total,
                        segments: 1,
                        resumed_bytes: 0,
                    });
                    started_emitted = true;
                }
                last_downloaded = tick.downloaded;
                if total.is_some() { last_total = total; }
                emit(progress_tx.as_ref(), ProgressEvent::Tick {
                    downloaded: tick.downloaded,
                    total: total.or(last_total),
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

/// Literal prefix on the `postprocess:` progress template, so the line is
/// recognizable in stdout without colliding with download ticks, banners,
/// or the `after_move:` path line.
const POSTPROCESS_TAG: &str = "unduhin-postprocess:";

/// How long the cancel path waits for the child to exit and the stderr
/// pipe to reach EOF before giving up and returning anyway.
///
/// A cancel is always someone waiting — `Core::remove` blocks on this
/// worker before it touches the disk, and the queue can't reclaim the row
/// until it exits. A stuck drain here used to surface as a dead Delete
/// button, so the drain gets a budget rather than a promise.
const CANCEL_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `line` is one of yt-dlp's post-processing progress ticks.
fn is_postprocess_line(line: &str) -> bool {
    line.trim_start().starts_with(POSTPROCESS_TAG)
}

/// Per-download scratch directory for yt-dlp's `--paths temp:`.
///
/// Prefers `%TEMP%\Unduhin\<id>` so the churn of `.part` / `.ytdl` /
/// `.part-Frag<N>.part` files never appears in the user's download folder.
///
/// Falls back to a dot-directory inside `output_dir` when the two are on
/// different volumes. yt-dlp finishes by `shutil.move`-ing temp → home,
/// which is an atomic rename within a volume but a full byte copy across
/// one — and a category folder may well live on another drive, where that
/// would mean re-copying multi-gigabyte videos at the end of every
/// download.
pub fn scratch_dir_for(output_dir: &Path, id: i64) -> PathBuf {
    let base = scratch_root();
    if same_volume(&base, output_dir) {
        base.join(id.to_string())
    } else {
        output_dir.join(SCRATCH_DIR_NAME).join(id.to_string())
    }
}

/// Name of the fallback scratch directory created inside the download
/// folder when `%TEMP%` is on a different volume.
const SCRATCH_DIR_NAME: &str = ".unduhin-tmp";

/// Root of the shared scratch area: `%TEMP%\Unduhin` (or the platform
/// equivalent). Every per-download directory lives directly under it and
/// is named for its download id, so the startup sweep
/// ([`crate::Core::open`]) and the Settings → "Clear temporary data"
/// action can enumerate and prune it.
pub fn scratch_root() -> PathBuf {
    std::env::temp_dir().join("Unduhin")
}

/// Whether two paths live on the same volume.
///
/// This decides whether the scratch dir can sit in the system temp dir or
/// has to live beside the output file. The error costs are deliberately
/// asymmetric, so both arms below fail to `false`: a wrong `false` costs a
/// hidden `.unduhin-tmp` directory next to the download, while a wrong
/// `true` costs a multi-gigabyte cross-device copy at the end of every
/// download.
///
/// Windows compares the path prefix (`C:`, `\\server\share`), which is what
/// decides rename-vs-copy for `shutil.move`. Unix compares `st_dev`,
/// walking up to the nearest existing ancestor because neither path is
/// guaranteed to exist yet. On macOS the optimistic answer would be wrong
/// often: `/Volumes/*` external disks are routine, and modern APFS puts
/// `$HOME` on a different device from the read-only system volume.
fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::path::Component;
        let prefix = |p: &Path| match p.components().next() {
            Some(Component::Prefix(pre)) => Some(pre.as_os_str().to_ascii_lowercase()),
            _ => None,
        };
        match (prefix(a), prefix(b)) {
            (Some(x), Some(y)) => x == y,
            // A relative or prefix-less path (only really reachable from
            // tests) can't be reasoned about — don't risk a cross-volume
            // copy on a guess.
            _ => false,
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Neither path need exist yet, so climb to the nearest ancestor
        // that does. That ancestor is on the same filesystem as the leaf
        // would be, short of a mount appearing in between.
        let device_of = |p: &Path| -> Option<u64> {
            let mut cur = p;
            loop {
                if let Ok(meta) = std::fs::metadata(cur) {
                    return Some(meta.dev());
                }
                cur = cur.parent()?;
            }
        };
        matches!((device_of(a), device_of(b)), (Some(x), Some(y)) if x == y)
    }
    #[cfg(not(any(target_os = "windows", unix)))]
    {
        let _ = (a, b);
        false
    }
}

/// yt-dlp options carrying a download's captured request context.
///
/// The captured User-Agent and Referer go through yt-dlp's dedicated flags
/// rather than `--add-header`. `--add-header User-Agent:…` is unreliable —
/// extractors set their own UA and can override it, and a non-browser UA is
/// exactly what trips Referer/hotlink-protection 403s. `--user-agent` /
/// `--referer` are authoritative. A custom UA from the global `user_agent`
/// setting still wins over the captured one.
fn request_args(user_agent: Option<&str>, extra_headers: &[(String, String)]) -> Vec<String> {
    let sanitized = sanitize_extra_headers(extra_headers);
    let mut args = Vec::new();
    let captured_ua = sanitized
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("user-agent"))
        .map(|(_, v)| v.as_str());
    if let Some(ua) = user_agent.or(captured_ua) {
        args.extend(["--user-agent".to_string(), ua.to_string()]);
    }
    if let Some((_, referer)) = sanitized
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("referer"))
    {
        args.extend(["--referer".to_string(), referer.clone()]);
    }
    for (name, value) in &sanitized {
        // Sent via their dedicated flags above; skipped here so yt-dlp
        // doesn't see conflicting duplicates.
        if name.eq_ignore_ascii_case("user-agent") || name.eq_ignore_ascii_case("referer") {
            continue;
        }
        args.extend(["--add-header".to_string(), format!("{name}:{value}")]);
    }
    args
}

/// A yt-dlp options file (`--config-locations`) holding `args`, readable
/// only by the current user, removed on drop.
struct RequestOptionsFile {
    path: PathBuf,
}

impl RequestOptionsFile {
    fn write(dir: &Path, args: &[String]) -> std::io::Result<Self> {
        use std::io::Write as _;

        let path = dir.join("request-options.conf");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Owner-only from the first byte. (On Windows the scratch dir
            // under %TEMP% is already private to the user.)
            opts.mode(0o600);
        }
        let mut file = opts.open(&path)?;
        // Removed from here on, even if the write fails part-way.
        let guard = Self { path };
        file.write_all(options_file_contents(args).as_bytes())?;
        file.sync_all()?;
        Ok(guard)
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RequestOptionsFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Render `args` as a yt-dlp options file.
///
/// yt-dlp splits the file with Python's `shlex.split(…, comments=True)`
/// (POSIX rules), so every argument is single-quoted — inside single quotes
/// nothing is special, `#` included — with an embedded `'` written as
/// `'"'"'`. Without a BOM yt-dlp decodes the file in the locale encoding,
/// which is not UTF-8 on most Windows systems; the `coding:` line pins it.
fn options_file_contents(args: &[String]) -> String {
    let mut out = String::from("# coding: utf-8\n");
    for pair in args.chunks(2) {
        let line: Vec<String> = pair
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', r#"'"'"'"#)))
            .collect();
        out.push_str(&line.join(" "));
        out.push('\n');
    }
    out
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
            // NUL cannot survive the trip through the options file either.
            if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
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

/// Cache key for [`impersonation_available`]: the binary path plus a
/// coarse content fingerprint (mtime + size), not the path alone.
///
/// `ytdlp_binary_path` ([`crate::settings::settings_keys::YTDLP_BINARY_PATH`])
/// is user-settable, so keying by path alone was already required to
/// avoid conflating two different configured binaries. But the path is
/// also NOT stable content: `Core::install_tool` performs an in-place
/// yt-dlp update by overwriting the same fixed `managed_dir()` path (see
/// `tooling.rs`'s `install` / `resolve_path`), so a user who updates
/// yt-dlp from Settings → Media keeps the exact same `PathBuf` the cache
/// is keyed by. Without the fingerprint, a build cached as "no targets"
/// before an update would silently stay cached as "no targets" for the
/// rest of the process even after updating to a build that has them —
/// reintroducing the 403s this phase exists to fix, now with no error to
/// debug. Including mtime+size means an updated binary naturally misses
/// the cache and gets re-probed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ImpersonateCacheKey {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
}

/// Per-binary-fingerprint cache for [`impersonation_available`]. See
/// [`ImpersonateCacheKey`] for why the key is more than just the path.
static IMPERSONATE_CACHE: OnceLock<Mutex<HashMap<ImpersonateCacheKey, bool>>> = OnceLock::new();

/// The cache's `Mutex`, recovering from poisoning instead of propagating
/// the panic. A panic elsewhere while this lock happened to be held
/// should not permanently wedge every future download's impersonation
/// check — degrading to "treat the cache as empty and re-probe" is a far
/// smaller blast radius than a poisoned lock taking down the download
/// path for the rest of the process.
fn impersonate_cache() -> &'static Mutex<HashMap<ImpersonateCacheKey, bool>> {
    IMPERSONATE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How long [`impersonation_available`] waits for
/// `--list-impersonate-targets` before giving up. This is a fast, purely
/// local metadata query (no network), so a couple of seconds is generous
/// — but it still needs *some* bound: `download()` awaits this on the hot
/// path for the first download against a given binary, and a hung yt-dlp
/// process (corrupt/partial binary, antivirus real-time-scanning the exe,
/// a network filesystem stall) would otherwise wedge the queue worker
/// with no error surfaced. Mirrors the same defensive shape as
/// [`probe`]'s subprocess timeout, just with a fixed constant instead of
/// a caller-supplied duration — this call has no user-facing "how long
/// should this wait" knob to plumb through.
const IMPERSONATE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Whether `binary_path` has at least one available browser-impersonation
/// target, i.e. was built with the optional `curl_cffi` backend.
///
/// This gates whether `download()` may pass the global `--impersonate`
/// flag. That flag is NOT a safe no-op when unsupported: passing it on a
/// build with no impersonation-capable request handler makes yt-dlp raise
/// `YoutubeDLError` inside `YoutubeDL.__init__`, before any download
/// attempt — a hard failure, not a warning (verified against yt-dlp
/// source: `_impersonate_target_available` returns `False` when no
/// `ImpersonateRequestHandler` is registered, and `__init__` raises on
/// that). Only `--extractor-args generic:impersonate` degrades gracefully
/// to a warning; the global flag does not, which is exactly why this
/// module stopped using the extractor-args form.
///
/// Result is cached for the process lifetime, keyed by path + content
/// fingerprint — see [`ImpersonateCacheKey`]. Reading the file's metadata
/// to build the key doubles as the "binary exists" check: a missing file
/// returns `false` immediately without ever spawning a process.
pub async fn impersonation_available(binary_path: &Path) -> bool {
    let Ok(meta) = tokio::fs::metadata(binary_path).await else {
        return false;
    };
    let key = ImpersonateCacheKey {
        path: binary_path.to_path_buf(),
        modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
        len: meta.len(),
    };

    let cache = impersonate_cache();
    if let Some(cached) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
    {
        return *cached;
    }

    let available = probe_impersonation_targets(binary_path, IMPERSONATE_PROBE_TIMEOUT).await;

    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, available);
    available
}

/// Run `yt-dlp --list-impersonate-targets` and report whether it lists at
/// least one genuinely available target. Never propagates an error — a
/// missing binary, non-zero exit, timeout, or unparseable output all mean
/// "no impersonation", which is the safe default (see
/// [`impersonation_available`] for why passing `--impersonate` without
/// confirming this first is unsafe).
///
/// `timeout_duration` is a parameter (rather than always reading
/// [`IMPERSONATE_PROBE_TIMEOUT`] directly) so tests can exercise the
/// timeout path with a short duration instead of waiting out the real
/// production value.
async fn probe_impersonation_targets(binary_path: &Path, timeout_duration: Duration) -> bool {
    if !binary_exists(binary_path).await {
        return false;
    }
    let mut cmd = Command::new(binary_path);
    cmd.arg("--list-impersonate-targets")
        .arg("--no-warnings")
        .arg("--no-update")
        // If the timeout below fires, the `cmd.output()` future is
        // dropped mid-await; `kill_on_drop` is what turns that drop into
        // an actual process kill instead of leaving a hung yt-dlp running
        // in the background.
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let output = match timeout(timeout_duration, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) | Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    parse_impersonate_targets(&String::from_utf8_lossy(&output.stdout))
}

/// Parse `yt-dlp --list-impersonate-targets` stdout. Returns `true` iff at
/// least one row represents a genuinely available target.
///
/// Real output (yt-dlp 2026.03.17) looks like:
///
/// ```text
/// [info] Available impersonate targets
/// Client        OS           Source
/// ------------------------------------
/// Chrome-136    Macos-15     curl_cffi
/// Safari-17.2   Ios-17.2     curl_cffi
/// ```
///
/// Critically, yt-dlp *always* renders the full table, even when
/// curl_cffi is entirely missing: it still lists a row for every
/// well-known client (Chrome, Safari, Firefox, Edge, Tor), tagging each
/// unavailable one with `(unavailable)` in the Source column instead of
/// omitting the row. So a naive "any data row present" check would report
/// `true` on a build with zero real impersonation support — exactly the
/// build [`impersonation_available`] exists to detect. Only rows WITHOUT
/// `(unavailable)` count.
fn parse_impersonate_targets(stdout: &str) -> bool {
    let lines: Vec<String> = stdout
        .lines()
        .map(|line| strip_ansi(line).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    // Require the table's own header/separator shape before trusting any
    // row as data. This is what rejects a Python traceback or an
    // unrelated error message: such text has no "Client ... Source"
    // header immediately followed by a dashed separator, so it never
    // reaches the row-counting step below. Anchoring on structure (not
    // just "no '(unavailable)' substring") is what keeps garbage/error
    // output from being misread as an available target.
    let Some(header_idx) = lines
        .iter()
        .position(|line| line.starts_with("Client") && line.contains("Source"))
    else {
        return false;
    };
    let Some(separator) = lines.get(header_idx + 1) else {
        return false;
    };
    if separator.is_empty() || !separator.chars().all(|c| c == '-') {
        return false;
    }

    lines[header_idx + 2..]
        .iter()
        .any(|line| !line.contains("(unavailable)"))
}

/// Strip ANSI SGR escape sequences (`\x1b[...m`). yt-dlp only emits color
/// when stdout is a TTY, and ours is always a piped `Stdio`, so this is a
/// no-op in practice — kept defensive rather than load-bearing, since
/// nothing here should crash if that assumption ever changes.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Pure decision for the `--impersonate` args `download()` should append,
/// given that impersonation should be used — i.e. the job requested it
/// AND [`impersonation_available`] confirmed a target exists. Split out
/// from the surrounding `Command`-building so this decision is
/// unit-testable without spawning a real yt-dlp process.
///
/// No explicit target (`--impersonate` followed by an empty-string arg,
/// equivalent to `--impersonate=""`) so yt-dlp auto-selects among
/// whatever's available rather than hardcoding a client/version that ages
/// out of the bundled curl_cffi's supported list.
fn impersonate_args(should_impersonate: bool) -> Vec<String> {
    if should_impersonate {
        vec!["--impersonate".to_string(), String::new()]
    } else {
        Vec::new()
    }
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

    // --- impersonation gating -----------------------------------------

    #[test]
    fn impersonate_args_present_when_should_impersonate() {
        let args = impersonate_args(true);
        assert_eq!(args, vec!["--impersonate".to_string(), String::new()]);
        // The old per-extractor form must never come back — it's what
        // silently left manifest/segment requests unimpersonated.
        assert!(!args.iter().any(|a| a.contains("generic:impersonate")));
    }

    #[test]
    fn impersonate_args_absent_when_not_should_impersonate() {
        // Covers both "available but toggle off" and "toggle on but
        // unavailable" — both collapse to `should_impersonate = false`
        // before reaching this function, which is the point of gating in
        // `download()` rather than here.
        assert!(impersonate_args(false).is_empty());
    }

    /// Real `--list-impersonate-targets` output captured from the bundled
    /// yt-dlp.exe (2026.03.17) via
    /// `yt-dlp --list-impersonate-targets --no-warnings --no-update`.
    const POPULATED_TARGETS_OUTPUT: &str = "\
[info] Available impersonate targets
Client        OS           Source
------------------------------------
Chrome-133    Macos-15     curl_cffi
Chrome-136    Macos-15     curl_cffi
Safari-17.2   Ios-17.2     curl_cffi
Edge-99       Windows-10   curl_cffi
";

    #[test]
    fn parse_impersonate_targets_true_for_populated_table() {
        assert!(parse_impersonate_targets(POPULATED_TARGETS_OUTPUT));
    }

    #[test]
    fn parse_impersonate_targets_false_for_empty_output() {
        assert!(!parse_impersonate_targets(""));
        assert!(!parse_impersonate_targets("\n\n  \n"));
    }

    #[test]
    fn parse_impersonate_targets_false_when_every_row_is_unavailable() {
        // yt-dlp always renders the full known-clients table, even with
        // zero curl_cffi support — missing targets get a row tagged
        // `(unavailable)` in the Source column rather than being omitted.
        // This is the exact shape a build with no impersonation backend
        // produces (traced through yt-dlp's `__init__.py`
        // `list_impersonate_targets` handling: `known_targets` are
        // inserted with `f'{known_handler} (unavailable)'` whenever no
        // matching entry exists in `available_targets`). A naive
        // "any data row present" check would wrongly report available
        // here.
        let output = "\
[info] Available impersonate targets
Client    OS    Source
------------------------------------
Chrome    -     curl_cffi (unavailable)
Safari    -     curl_cffi (unavailable)
Firefox   -     curl_cffi>=0.10 (unavailable)
Edge      -     curl_cffi (unavailable)
Tor       -     curl_cffi>=0.11 (unavailable)
";
        assert!(!parse_impersonate_targets(output));
    }

    #[test]
    fn parse_impersonate_targets_false_for_garbage_or_error_output() {
        assert!(!parse_impersonate_targets(
            "Traceback (most recent call last):\n  File ...\n"
        ));
        assert!(!parse_impersonate_targets(
            "ERROR: something unrelated failed\n"
        ));
        // Header + separator with no data rows at all.
        assert!(!parse_impersonate_targets(
            "[info] Available impersonate targets\nClient    OS    Source\n------\n"
        ));
    }

    #[tokio::test]
    async fn impersonation_available_returns_false_for_missing_binary() {
        // No real subprocess involved — `binary_exists` short-circuits
        // before any spawn, so this doubles as the "missing binary never
        // crashes" case.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.exe");
        assert!(!impersonation_available(&missing).await);
    }

    // `Command::new` can spawn a `.cmd` script directly on Windows (no
    // `cmd /c` wrapper needed — verified manually), which is what lets
    // these two tests fake a yt-dlp binary's behavior without needing a
    // real yt-dlp install. Gated to Windows since that mechanism is
    // platform-specific and every caller of this module already runs on
    // Windows in practice.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn probe_impersonation_targets_times_out_instead_of_hanging() {
        // Simulates a hung yt-dlp process — corrupt/partial binary,
        // antivirus real-time-scanning the exe, a network filesystem
        // stall — any of which would otherwise block `download()`'s
        // first call for a given binary path forever. The script spins
        // forever via `goto` rather than shelling out to a sleep/ping
        // command: a real hung yt-dlp.exe is a single process, and a
        // `goto`-loop keeps this test that way too — no child process
        // means nothing can outlive `kill_on_drop`'s `TerminateProcess`
        // call on the direct child. (An earlier version of this test used
        // `ping -n 30`, which cmd.exe runs as a child process; Windows
        // doesn't cascade-kill children, so the orphaned `ping.exe`
        // wouldn't stop, and it inherits a duplicate handle to our piped
        // stdout — that inherited handle alone was enough to stall tokio
        // runtime shutdown for the pipe read to actually observe EOF,
        // making the *test* hang for the ping's full duration even though
        // the probe itself returned in ~200ms. Not a production bug, just
        // a bad test double.)
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hangs.cmd");
        tokio::fs::write(&script, "@echo off\r\n:loop\r\ngoto loop\r\n")
            .await
            .unwrap();

        let start = std::time::Instant::now();
        let available = probe_impersonation_targets(&script, Duration::from_millis(200)).await;
        assert!(!available);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "expected the 200ms timeout to fire well before the script's infinite loop, took {:?}",
            start.elapsed()
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn impersonation_available_reprobes_after_binary_is_updated_in_place() {
        // `Core::install_tool` updates yt-dlp by overwriting the SAME
        // fixed `managed_dir()` path (see `tooling.rs`'s `install` /
        // `resolve_path`), so a path-only cache key would keep serving a
        // stale verdict across an in-app update. Simulate exactly that:
        // probe a "no targets" fake binary at `path` (caches `false`),
        // then overwrite the SAME path in place with a "has targets" fake
        // binary and probe again.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake-yt-dlp.cmd");

        tokio::fs::write(&path, "@echo off\r\necho no impersonate targets\r\n")
            .await
            .unwrap();
        assert!(
            !impersonation_available(&path).await,
            "old build should report unavailable"
        );

        // The sleep plus a deliberately different content length means
        // the mtime+size fingerprint changes regardless of the
        // filesystem's mtime resolution.
        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::fs::write(
            &path,
            "@echo off\r\n\
             echo [info] Available impersonate targets\r\n\
             echo Client        OS           Source\r\n\
             echo ------------------------------------\r\n\
             echo Chrome-136    Macos-15     curl_cffi\r\n",
        )
        .await
        .unwrap();

        assert!(
            impersonation_available(&path).await,
            "updated build must be re-probed, not served the stale cached `false`"
        );
    }

    // --- probe() / probe_media_formats() impersonation gating ----------

    #[test]
    fn wants_impersonation_probe_follows_the_setting_alone() {
        // Regression case one: setting off must be `false` even when a
        // referrer is present — an early version inferred impersonation
        // from referrer presence alone and silently overrode the setting.
        assert!(!wants_impersonation_probe(false));
        // Regression case two: setting on must be `true` with no referrer
        // in play — a later version required a referrer too, which made
        // the flag unreachable for the app's referrer-less paste-a-URL
        // flow. This now matches `download()`, which gates on the setting
        // alone via `job.impersonate`.
        assert!(wants_impersonation_probe(true));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn probe_media_formats_omits_impersonate_flag_when_setting_is_off_even_with_referrer() {
        // End-to-end regression test (not just the pure decision function
        // above): drives the real `probe_media_formats` -> `probe_raw`
        // path against a fake yt-dlp binary that records its own argv,
        // with a referrer present but `impersonate: false`. Before the
        // fix, referrer presence alone was enough to trigger the
        // availability probe and append `--impersonate` — silently
        // overriding a user who turned the setting off.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let captured = dir.path().join("captured_args.txt");
        tokio::fs::write(
            &script,
            format!(
                "@echo off\r\n\
                 echo %* > \"{}\"\r\n\
                 echo {{\"extractor\":\"generic\",\"title\":\"t\",\"formats\":[]}}\r\n",
                captured.display()
            ),
        )
        .await
        .unwrap();

        let formats = probe_media_formats(
            "https://cdn.example.com/master.m3u8",
            &script,
            Duration::from_secs(5),
            Some("https://example.com/watch"), // referrer IS present
            false,                             // ytdlp_impersonate setting is OFF
        )
        .await
        .unwrap();
        assert!(formats.is_empty());

        let args = tokio::fs::read_to_string(&captured).await.unwrap();
        assert!(
            !args.contains("--impersonate"),
            "impersonate=false must suppress the flag even with a referrer present, got args: {args}"
        );
        // Referer forwarding is independent of impersonation and must
        // still happen.
        assert!(args.contains("--referer"), "got args: {args}");
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn probe_media_formats_includes_impersonate_flag_when_setting_on_and_referrer_present() {
        // Positive counterpart to the test above, against the same fake
        // binary shape — confirms the fix doesn't just always suppress
        // the flag, it correctly re-enables it when both factors hold.
        // The fake binary also answers `--list-impersonate-targets` with
        // a populated table so `impersonation_available` reports true.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let captured = dir.path().join("captured_args.txt");
        tokio::fs::write(
            &script,
            format!(
                "@echo off\r\n\
                 echo %1 | findstr /C:\"--list-impersonate-targets\" >NUL\r\n\
                 if %ERRORLEVEL% EQU 0 (\r\n\
                 echo [info] Available impersonate targets\r\n\
                 echo Client        OS           Source\r\n\
                 echo ------------------------------------\r\n\
                 echo Chrome-136    Macos-15     curl_cffi\r\n\
                 ) else (\r\n\
                 echo %* > \"{}\"\r\n\
                 echo {{\"extractor\":\"generic\",\"title\":\"t\",\"formats\":[]}}\r\n\
                 )\r\n",
                captured.display()
            ),
        )
        .await
        .unwrap();

        let formats = probe_media_formats(
            "https://cdn.example.com/master.m3u8",
            &script,
            Duration::from_secs(5),
            Some("https://example.com/watch"),
            true, // ytdlp_impersonate setting is ON
        )
        .await
        .unwrap();
        assert!(formats.is_empty());

        let args = tokio::fs::read_to_string(&captured).await.unwrap();
        assert!(args.contains("--impersonate"), "got args: {args}");
        assert!(args.contains("--referer"), "got args: {args}");
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn probe_media_formats_impersonates_with_no_referrer_when_setting_is_on() {
        // The app's own paste-a-URL flow has no referring page, so it
        // probes with `referrer: None`. An earlier version additionally
        // required a referrer before impersonating, which made the flag
        // unreachable here — pasting a link from a host gated on TLS
        // fingerprint alone failed at the probe step even though the
        // download that followed would have impersonated fine.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let captured = dir.path().join("captured_args.txt");
        tokio::fs::write(
            &script,
            format!(
                "@echo off\r\n\
                 echo %1 | findstr /C:\"--list-impersonate-targets\" >NUL\r\n\
                 if %ERRORLEVEL% EQU 0 (\r\n\
                 echo [info] Available impersonate targets\r\n\
                 echo Client        OS           Source\r\n\
                 echo ------------------------------------\r\n\
                 echo Chrome-136    Macos-15     curl_cffi\r\n\
                 ) else (\r\n\
                 echo %* > \"{}\"\r\n\
                 echo {{\"extractor\":\"generic\",\"title\":\"t\",\"formats\":[]}}\r\n\
                 )\r\n",
                captured.display()
            ),
        )
        .await
        .unwrap();

        let formats = probe_media_formats(
            "https://cdn.example.com/master.m3u8",
            &script,
            Duration::from_secs(5),
            None, // no referring page — the paste-a-URL case
            true, // ytdlp_impersonate setting is ON
        )
        .await
        .unwrap();
        assert!(formats.is_empty());

        let args = tokio::fs::read_to_string(&captured).await.unwrap();
        assert!(
            args.contains("--impersonate"),
            "setting on must impersonate even with no referrer, got args: {args}"
        );
        assert!(
            !args.contains("--referer"),
            "no referrer supplied, so none must be forwarded, got args: {args}"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn probe_media_formats_keeps_its_full_timeout_when_the_impersonation_check_is_slow() {
        // The impersonation-availability check used to run *inside*
        // `timeout_duration`. Because it is itself a subprocess spawn with
        // its own 3s cap — the same order as the default
        // `ytdlp_probe_timeout_ms` — the first probe against a given
        // binary could spend the caller's entire budget on
        // `--list-impersonate-targets` and then fail with `Timeout`,
        // having never probed the URL at all. That is exactly the call
        // the user waits on when the popup asks for a stream's qualities.
        //
        // Here the availability check takes ~2s while the probe itself is
        // given only 1s. Hoisted out, the probe still gets its full
        // second and succeeds.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let captured = dir.path().join("captured_args.txt");
        tokio::fs::write(
            &script,
            format!(
                "@echo off\r\n\
                 echo %1 | findstr /C:\"--list-impersonate-targets\" >NUL\r\n\
                 if %ERRORLEVEL% EQU 0 (\r\n\
                 ping -n 3 127.0.0.1 >NUL\r\n\
                 echo [info] Available impersonate targets\r\n\
                 echo Client        OS           Source\r\n\
                 echo ------------------------------------\r\n\
                 echo Chrome-136    Macos-15     curl_cffi\r\n\
                 ) else (\r\n\
                 echo %* > \"{}\"\r\n\
                 echo {{\"extractor\":\"generic\",\"title\":\"t\",\"formats\":[]}}\r\n\
                 )\r\n",
                captured.display()
            ),
        )
        .await
        .unwrap();

        let probe_timeout = Duration::from_secs(1);
        let started = std::time::Instant::now();
        let formats = probe_media_formats(
            "https://cdn.example.com/master.m3u8",
            &script,
            probe_timeout,
            Some("https://example.com/watch"),
            true, // impersonate ON, so the slow availability check runs
        )
        .await
        .expect("the slow availability check must not consume the probe's own budget");
        assert!(formats.is_empty());

        // Proves the check really did run (and really was slow) rather
        // than the test passing because it was skipped outright.
        assert!(
            started.elapsed() > probe_timeout,
            "the whole call should have outlasted the probe's timeout, taking the \
             availability check's ~2s on top; took {:?}",
            started.elapsed()
        );

        let args = tokio::fs::read_to_string(&captured).await.unwrap();
        assert!(args.contains("--impersonate"), "got args: {args}");
    }

    /// A `YtdlpJob` pointed at `script`, writing into `dir`, with every
    /// optional knob off. Keeps the download tests below focused on the
    /// one thing each is actually about.
    #[cfg(any(windows, unix))]
    fn download_job(script: &Path, dir: &Path) -> YtdlpJob {
        YtdlpJob {
            url: "https://cdn.example.com/media.m3u8".into(),
            format_selector: "best".into(),
            output_dir: dir.join("out"),
            temp_dir: dir.join("scratch"),
            output_template: "Some Video.%(ext)s".into(),
            binary_path: script.to_path_buf(),
            ffmpeg_path: None,
            user_agent: None,
            extra_headers: Vec::new(),
            limit_rate_bps: None,
            impersonate: false,
        }
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn download_routes_scratch_through_paths_and_keeps_output_relative() {
        // yt-dlp ignores `--paths` outright when the output template
        // carries an absolute path, so "scratch goes to temp:" and
        // "`--output` is relative" are one invariant, not two. Assert both
        // together — an absolute `--output` sneaking back in would put
        // `.part` / `.ytdl` / `.part-Frag<N>.part` right back in the
        // user's download folder with no other visible symptom.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let captured = dir.path().join("captured_args.txt");
        tokio::fs::write(
            &script,
            format!("@echo off\r\necho %* > \"{}\"\r\n", captured.display()),
        )
        .await
        .unwrap();

        let job = download_job(&script, dir.path());
        let (home, temp) = (job.output_dir.clone(), job.temp_dir.clone());
        // The fake binary exits 0 having downloaded nothing, so the call
        // succeeds with a zero-byte outcome; the argv is the assertion.
        let _ = download(job, CancellationToken::new(), None).await;

        // Windows re-quotes any argument containing a space or a colon,
        // so the captured line reads `--paths "home:C:\…"`.
        let args = tokio::fs::read_to_string(&captured).await.unwrap();
        assert!(
            args.contains(&format!("--paths \"home:{}\"", home.display())),
            "got args: {args}"
        );
        assert!(
            args.contains(&format!("--paths \"temp:{}\"", temp.display())),
            "got args: {args}"
        );
        assert!(
            args.contains("--output \"Some Video.%(ext)s\""),
            "the output template must stay relative or --paths is ignored; got args: {args}"
        );
        assert!(
            !args.contains(&format!("--output \"{}", home.display())),
            "an absolute --output silently disables --paths; got args: {args}"
        );
        // Both directories are created up front — yt-dlp will not make
        // the temp dir itself.
        assert!(tokio::fs::metadata(&temp).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn download_progress_template_prefixes_every_alternate_field() {
        // `%(progress.total_bytes,total_bytes_estimate)s` looks right and
        // is silently dead: alternates resolve from the template root,
        // whose only keys are `info` and `progress`, so the bare second
        // name never matches and the field is `NA` forever. HLS has no
        // `total_bytes`, so the bar sat empty for the whole download.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let captured = dir.path().join("captured_args.txt");
        tokio::fs::write(
            &script,
            format!("@echo off\r\necho %* > \"{}\"\r\n", captured.display()),
        )
        .await
        .unwrap();

        let _ = download(
            download_job(&script, dir.path()),
            CancellationToken::new(),
            None,
        )
        .await;

        let args = tokio::fs::read_to_string(&captured).await.unwrap();
        assert!(
            args.contains("%(progress.total_bytes,progress.total_bytes_estimate)s"),
            "got args: {args}"
        );
        assert!(
            args.contains("%(progress.fragment_index)s"),
            "fragment counts are the fallback when neither byte total exists; got args: {args}"
        );
        assert!(
            args.contains(&format!("postprocess:{POSTPROCESS_TAG}")),
            "got args: {args}"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn post_processing_is_detected_on_stderr_not_stdout() {
        // yt-dlp writes `postprocess:` ticks to **stderr** while download
        // ticks and the `after_move:` line go to stdout. Verified against
        // the real binary with `--remux-video mkv`: stdout carried only
        // the two progress lines and the final path, stderr carried all
        // four `started`/`finished` post-processor ticks.
        //
        // The whole `Muxing` state now hangs off this one signal, and
        // watching the wrong stream fails silently — the row would just
        // never leave `Active`. Hence a test that puts the tag exactly
        // where the real binary puts it.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        tokio::fs::write(
            &script,
            // `^|` — a bare `|` in a .cmd is a pipe operator, not text.
            "@echo off\r\n\
             echo 1024^|2048^|NA^|NA^|NA^|NA\r\n\
             echo unduhin-postprocess:started 1>&2\r\n\
             echo unduhin-postprocess:finished 1>&2\r\n",
        )
        .await
        .unwrap();

        let (tx, mut rx) = broadcast::channel(16);
        download(
            download_job(&script, dir.path()),
            CancellationToken::new(),
            Some(tx),
        )
        .await
        .expect("fake binary exits 0");

        let mut saw_postprocessing = 0usize;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, ProgressEvent::PostProcessing) {
                saw_postprocessing += 1;
            }
        }
        assert_eq!(
            saw_postprocessing, 1,
            "expected exactly one PostProcessing event — yt-dlp ticks the \
             hook per post-processor and twice each, but the row enters \
             Muxing once"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn cancel_kills_grandchildren_and_returns_promptly() {
        // The real failure this guards: `yt-dlp.exe` is a PyInstaller
        // one-file build whose bootloader runs the actual downloader as
        // its own child, and yt-dlp spawns ffmpeg on top of that.
        // `TerminateProcess` on the PID we spawned does not cascade, so
        // the downloader survived a pause and kept writing — and because
        // it inherited our stdout/stderr write handles, the drain never
        // saw EOF and `download()` never returned, which is what made
        // Delete look dead in the UI.
        //
        // `start /b ping` reproduces both halves: a grandchild that
        // outlives its parent and holds the inherited pipe. With the job
        // object in place the whole tree dies and the call returns fast.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp.cmd");
        let marker = dir.path().join("grandchild-alive.txt");
        // The grandchild outlives the .cmd (`start /b` doesn't wait) and
        // keeps writing the marker for a minute unless it is killed.
        tokio::fs::write(
            &script,
            format!(
                "@echo off\r\n\
                 start /b cmd /c \"for /l %%i in (1,1,600) do (echo %%i> \"{}\" & ping -n 2 127.0.0.1 >NUL)\"\r\n\
                 ping -n 60 127.0.0.1 >NUL\r\n",
                marker.display()
            ),
        )
        .await
        .unwrap();

        let cancel = CancellationToken::new();
        let handle = tokio::spawn(download(
            download_job(&script, dir.path()),
            cancel.clone(),
            None,
        ));
        // Give the script time to spawn its grandchild before cancelling.
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(
            tokio::fs::metadata(&marker).await.is_ok(),
            "the fake binary never spawned its grandchild, so this test would pass vacuously"
        );

        let started = std::time::Instant::now();
        cancel.cancel();
        let result = timeout(Duration::from_secs(15), handle)
            .await
            .expect("download() must return after a cancel, not hang on the pipe drain")
            .unwrap();
        assert!(
            matches!(result, Err(YtdlpError::Process { message, .. }) if message == "cancelled")
        );
        assert!(
            started.elapsed() < CANCEL_DRAIN_TIMEOUT,
            "cancel should be prompt, not ride out the drain budget; took {:?}",
            started.elapsed()
        );

        // The grandchild must be gone: if it were still looping it would
        // keep touching the marker.
        let before = tokio::fs::metadata(&marker)
            .await
            .unwrap()
            .modified()
            .unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let after = tokio::fs::metadata(&marker)
            .await
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            before, after,
            "the grandchild survived the cancel and is still writing"
        );
    }

    #[test]
    fn scratch_dir_is_per_download_and_under_the_shared_root() {
        let root = scratch_root();
        // Same volume as %TEMP% (it *is* %TEMP%), so the temp root wins.
        let dir = scratch_dir_for(&root.join("downloads"), 42);
        assert_eq!(dir, root.join("42"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn scratch_dir_falls_back_in_place_across_volumes() {
        // yt-dlp finishes by moving temp → home. Across volumes that is a
        // byte copy, so an 8 GB download would be written twice; keep the
        // scratch next to the output instead.
        let output = Path::new(r"Z:\Media\Clips");
        let dir = scratch_dir_for(output, 7);
        assert_eq!(dir, output.join(SCRATCH_DIR_NAME).join("7"));
    }

    /// Every argument is single-quoted for yt-dlp's `shlex.split`, with an
    /// embedded `'` closed, emitted in double quotes, and reopened.
    #[test]
    fn options_file_quotes_every_argument() {
        let args = vec![
            "--add-header".to_string(),
            r#"Cookie:a=1; b="x y"; c=it's#not-a-comment"#.to_string(),
            "--referer".to_string(),
            "https://example.com/pâge?q=1".to_string(),
        ];
        assert_eq!(
            options_file_contents(&args),
            "# coding: utf-8\n\
             '--add-header' 'Cookie:a=1; b=\"x y\"; c=it'\"'\"'s#not-a-comment'\n\
             '--referer' 'https://example.com/pâge?q=1'\n"
        );
    }

    #[test]
    fn request_args_route_ua_and_referer_through_their_flags() {
        let headers = vec![
            ("User-Agent".to_string(), "browser/1".to_string()),
            ("Referer".to_string(), "https://example.com/".to_string()),
            ("Cookie".to_string(), "s=1".to_string()),
        ];
        assert_eq!(
            request_args(None, &headers),
            vec![
                "--user-agent",
                "browser/1",
                "--referer",
                "https://example.com/",
                "--add-header",
                "Cookie:s=1",
            ]
        );
        // The global setting wins over the captured UA.
        assert_eq!(request_args(Some("mine"), &headers)[1], "mine");
        assert!(request_args(None, &[]).is_empty());
    }

    /// Cookies must never appear in yt-dlp's argv, which other processes can
    /// read. They travel in an owner-only options file that is gone once
    /// the download returns.
    #[cfg(unix)]
    #[tokio::test]
    async fn download_keeps_captured_cookies_off_the_command_line() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-yt-dlp");
        let argv = dir.path().join("argv.txt");
        let conf = dir.path().join("conf.txt");
        let mode = dir.path().join("mode.txt");
        // Record argv, and copy the options file (and its permissions)
        // while yt-dlp would be reading it.
        tokio::fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 echo \"$@\" > '{argv}'\n\
                 while [ $# -gt 0 ]; do\n\
                   if [ \"$1\" = --config-locations ]; then\n\
                     cp \"$2\" '{conf}'; stat -c %a \"$2\" > '{mode}' 2>/dev/null || stat -f %Lp \"$2\" > '{mode}'\n\
                   fi\n\
                   shift\n\
                 done\n",
                argv = argv.display(),
                conf = conf.display(),
                mode = mode.display(),
            ),
        )
        .await
        .unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        let mut job = download_job(&script, dir.path());
        job.extra_headers = vec![("Cookie".into(), "session=top-secret".into())];
        let options_path = job.temp_dir.join("request-options.conf");
        let _ = download(job, CancellationToken::new(), None).await;

        let argv = tokio::fs::read_to_string(&argv).await.unwrap();
        assert!(
            !argv.contains("top-secret"),
            "cookie on the command line: {argv}"
        );
        assert!(argv.contains("--config-locations"), "got argv: {argv}");
        let conf = tokio::fs::read_to_string(&conf).await.unwrap();
        assert!(
            conf.contains("'--add-header' 'Cookie:session=top-secret'"),
            "{conf}"
        );
        assert_eq!(
            tokio::fs::read_to_string(&mode).await.unwrap().trim(),
            "600"
        );
        assert!(
            tokio::fs::metadata(&options_path).await.is_err(),
            "the options file must be removed after the run"
        );
    }
}
