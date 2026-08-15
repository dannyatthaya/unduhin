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
    /// Where the bytes come from and how their SHA-256 is established.
    source: ToolSource,
    /// Set when the downloaded asset is a zip archive containing the
    /// binary at the path inside it that ends with `binary_name()`.
    is_archive: bool,
}

/// How a tool's download URL and expected digest are obtained.
///
/// Both variants end at the same place — a [`ResolvedAsset`] carrying a URL
/// and a SHA-256 — so [`install_inner`]'s integrity gate is identical for
/// every tool on every platform. There is deliberately no "trust TLS"
/// variant: a source we cannot verify is a source we do not install from.
enum ToolSource {
    /// Resolve through GitHub's REST API. The digest comes from the
    /// per-asset `digest` field, falling back to a companion checksums
    /// asset. Tracks the latest release, which is what yt-dlp needs.
    GitHubLatest {
        /// GitHub REST endpoint for the latest release of the asset's repo.
        api_url: &'static str,
        /// Exact asset filename to download and verify.
        asset_name: &'static str,
        /// Companion checksums asset to fall back on if the API omits a
        /// per-asset `digest` (yt-dlp publishes `SHA2-256SUMS`).
        checksums_asset: Option<&'static str>,
    },
    /// A fixed URL whose SHA-256 is pinned in this file.
    ///
    /// For hosts that publish no checksums at all. Pinning rather than
    /// tracking "latest" is what preserves the integrity guarantee: an
    /// unverifiable moving target would mean trusting TLS alone, which is
    /// exactly what the digest gate exists to avoid.
    /// Only macOS ffmpeg uses this today, so the variant is dead code on
    /// other targets. Kept unconditional so both arms of `resolve_asset`
    /// compile and stay type-checked everywhere.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    PinnedUrl {
        url: &'static str,
        sha256_hex: &'static str,
    },
}

/// yt-dlp release. Resolved via the GitHub API so the install button always
/// pulls the most recent stable release (a pinned version goes stale within
/// weeks) while still verifying the SHA-256 the API reports for the asset.
/// Platform asset name inside the yt-dlp release. `yt-dlp_macos` is a
/// universal2 PyInstaller bundle, so one asset serves both Mac slices. The
/// installed filename is [`Tool::binary_name`], not this — `install_inner`
/// renames on the way in.
#[cfg(target_os = "windows")]
const YTDLP_ASSET: &str = "yt-dlp.exe";
#[cfg(target_os = "macos")]
const YTDLP_ASSET: &str = "yt-dlp_macos";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const YTDLP_ASSET: &str = "yt-dlp";

const YTDLP_RELEASE: ToolRelease = ToolRelease {
    version: "latest",
    source: ToolSource::GitHubLatest {
        api_url: "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
        asset_name: YTDLP_ASSET,
        checksums_asset: Some("SHA2-256SUMS"),
    },
    is_archive: false,
};

/// ffmpeg release — BtbN's win64-gpl static builds on GitHub Releases.
/// Picked over gyan.dev because GitHub's CDN is consistently fast
/// worldwide while gyan.dev hosts from a single origin and crawls for
/// users outside its region. The zip contains
/// `ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe`; our zip extractor
/// finds it by filename so the top-folder prefix doesn't matter.
/// This is also the source yt-dlp itself recommends.
#[cfg(target_os = "windows")]
const FFMPEG_RELEASE: ToolRelease = ToolRelease {
    version: "latest win64-gpl (BtbN)",
    source: ToolSource::GitHubLatest {
        api_url: "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest",
        asset_name: "ffmpeg-master-latest-win64-gpl.zip",
        checksums_asset: None,
    },
    is_archive: true,
};

