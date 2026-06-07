//! Extension → core torrent hand-off (design §3.E, plan §5 P6).
//!
//! The browser extension captures a clicked `magnet:` link or a downloaded
//! `.torrent` file and forwards it over the named pipe as
//! [`wire::Inbound::DownloadTorrent`]. The native side turns that untrusted
//! [`wire::TorrentJob`] into an [`AddDownload`] this module owns the
//! translation so the heavy lifting (decoding / size-limiting / validating
//! untrusted bytes, writing the `.torrent` into the managed dir, guarding
//! path traversal) is unit-testable in `core` without the Tauri shell.
//!
//! The pipe server's `handle_download_torrent` is then a thin caller:
//!
//! ```ignore
//! let input = unduhin_core::torrent_handoff::add_download_from_torrent_job(
//!     job,
//!     unduhin_core::torrent_handoff::incoming_torrents_dir(),
//! )?;
//! let id = core.add_download(input).await?;
//! Outbound::Ack { id }
//! ```
//!
//! ## Untrusted-input hardening
//!
//! Both fields of a [`wire::TorrentJob`] cross a trust boundary (a hostile or
//! buggy page could synthesize either):
//!
//! - **`torrent_file_b64`** is size-limited *before* and *after* base64 decode
//!   ([`MAX_TORRENT_FILE_BYTES`]) so a huge payload can't exhaust memory or
//!   fill the disk, and is written under a filename derived purely from a
//!   content hash (hex) — never from any caller-supplied string — so it cannot
//!   contain a path separator or `..`.
//! - **`magnet`** must carry a v1 `xt=urn:btih:` hash, rejected otherwise.
//! - **`suggested_filename`** is only ever surfaced as the provisional
//!   [`AddDownload::filename`], which [`crate::download`]'s `insert` runs
//!   through `sanitize_filename` (its path-traversal guard) — it never reaches
//!   a filesystem path here.

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::download::{AddDownload, DownloadKind, DownloadSource, TorrentMeta, TorrentSource};
use crate::error::{CoreError, Result};

/// Hard ceiling on a decoded `.torrent` file. Real metainfo files for even
/// enormous multi-file torrents are a few hundred KB (the piece-hash table
/// dominates); 8 MiB is a generous ceiling that still rejects a payload
/// crafted to exhaust memory or the disk. The base64 form is bounded to ~4/3
/// of this before decoding so we never allocate the decoded buffer for an
/// over-limit input.
pub const MAX_TORRENT_FILE_BYTES: usize = 8 * 1024 * 1024;

/// `.torrent` files captured by the extension land here, named by content
/// hash. An `incoming/` subdir of the librqbit state root
/// (`directories_root()/torrents`, see [`crate::download::torrent_state_root`])
/// so the incoming `.torrent` cache never collides with librqbit's flat
/// `<info_hash>.bitv` / `<info_hash>.torrent` state files. `None` when no
/// app-data root is resolvable (mirrors [`crate::directories_root`]).
pub fn incoming_torrents_dir() -> Option<PathBuf> {
    crate::directories_root().map(|d| d.join("torrents").join("incoming"))
}

