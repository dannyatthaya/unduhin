//! Runtime install + status detection for the external tools yt-dlp and
//! ffmpeg. We deliberately do not bundle these binaries in the installer
//! — Settings → Media has user-facing install/update buttons that call
//! into this module.
//!
//! Resolution order for [`resolve_path`]:
//!
//! 1. The user-overridden absolute path in settings (`ytdlp_binary_path`
//!    or `ffmpeg_binary_path`) — only when non-empty and the file exists.
//! 2. The managed directory under `directories_root()/binaries/` (where
//!    [`install_or_update`] writes).
//! 3. The system PATH — a developer who already has `yt-dlp.exe` on PATH
//!    can use the app without explicitly installing.
//!
//! Installs report progress via dedicated [`CoreEvent`] variants
//! (`ToolInstallProgress` / `ToolInstallCompleted` / `ToolInstallFailed`)
//! so the frontend can show a per-tool progress bar without conflating
//! with regular downloads.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use engine::CancellationToken;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::broadcast;

use crate::event::CoreEvent;
use crate::settings::{self, settings_keys};

/// One of the external tools Unduhin can manage on the user's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    YtDlp,
    Ffmpeg,
}

impl Tool {
    pub fn binary_name(self) -> &'static str {
        match self {
            Tool::YtDlp => {
                if cfg!(target_os = "windows") {
                    "yt-dlp.exe"
                } else {
                    "yt-dlp"
                }
            }
            Tool::Ffmpeg => {
                if cfg!(target_os = "windows") {
                    "ffmpeg.exe"
                } else {
                    "ffmpeg"
                }
            }
        }
    }

    fn settings_key(self) -> &'static str {
        match self {
            Tool::YtDlp => settings_keys::YTDLP_BINARY_PATH,
            Tool::Ffmpeg => settings_keys::FFMPEG_BINARY_PATH,
        }
    }

    fn release(self) -> &'static ToolRelease {
        match self {
            Tool::YtDlp => &YTDLP_RELEASE,
            Tool::Ffmpeg => &FFMPEG_RELEASE,
        }
    }
}

/// Snapshot of a tool's installation state for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub tool: Tool,
    pub installed: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Pinned version that the next [`install_or_update`] will fetch.
    pub latest_known: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolingError {
    #[error("download failed: {0}")]
    Download(String),
    #[error("could not determine install directory")]
    NoInstallDir,
    #[error("archive extraction failed: {0}")]
    Archive(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("verification failed after install: tool did not run")]
    VerifyFailed,
    #[error("integrity check failed: expected SHA-256 {expected}, got {actual}")]
    IntegrityFailed { expected: String, actual: String },
    #[error("could not obtain a checksum to verify the download: {0}")]
    NoChecksum(String),
    #[error("cancelled")]
    Cancelled,
}

/// Release info. We deliberately stay on "latest" rather than pinning a
/// version (YouTube's anti-bot measures require frequent yt-dlp updates),
/// but every download is integrity-checked: the asset is resolved through
/// GitHub's REST API, which reports a per-asset SHA-256 `digest`, and the
/// bytes we fetch must match that digest before the binary is installed or
/// executed. This blocks a TLS-intercepting proxy or a tampered mirror
/// from substituting an arbitrary executable, while still tracking latest.
struct ToolRelease {
    /// Descriptive only — the real version is read back from the binary
    /// after install. Shown as `latest_known` in the UI.
    version: &'static str,
    /// GitHub REST endpoint for the latest release of the asset's repo.
    api_url: &'static str,
    /// Exact asset filename to download and verify.
    asset_name: &'static str,
    /// Companion checksums asset to fall back on if the API omits a
    /// per-asset `digest` (yt-dlp publishes `SHA2-256SUMS`).
    checksums_asset: Option<&'static str>,
    /// Set when the downloaded asset is a zip archive containing the
    /// binary at the path inside it that ends with `binary_name()`.
    is_archive: bool,
}

/// yt-dlp release. Resolved via the GitHub API so the install button always
/// pulls the most recent stable release (a pinned version goes stale within
/// weeks) while still verifying the SHA-256 the API reports for the asset.
const YTDLP_RELEASE: ToolRelease = ToolRelease {
    version: "latest",
    api_url: "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
    asset_name: "yt-dlp.exe",
    checksums_asset: Some("SHA2-256SUMS"),
    is_archive: false,
};