/// macOS ffmpeg comes from Martin Riedl's build server, because BtbN
/// publishes no macOS builds.
///
/// That host publishes no checksum files of any kind (`.sha256`, `.md5`,
/// `checksums.txt`, and `SHA256SUMS` all 404), so tracking its
/// `/redirect/latest/` endpoint would mean installing an executable we
/// cannot verify. Instead we pin one dated build per architecture and carry
/// its SHA-256 here, which keeps the same fail-closed guarantee Windows has.
///
/// The staleness cost is low. yt-dlp tracks latest because YouTube's
/// anti-bot changes rot it within weeks; ffmpeg's demux/mux behavior is
/// stable across years, so this pin needs bumping about once or twice a
/// year. To bump: resolve
/// `https://ffmpeg.martin-riedl.de/redirect/latest/macos/<arch>/release/ffmpeg.zip`,
/// note the dated URL it lands on, and record that file's SHA-256.
///
/// Each archive holds exactly one root-level entry named `ffmpeg`, which is
/// what `extract_binary_from_zip` looks for. The builds are thin Mach-O per
/// architecture, so a universal app resolves the right one at compile time:
/// each slice is its own compilation, so `cfg(target_arch)` is the running
/// architecture.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const FFMPEG_RELEASE: ToolRelease = ToolRelease {
    version: "9.0 arm64 (martin-riedl)",
    source: ToolSource::PinnedUrl {
        url: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1785863997_9.0/ffmpeg.zip",
        sha256_hex: "5267ef149ee0d208057a1b316aac079b661b0476574dee5da7d225769773c603",
    },
    is_archive: true,
};

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const FFMPEG_RELEASE: ToolRelease = ToolRelease {
    version: "9.0 x86_64 (martin-riedl)",
    source: ToolSource::PinnedUrl {
        url: "https://ffmpeg.martin-riedl.de/download/macos/amd64/1785871427_9.0/ffmpeg.zip",
        sha256_hex: "79d14663d8b078dbbc38de18d63a30f8a5bfc860af5dfee7f8cf3e387cf1c02c",
    },
    is_archive: true,
};

/// Unduhin ships on Windows and macOS only. This arm exists so the crate
/// still compiles on a contributor's Linux box; the tarball BtbN publishes
/// for Linux is `.tar.xz`, which the zip extractor cannot open, so a Linux
/// install will fail loudly rather than silently doing the wrong thing.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FFMPEG_RELEASE: ToolRelease = ToolRelease {
    version: "unsupported platform",
    source: ToolSource::GitHubLatest {
        api_url: "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest",
        asset_name: "ffmpeg-master-latest-linux64-gpl.tar.xz",
        checksums_asset: None,
    },
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

    // Resolve the asset: an immutable, version-pinned download URL *and*
    // the SHA-256 the bytes must match, whatever the source.
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

    // Integrity gate: the downloaded bytes must match the expected digest
    // before we extract/rename or ever execute them.
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

    // Make it executable. Neither install path does this for us: the
    // archive path writes extracted bytes with `fs::write` (0644) and the
    // direct path inherits 0644 from `File::create`. Zips built on Windows
    // carry no unix mode at all, so set the bit unconditionally rather than
    // trusting `ZipFile::unix_mode`. Without this every macOS install fails
    // the `probe_version` gate below with a permission error.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&target).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&target, perms).await?;
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

/// Resolve a release asset to a download URL and the SHA-256 its bytes must
/// match.
///
/// For a [`ToolSource::PinnedUrl`] both are already known. For a
/// [`ToolSource::GitHubLatest`] this queries the REST API, preferring the
/// per-asset `digest` field and falling back to a companion `SHA2-256SUMS`
/// asset when the release predates digests. Fails closed (`NoChecksum`) if
/// neither is available — we never install a binary we cannot verify.
async fn resolve_asset(
    client: &reqwest::Client,
    release: &ToolRelease,
) -> Result<ResolvedAsset, ToolingError> {
    let (api_url, asset_name, checksums_asset) = match release.source {
        ToolSource::PinnedUrl { url, sha256_hex } => {
            return Ok(ResolvedAsset {
                download_url: url.to_string(),
                sha256_hex: sha256_hex.to_string(),
            })
        }
        ToolSource::GitHubLatest {
            api_url,
            asset_name,
            checksums_asset,
        } => (api_url, asset_name, checksums_asset),
    };

    let body = client
        .get(api_url)
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
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            ToolingError::Download(format!("asset {asset_name} not found in latest release"))
        })?;

    // Primary: the digest GitHub reports for this asset.
    if let Some(hex) = asset.digest.as_deref().and_then(parse_digest) {
        return Ok(ResolvedAsset {
            download_url: asset.browser_download_url.clone(),
            sha256_hex: hex,
        });
    }

    // Fallback: a SHA2-256SUMS companion asset (yt-dlp).
    if let Some(sums_name) = checksums_asset {
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
            if let Some(hex) = parse_sha256sums(&body, asset_name) {
                return Ok(ResolvedAsset {
                    download_url: asset.browser_download_url.clone(),
                    sha256_hex: hex,
                });
            }
        }
    }

    Err(ToolingError::NoChecksum(format!(
        "no SHA-256 published for {asset_name}"
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
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
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
        assert_eq!(
            parse_digest(&format!("sha256:{hex}")).as_deref(),
            Some(hex.as_str())
        );
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