/// Build an [`AddDownload`] from an extension-captured [`wire::TorrentJob`].
///
/// `managed_dir` is the directory captured `.torrent` bytes are written into
/// (typically [`incoming_torrents_dir`]); it is created if missing. Magnet
/// jobs touch no disk. Returns [`CoreError::InvalidArgument`] for an empty or
/// malformed job, or an oversize / undecodable `.torrent`.
///
/// The returned `AddDownload` carries `kind: Torrent`, `source: ExtensionPipe`,
/// and a populated `torrent: Some(..)`. `Core::add_download` then computes the
/// canonical `info_hash` (for magnets), engages Q7 front-door de-dup, and
/// assigns the provisional name. [`wire::TorrentJob`]: crate::wire::TorrentJob
pub fn add_download_from_torrent_job(
    job: crate::wire::TorrentJob,
    managed_dir: Option<PathBuf>,
) -> Result<AddDownload> {
    let crate::wire::TorrentJob {
        magnet,
        torrent_file_b64,
        page_url: _,
        tab_id: _,
        suggested_filename,
    } = job;

    let magnet = magnet.filter(|s| !s.trim().is_empty());
    let torrent_file_b64 = torrent_file_b64.filter(|s| !s.trim().is_empty());

    // Build the source + the synthetic `url` column value. A magnet wins when
    // both are (unexpectedly) present — it needs no disk write and de-dups for
    // free. `.torrent` bytes are decoded, validated, and written to the managed
    // dir; the row references the on-disk path (design §3.B: store path, not
    // bytes).
    let (source, url) = match (magnet, torrent_file_b64) {
        (Some(uri), _) => {
            validate_magnet(&uri)?;
            let url = parse_torrent_url(&uri);
            (TorrentSource::Magnet { uri }, url)
        }
        (None, Some(b64)) => {
            let bytes = decode_torrent_b64(&b64)?;
            let dir = managed_dir.ok_or_else(|| {
                CoreError::InvalidArgument(
                    "no managed directory available to store the .torrent file".to_string(),
                )
            })?;
            let path = write_managed_torrent(&dir, &bytes)?;
            // No usable info-hash without bencode parsing (core has no
            // bencode/SHA-1); the facade resolves the real v1 hash at
            // add/metadata time. Leave `url` as a content-hash-keyed synthetic
            // so the `url` column stays populated and unique per file.
            let content_hash = sha256_hex(&bytes);
            let url = synthetic_file_url(&content_hash);
            (TorrentSource::File { path }, url)
        }
        (None, None) => {
            return Err(CoreError::InvalidArgument(
                "torrent job carried neither a magnet link nor a .torrent file".to_string(),
            ));
        }
    };

    let meta = TorrentMeta {
        // Left empty for `.torrent`; derived from the magnet's `xt=urn:btih:`
        // by `download::normalize_torrent_meta` inside `add_download`.
        info_hash: String::new(),
        source,
        selected_files: None,
        files: None,
        swarm: None,
    };

    Ok(AddDownload {
        url,
        filename: suggested_filename.filter(|s| !s.trim().is_empty()),
        output_path: None,
        category: None,
        priority: 0,
        segments: None,
        media_info: None,
        headers: None,
        source: DownloadSource::ExtensionPipe,
        kind: DownloadKind::Torrent,
        torrent: Some(meta),
    })
}

/// A magnet must carry a BitTorrent v1 `xt=urn:btih:<hash>` so de-dup and the
/// facade have something to resolve. Accepts the 40-char hex and the 32-char
/// base32 forms (librqbit handles both); rejects anything else.
fn validate_magnet(uri: &str) -> Result<()> {
    let trimmed = uri.trim();
    if !trimmed
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("magnet:"))
    {
        return Err(CoreError::InvalidArgument(format!(
            "not a magnet URI: {trimmed:?}"
        )));
    }
    let query = trimmed.split_once('?').map(|(_, q)| q).unwrap_or("");
    let has_btih = query.split('&').any(|pair| {
        let Some((k, v)) = pair.split_once('=') else {
            return false;
        };
        if !k.eq_ignore_ascii_case("xt") {
            return false;
        }
        let lower = v.to_ascii_lowercase();
        let Some(hash) = lower.strip_prefix("urn:btih:") else {
            return false;
        };
        // 40-hex (v1) or 32-char base32 — both are valid btih encodings.
        (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            || (hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_alphanumeric()))
    });
    if !has_btih {
        return Err(CoreError::InvalidArgument(
            "magnet URI is missing a BitTorrent v1 xt=urn:btih: hash".to_string(),
        ));
    }
    Ok(())
}

/// Decode the base64 `.torrent` payload, size-limiting both encodings so a
/// hostile input can't force a giant allocation. Base64 inflates by ~4/3, so
/// an over-limit decoded size is caught by the pre-check on the encoded length
/// before we ever allocate the decode buffer.
fn decode_torrent_b64(b64: &str) -> Result<Vec<u8>> {
    // base64 of N bytes is ceil(N/3)*4; bound the encoded length so the decode
    // buffer can't exceed the cap. Strip whitespace the browser may have
    // wrapped in (data-URI line breaks).
    let max_encoded = (MAX_TORRENT_FILE_BYTES / 3 + 1) * 4 + 4;
    if b64.len() > max_encoded {
        return Err(CoreError::InvalidArgument(format!(
            "encoded .torrent too large: {} bytes (max {} decoded)",
            b64.len(),
            MAX_TORRENT_FILE_BYTES
        )));
    }
    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|e| CoreError::InvalidArgument(format!("invalid base64 .torrent: {e}")))?;
    if bytes.is_empty() {
        return Err(CoreError::InvalidArgument(
            "decoded .torrent file was empty".to_string(),
        ));
    }
    if bytes.len() > MAX_TORRENT_FILE_BYTES {
        return Err(CoreError::InvalidArgument(format!(
            "decoded .torrent too large: {} bytes (max {})",
            bytes.len(),
            MAX_TORRENT_FILE_BYTES
        )));
    }
    // Cheap sanity: a bencoded `.torrent` is a top-level dict, i.e. starts with
    // 'd'. This is not a full parse (the facade validates fully when librqbit
    // adds it), just a guard against obviously-bogus bytes reaching disk.
    if bytes.first() != Some(&b'd') {
        return Err(CoreError::InvalidArgument(
            "payload is not a bencoded .torrent (expected a top-level dictionary)".to_string(),
        ));
    }
    Ok(bytes)
}