/// ffmpeg release — BtbN's win64-gpl static builds on GitHub Releases.
/// Picked over gyan.dev because GitHub's CDN is consistently fast
/// worldwide while gyan.dev hosts from a single origin and crawls for
/// users outside its region. The zip contains
/// `ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe`; our zip extractor
/// finds it by filename so the top-folder prefix doesn't matter.
/// This is also the source yt-dlp itself recommends.
const FFMPEG_RELEASE: ToolRelease = ToolRelease {
    version: "latest win64-gpl (BtbN)",
    api_url: "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest",
    asset_name: "ffmpeg-master-latest-win64-gpl.zip",
    checksums_asset: None,
    is_archive: true,
};

/// Compute the managed directory: `<LOCALAPPDATA>/unduhin/binaries/` on
/// Windows, falling back to `~/.local/share/unduhin/binaries/`.
pub fn managed_dir() -> Option<PathBuf> {
    crate::directories_root().map(|d| d.join("binaries"))
}

/// Resolve the binary path for `tool`, preferring (1) the user-set path
/// setting, (2) the managed directory, (3) the system `PATH`.
pub async fn resolve_path(tool: Tool, pool: &SqlitePool) -> Option<PathBuf> {
    let override_path = settings::get(pool, tool.settings_key())
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty());
    if let Some(p) = override_path {
        if tokio::fs::metadata(&p).await.is_ok() {
            return Some(p);
        }
    }

    if let Some(dir) = managed_dir() {
        let candidate = dir.join(tool.binary_name());
        if tokio::fs::metadata(&candidate).await.is_ok() {
            return Some(candidate);
        }
    }

    which(tool.binary_name()).await
}

/// Read the tool's installation status — whether it can be found at all,
/// and its self-reported version if it ran cleanly.
pub async fn status(tool: Tool, pool: &SqlitePool) -> ToolStatus {
    let path = resolve_path(tool, pool).await;
    let version = match path.as_deref() {
        Some(p) => probe_version(tool, p).await,
        None => None,
    };
    ToolStatus {
        tool,
        installed: path.is_some(),
        path,
        version,
        latest_known: Some(tool.release().version.to_string()),
    }
}

/// Fetch the pinned release and write it into the managed directory.
/// Reports progress on the supplied broadcast channel.
pub async fn install_or_update(
    tool: Tool,
    pool: &SqlitePool,
    events: broadcast::Sender<CoreEvent>,
    cancel: CancellationToken,
) -> Result<ToolStatus, ToolingError> {
    let dir = managed_dir().ok_or(ToolingError::NoInstallDir)?;
    tokio::fs::create_dir_all(&dir).await?;

    let result = install_inner(tool, &dir, &events, cancel).await;
    match &result {
        Ok(_) => {
            let status = status(tool, pool).await;
            let _ = events.send(CoreEvent::ToolInstallCompleted {
                tool,
                version: status.version.clone(),
            });
            Ok(status)
        }
        Err(e) => {
            let _ = events.send(CoreEvent::ToolInstallFailed {
                tool,
                error: e.to_string(),
            });
            result.map(|_| unreachable!())
        }
    }
}

async fn install_inner(
    tool: Tool,
    dir: &Path,
    events: &broadcast::Sender<CoreEvent>,
    cancel: CancellationToken,
) -> Result<(), ToolingError> {
    let release = tool.release();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .user_agent(concat!("unduhin/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| ToolingError::Download(e.to_string()))?;

    // Resolve the asset through the GitHub API: this gives an immutable,
    // version-pinned download URL *and* the SHA-256 the bytes must match.
    let resolved = resolve_asset(&client, release).await?;

    let resp = client
        .get(&resolved.download_url)
        .send()
        .await
        .map_err(|e| ToolingError::Download(e.to_string()))?
        .error_for_status()
        .map_err(|e| ToolingError::Download(e.to_string()))?;
    let total = resp.content_length();

    let _ = events.send(CoreEvent::ToolInstallProgress {
        tool,
        downloaded: 0,
        total,
    });

    let tmp = dir.join(format!(".{}-download.tmp", tool.binary_name()));
    let mut out = tokio::fs::File::create(&tmp).await?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    // Hash while streaming so we don't re-read the file to verify it.
    let mut hasher = Sha256::new();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            drop(out);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(ToolingError::Cancelled);
        }
        let bytes = chunk.map_err(|e| ToolingError::Download(e.to_string()))?;
        hasher.update(&bytes);
        out.write_all(&bytes).await?;
        downloaded = downloaded.saturating_add(bytes.len() as u64);
        let _ = events.send(CoreEvent::ToolInstallProgress {
            tool,
            downloaded,
            total,
        });
    }
    out.flush().await?;
    drop(out);

    // Integrity gate: the downloaded bytes must match the digest GitHub
    // reports for this asset before we extract/rename or ever execute it.
    let actual = to_hex(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(&resolved.sha256_hex) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(ToolingError::IntegrityFailed {
            expected: resolved.sha256_hex,
            actual,
        });
    }

    let target = dir.join(tool.binary_name());

    if release.is_archive {
        let extracted = extract_binary_from_zip(&tmp, tool.binary_name())?;
        tokio::fs::write(&target, extracted).await?;
        tokio::fs::remove_file(&tmp).await.ok();
    } else {
        // Atomic-ish swap: overwrite the target.
        if tokio::fs::metadata(&target).await.is_ok() {
            tokio::fs::remove_file(&target).await.ok();
        }
        tokio::fs::rename(&tmp, &target).await?;
    }

    // Verify by running --version (or -version for ffmpeg).
    if probe_version(tool, &target).await.is_none() {
        return Err(ToolingError::VerifyFailed);
    }
    Ok(())
}

