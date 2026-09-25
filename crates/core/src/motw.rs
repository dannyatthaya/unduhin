//! Mark finished downloads as coming from the internet.
//!
//! Browsers tag every download: an NTFS `Zone.Identifier` stream on Windows
//! (the "Mark of the Web"), a `com.apple.quarantine` extended attribute on
//! macOS. That tag is what makes SmartScreen check an executable, Office open
//! a document in Protected View, and Gatekeeper vet an app before it first
//! runs. A download the extension takes over from the browser would otherwise
//! lose it — the browser never writes the file — so the app adds the same tag
//! when a download completes.
//!
//! Best-effort throughout. A volume without alternate data streams or
//! extended attributes (FAT32, exFAT, some network shares) simply cannot hold
//! the tag, and a download that finished must not be reported as failed
//! because of it.

use std::path::{Path, PathBuf};

use crate::download::{DownloadKind, DownloadRecord};

/// Windows zone 3: the internet.
const INTERNET_ZONE: u8 = 3;

/// Tag every file a completed download produced.
pub(crate) async fn mark_download(record: &DownloadRecord) {
    let host_url = match record.kind {
        // The page the media came from, not the CDN URL yt-dlp resolved.
        DownloadKind::Media => record.media_info.as_ref().map(|m| m.original_url.as_str()),
        DownloadKind::Http => Some(record.url.as_str()),
        // A magnet is not a location anything was fetched from.
        DownloadKind::Torrent => None,
    };
    let referrer = record.headers.as_ref().and_then(|hs| {
        hs.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("referer"))
            .map(|(_, v)| v.as_str())
    });
    let tag = Tag {
        host_url: host_url.and_then(scrub_url),
        referrer_url: referrer.and_then(scrub_url),
    };

    for path in files_of(record).await {
        if let Err(e) = tag.apply(&path) {
            tracing::debug!(path = %path.display(), error = %e, "motw: could not tag file");
        }
    }
}

/// Every file the download produced. A torrent's content lives in a folder;
/// only files known to be the torrent's are tagged, never a user's files that
/// happen to share an explicit content folder.
async fn files_of(record: &DownloadRecord) -> Vec<PathBuf> {
    if record.kind != DownloadKind::Torrent {
        return vec![record.output_path.clone()];
    }
    if crate::download::owns_torrent_dir(record) {
        return walk_files(&record.output_path).await;
    }
    record
        .torrent
        .as_ref()
        .and_then(|t| t.files.as_ref())
        .map(|files| {
            files
                .iter()
                .filter_map(|f| crate::download::safe_relative_path(&f.path))
                .map(|rel| record.output_path.join(rel))
                .collect()
        })
        .unwrap_or_default()
}

/// Regular files under `root`, recursively. Symlinks are skipped: a torrent
/// never produces one, and following one could tag a file outside the folder.
async fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            match entry.file_type().await {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => out.push(entry.path()),
                _ => {}
            }
        }
    }
    out
}

/// Keep only what identifies where a file came from. Query strings and
/// fragments routinely carry signed tokens and session ids, and the tag
/// travels with the file (into archives, onto other machines), so they are
/// dropped along with any userinfo. Non-web URLs yield nothing.
fn scrub_url(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Some(url.into())
}

struct Tag {
    host_url: Option<String>,
    referrer_url: Option<String>,
}

impl Tag {
    /// Contents of the `Zone.Identifier` stream, in the layout browsers
    /// write.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn zone_identifier(&self) -> String {
        let mut s = format!("[ZoneTransfer]\r\nZoneId={INTERNET_ZONE}\r\n");
        if let Some(r) = &self.referrer_url {
            s.push_str(&format!("ReferrerUrl={r}\r\n"));
        }
        if let Some(h) = &self.host_url {
            s.push_str(&format!("HostUrl={h}\r\n"));
        }
        s
    }

    #[cfg(windows)]
    fn apply(&self, path: &Path) -> std::io::Result<()> {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        std::fs::write(PathBuf::from(stream), self.zone_identifier())
    }

    #[cfg(target_os = "macos")]
    fn apply(&self, path: &Path) -> std::io::Result<()> {
        use std::os::unix::ffi::OsStrExt;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let value = quarantine_value(now, &event_id(path, now));
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: both strings are NUL-terminated and outlive the call;
        // `value` is valid for `value.len()` bytes.
        let rc = unsafe {
            libc::setxattr(
                c_path.as_ptr(),
                b"com.apple.quarantine\0".as_ptr().cast(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// No platform tag to write.
    #[cfg(not(any(windows, target_os = "macos")))]
    fn apply(&self, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

/// `com.apple.quarantine` value: flags; hex timestamp; agent; event id.
/// `0081` is the flag set browsers use for a download that has not yet been
/// opened (Gatekeeper sets the "approved" bit itself once the user allows
/// it).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn quarantine_value(unix_secs: u64, event_id: &str) -> String {
    format!("0081;{unix_secs:08x};Unduhin;{event_id}")
}

/// UUID-shaped id for the quarantine record. It only has to be unique per
/// event, not unpredictable, so a hash of the path, time and process is
/// enough without pulling in a UUID crate.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn event_id(path: &Path, unix_secs: u64) -> String {
    use sha2::{Digest, Sha256};

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(path.as_os_str().as_encoded_bytes());
    h.update(unix_secs.to_le_bytes());
    h.update(nanos.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let hex: String = h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_url_drops_tokens_and_credentials() {
        assert_eq!(
            scrub_url("https://user:pw@cdn.example.com/f/a.exe?token=secret#frag").as_deref(),
            Some("https://cdn.example.com/f/a.exe")
        );
    }

    #[test]
    fn scrub_url_ignores_non_web_urls() {
        assert_eq!(scrub_url("magnet:?xt=urn:btih:abc"), None);
        assert_eq!(scrub_url("file:///C:/x.exe"), None);
        assert_eq!(scrub_url("not a url"), None);
    }

    #[test]
    fn zone_identifier_matches_the_browser_layout() {
        let tag = Tag {
            host_url: Some("https://cdn.example.com/a.exe".into()),
            referrer_url: Some("https://example.com/page".into()),
        };
        assert_eq!(
            tag.zone_identifier(),
            "[ZoneTransfer]\r\nZoneId=3\r\n\
             ReferrerUrl=https://example.com/page\r\n\
             HostUrl=https://cdn.example.com/a.exe\r\n"
        );
    }

    #[test]
    fn zone_identifier_without_urls_still_marks_the_internet_zone() {
        let tag = Tag {
            host_url: None,
            referrer_url: None,
        };
        assert_eq!(tag.zone_identifier(), "[ZoneTransfer]\r\nZoneId=3\r\n");
    }

    #[test]
    fn quarantine_value_layout() {
        let v = quarantine_value(0x6500_0000, "ID");
        assert_eq!(v, "0081;65000000;Unduhin;ID");
    }

    #[test]
    fn event_id_is_uuid_shaped() {
        let id = event_id(Path::new("/tmp/a"), 1);
        let parts: Vec<usize> = id.split('-').map(str::len).collect();
        assert_eq!(parts, vec![8, 4, 4, 4, 12]);
        assert!(id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    }
}