/// Write validated `.torrent` bytes into `dir`, named `<sha256hex>.torrent`.
/// The filename is derived solely from the content hash, so it is path-safe by
/// construction (lowercase hex + a fixed extension — no separators, no `..`).
/// Idempotent: re-writing the same bytes lands the same file.
fn write_managed_torrent(dir: &Path, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| {
        CoreError::InvalidArgument(format!(
            "creating managed torrent dir {}: {e}",
            dir.display()
        ))
    })?;
    let name = format!("{}.torrent", sha256_hex(bytes));
    let path = dir.join(&name);
    std::fs::write(&path, bytes)
        .map_err(|e| CoreError::InvalidArgument(format!("writing {}: {e}", path.display())))?;
    Ok(path)
}

/// Lowercase hex SHA-256 of `bytes`. Used to name the managed `.torrent` file
/// (path-safe) and to key the synthetic `url` for a `.torrent` row.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Parse a magnet URI into a `url::Url` for the row's `url` column. Magnet URIs
/// are valid URLs (they have a `magnet:` scheme), so this normally succeeds;
/// on the rare parse failure fall back to a synthetic placeholder so the
/// non-null `url` column is always populated. The torrent run path reads
/// `record.torrent`, never `record.url`, so this value is informational.
fn parse_torrent_url(magnet: &str) -> url::Url {
    url::Url::parse(magnet.trim()).unwrap_or_else(|_| synthetic_file_url("magnet"))
}