/// An asset resolved from the GitHub API: where to download it and the
/// SHA-256 (lowercase hex) its bytes must match.
struct ResolvedAsset {
    download_url: String,
    sha256_hex: String,
}

#[derive(Deserialize)]
struct GhRelease {
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    /// Per-asset digest GitHub computes, formatted `sha256:<hex>`. Absent
    /// on releases uploaded before the feature existed.
    #[serde(default)]
    digest: Option<String>,
}

/// Resolve a release asset through the GitHub REST API, returning its
/// immutable download URL and expected SHA-256. Prefers the per-asset
/// `digest` field; falls back to a companion `SHA2-256SUMS` asset when the
/// release predates digests. Fails closed (`NoChecksum`) if neither is
/// available — we never install a binary we cannot verify.
async fn resolve_asset(
    client: &reqwest::Client,
    release: &ToolRelease,
) -> Result<ResolvedAsset, ToolingError> {
    let body = client
        .get(release.api_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| ToolingError::Download(e.to_string()))?
        .error_for_status()
        .map_err(|e| ToolingError::Download(e.to_string()))?
        .text()
        .await
        .map_err(|e| ToolingError::Download(e.to_string()))?;
    // reqwest is built without the `json` feature, so parse with serde_json.
    let rel: GhRelease = serde_json::from_str(&body)
        .map_err(|e| ToolingError::Download(format!("parsing GitHub release: {e}")))?;

    let asset = rel
        .assets
        .iter()
        .find(|a| a.name == release.asset_name)
        .ok_or_else(|| {
            ToolingError::Download(format!(
                "asset {} not found in latest release",
                release.asset_name
            ))
        })?;

    // Primary: the digest GitHub reports for this asset.
    if let Some(hex) = asset.digest.as_deref().and_then(parse_digest) {
        return Ok(ResolvedAsset {
            download_url: asset.browser_download_url.clone(),
            sha256_hex: hex,
        });
    }

    // Fallback: a SHA2-256SUMS companion asset (yt-dlp).
    if let Some(sums_name) = release.checksums_asset {
        if let Some(sums_asset) = rel.assets.iter().find(|a| a.name == sums_name) {
            let body = client
                .get(&sums_asset.browser_download_url)
                .send()
                .await
                .map_err(|e| ToolingError::Download(e.to_string()))?
                .error_for_status()
                .map_err(|e| ToolingError::Download(e.to_string()))?
                .text()
                .await
                .map_err(|e| ToolingError::Download(e.to_string()))?;
            if let Some(hex) = parse_sha256sums(&body, release.asset_name) {
                return Ok(ResolvedAsset {
                    download_url: asset.browser_download_url.clone(),
                    sha256_hex: hex,
                });
            }
        }
    }

    Err(ToolingError::NoChecksum(format!(
        "no SHA-256 published for {}",
        release.asset_name
    )))
}

/// Parse a GitHub asset `digest` field (`sha256:<hex>`) into the hex
/// portion. Returns `None` for any non-sha256 algorithm.
fn parse_digest(digest: &str) -> Option<String> {
    digest
        .strip_prefix("sha256:")
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|h| h.to_ascii_lowercase())
}

/// Find the SHA-256 for `asset_name` in a `SHA2-256SUMS`-style file
/// (`<hex>  <filename>` per line; the filename may be path-prefixed).
fn parse_sha256sums(body: &str, asset_name: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let name = parts.next()?;
        let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
        if leaf == asset_name && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(hex.to_ascii_lowercase())
        } else {
            None
        }
    })
}