/// Synthetic `urn:`-style URL for the row's non-null `url` column when there is
/// no real fetchable URL (a `.torrent`-file row, or an unparseable magnet).
/// Never fetched — `run_torrent` reads `record.torrent`.
fn synthetic_file_url(key: &str) -> url::Url {
    // `urn:` is a valid absolute-URI scheme `url::Url` accepts; keep the key in
    // the path so distinct `.torrent` files get distinct `url` values.
    url::Url::parse(&format!("urn:unduhin-torrent:{key}"))
        .expect("urn: scheme with hex/ascii key always parses")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::TorrentJob;

    fn magnet_job(uri: &str) -> TorrentJob {
        TorrentJob {
            magnet: Some(uri.to_string()),
            torrent_file_b64: None,
            page_url: None,
            tab_id: None,
            suggested_filename: None,
        }
    }

    /// A minimal-but-bencoded `.torrent`-shaped blob: a top-level dict so the
    /// `decode_torrent_b64` sanity check passes. Not a real metainfo — the
    /// facade does the full parse; this only exercises the hand-off plumbing.
    fn fake_torrent_bytes() -> Vec<u8> {
        b"d8:announce11:udp://x:806:lengthi42ee".to_vec()
    }

    #[test]
    fn magnet_job_builds_torrent_add_download() {
        let uri = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789abcdef01234567&dn=Cool+Thing";
        let input =
            add_download_from_torrent_job(magnet_job(uri), None).expect("magnet job is valid");
        assert_eq!(input.kind, DownloadKind::Torrent);
        assert_eq!(input.source, DownloadSource::ExtensionPipe);
        let meta = input.torrent.expect("torrent meta present");
        match meta.source {
            TorrentSource::Magnet { uri: u } => assert_eq!(u, uri),
            other => panic!("expected Magnet source, got {other:?}"),
        }
        // The url column carries the magnet so normalize_torrent_meta can
        // recover the hash inside add_download.
        assert_eq!(input.url.scheme(), "magnet");
    }

    #[test]
    fn suggested_filename_flows_through_as_provisional_name() {
        let mut job = magnet_job("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567");
        job.suggested_filename = Some("My Linux ISO".into());
        let input = add_download_from_torrent_job(job, None).unwrap();
        assert_eq!(input.filename.as_deref(), Some("My Linux ISO"));
    }

    #[test]
    fn empty_suggested_filename_becomes_none() {
        let mut job = magnet_job("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567");
        job.suggested_filename = Some("   ".into());
        let input = add_download_from_torrent_job(job, None).unwrap();
        assert!(input.filename.is_none());
    }

    #[test]
    fn magnet_without_btih_is_rejected() {
        // A magnet with only a `dn=` and no xt= hash is meaningless to us.
        let err =
            add_download_from_torrent_job(magnet_job("magnet:?dn=NoHash"), None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn non_magnet_uri_is_rejected() {
        let err = add_download_from_torrent_job(
            magnet_job("https://evil.example/not-a-magnet"),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn base32_btih_magnet_is_accepted() {
        // 32-char base32 info-hash form.
        let uri = "magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        add_download_from_torrent_job(magnet_job(uri), None).expect("base32 btih is valid");
    }

    #[test]
    fn empty_job_is_rejected() {
        let job = TorrentJob {
            magnet: None,
            torrent_file_b64: None,
            page_url: Some("https://x".into()),
            tab_id: Some(1),
            suggested_filename: None,
        };
        let err = add_download_from_torrent_job(job, None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn torrent_file_job_writes_managed_file() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = fake_torrent_bytes();
        let b64 = STANDARD.encode(&bytes);
        let job = TorrentJob {
            magnet: None,
            torrent_file_b64: Some(b64),
            page_url: None,
            tab_id: None,
            suggested_filename: Some("thing".into()),
        };
        let input = add_download_from_torrent_job(job, Some(dir.path().to_path_buf()))
            .expect("torrent file job is valid");
        let meta = input.torrent.expect("torrent meta present");
        let written = match meta.source {
            TorrentSource::File { path } => path,
            other => panic!("expected File source, got {other:?}"),
        };
        // The file landed under the managed dir with a content-hash name, and
        // the bytes round-trip.
        assert!(written.starts_with(dir.path()), "{written:?}");
        assert_eq!(written.extension().and_then(|e| e.to_str()), Some("torrent"));
        let on_disk = std::fs::read(&written).unwrap();
        assert_eq!(on_disk, bytes);
        // The filename is pure hex — no path separators could have leaked in.
        let stem = written.file_stem().unwrap().to_str().unwrap();
        assert_eq!(stem.len(), 64);
        assert!(stem.bytes().all(|b| b.is_ascii_hexdigit()));
        // `.torrent` rows get a synthetic urn: url (never fetched).
        assert_eq!(input.url.scheme(), "urn");
    }

    #[test]
    fn torrent_file_b64_missing_managed_dir_errors() {
        let bytes = fake_torrent_bytes();
        let job = TorrentJob {
            magnet: None,
            torrent_file_b64: Some(STANDARD.encode(&bytes)),
            page_url: None,
            tab_id: None,
            suggested_filename: None,
        };
        let err = add_download_from_torrent_job(job, None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn oversize_b64_is_rejected_before_decode() {
        // A base64 string longer than the encoded cap is rejected without
        // allocating the decode buffer.
        let huge = "A".repeat((MAX_TORRENT_FILE_BYTES / 3 + 1) * 4 + 100);
        let job = TorrentJob {
            magnet: None,
            torrent_file_b64: Some(huge),
            page_url: None,
            tab_id: None,
            suggested_filename: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let err =
            add_download_from_torrent_job(job, Some(dir.path().to_path_buf())).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn invalid_base64_is_rejected() {
        let job = TorrentJob {
            magnet: None,
            torrent_file_b64: Some("not valid base64 !!!".into()),
            page_url: None,
            tab_id: None,
            suggested_filename: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let err =
            add_download_from_torrent_job(job, Some(dir.path().to_path_buf())).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn non_bencode_payload_is_rejected() {
        // Decodes fine as base64 but is not a bencoded dict (doesn't start
        // with 'd') — rejected before it reaches disk.
        let job = TorrentJob {
            magnet: None,
            torrent_file_b64: Some(STANDARD.encode(b"i am not a torrent")),
            page_url: None,
            tab_id: None,
            suggested_filename: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let err =
            add_download_from_torrent_job(job, Some(dir.path().to_path_buf())).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn managed_filename_cannot_traverse() {
        // Even if a hostile caller crafted bytes, the on-disk name is a pure
        // SHA-256 hex string + ".torrent", so it can never contain a separator
        // or "..". This asserts the invariant directly.
        let dir = tempfile::tempdir().unwrap();
        let path = write_managed_torrent(dir.path(), b"dee").unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(!name.contains('/') && !name.contains('\\') && !name.contains(".."));
        assert!(path.starts_with(dir.path()));
    }
}