/// Lowercase-hex encode a byte slice (digest output).
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Search a zip archive for the first entry whose path ends with
/// `binary_name` and return its decompressed bytes.
fn extract_binary_from_zip(archive: &Path, binary_name: &str) -> Result<Vec<u8>, ToolingError> {
    use std::io::Read;
    let f = std::fs::File::open(archive).map_err(ToolingError::Io)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| ToolingError::Archive(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| ToolingError::Archive(e.to_string()))?;
        let name = entry.name().to_string();
        let leaf = name.rsplit(['/', '\\']).next().unwrap_or("");
        if leaf.eq_ignore_ascii_case(binary_name) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| ToolingError::Archive(e.to_string()))?;
            return Ok(buf);
        }
    }
    Err(ToolingError::Archive(format!(
        "no entry matching {binary_name} found in archive"
    )))
}

/// Read the tool's self-reported version. Returns `None` if the binary
/// can't be executed at all (`NotInstalled`-equivalent).
async fn probe_version(tool: Tool, path: &Path) -> Option<String> {
    let arg = match tool {
        Tool::YtDlp => "--version",
        Tool::Ffmpeg => "-version",
    };
    let mut cmd = Command::new(path);
    cmd.arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    match tool {
        Tool::YtDlp => Some(text.trim().to_string()).filter(|s| !s.is_empty()),
        Tool::Ffmpeg => parse_ffmpeg_version(&text),
    }
}

/// Pull a version string out of `ffmpeg -version`'s first line. The line
/// looks like `ffmpeg version N-119240-g0ed... Copyright …` — keep just
/// the token after `version`.
fn parse_ffmpeg_version(out: &str) -> Option<String> {
    let first = out.lines().next()?.trim();
    let rest = first.strip_prefix("ffmpeg version ")?;
    rest.split_whitespace().next().map(|s| s.to_string())
}

/// Walk `PATH` looking for `binary_name`. Returns the first match. Async
/// only because the callers are async — this is pure metadata reads.
async fn which(binary_name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary_name);
        if tokio::fs::metadata(&candidate).await.is_ok() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_names_have_correct_extension() {
        let yt = Tool::YtDlp.binary_name();
        let ff = Tool::Ffmpeg.binary_name();
        if cfg!(target_os = "windows") {
            assert_eq!(yt, "yt-dlp.exe");
            assert_eq!(ff, "ffmpeg.exe");
        } else {
            assert_eq!(yt, "yt-dlp");
            assert_eq!(ff, "ffmpeg");
        }
    }

    #[test]
    fn parse_digest_accepts_sha256_only() {
        let hex = "a".repeat(64);
        assert_eq!(parse_digest(&format!("sha256:{hex}")).as_deref(), Some(hex.as_str()));
        assert_eq!(parse_digest(&format!("SHA256:{hex}")), None); // case-sensitive prefix
        assert_eq!(parse_digest("sha512:deadbeef"), None);
        assert_eq!(parse_digest("sha256:tooshort"), None);
    }

    #[test]
    fn parse_sha256sums_finds_matching_asset() {
        let hex = "b".repeat(64);
        let other = "c".repeat(64);
        let body = format!("{other}  some-other-file.zip\n{hex}  yt-dlp.exe\n");
        assert_eq!(
            parse_sha256sums(&body, "yt-dlp.exe").as_deref(),
            Some(hex.as_str())
        );
        assert_eq!(parse_sha256sums(&body, "missing.exe"), None);
    }

    #[test]
    fn to_hex_matches_known_digest() {
        // SHA-256 of the empty input.
        let digest = Sha256::digest([]);
        assert_eq!(
            to_hex(&digest),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn ffmpeg_version_parser_handles_typical_output() {
        let sample = "ffmpeg version 7.0.1-essentials_build-www.gyan.dev Copyright (c) 2000-2024\n\
                      built with gcc 14.1.0 (Rev1, Built by MSYS2 project)";
        assert_eq!(
            parse_ffmpeg_version(sample).as_deref(),
            Some("7.0.1-essentials_build-www.gyan.dev")
        );
    }

    #[test]
    fn ffmpeg_version_parser_returns_none_on_garbage() {
        assert!(parse_ffmpeg_version("").is_none());
        assert!(parse_ffmpeg_version("hello world").is_none());
    }

    #[test]
    fn tool_serializes_as_snake_case() {
        let yt = serde_json::to_string(&Tool::YtDlp).unwrap();
        let ff = serde_json::to_string(&Tool::Ffmpeg).unwrap();
        assert_eq!(yt, "\"yt_dlp\"");
        assert_eq!(ff, "\"ffmpeg\"");
    }
}
