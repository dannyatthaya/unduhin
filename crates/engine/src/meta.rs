//! The `<filename>.unduhin-meta` sidecar file.
//!
//! Holds enough state to resume a partially-completed download in a new
//! process. JSON for now — small, debuggable, and forward-compatible.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::{EngineError, Result};
use crate::segment::Segment;

/// Suffix appended to the output filename to form the sidecar path.
pub const META_SUFFIX: &str = ".unduhin-meta";

/// File-format version. Bump when making breaking changes; old sidecars
/// with a different `version` are rejected and the download starts over.
pub const META_VERSION: u32 = 1;

/// Persisted state of one segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentState {
    pub segment: Segment,
    /// Bytes already written for this segment, counted from `segment.start`.
    pub bytes_downloaded: u64,
}

impl SegmentState {
    pub fn is_complete(&self) -> bool {
        self.bytes_downloaded >= self.segment.len()
    }

    pub fn remaining(&self) -> u64 {
        self.segment.len().saturating_sub(self.bytes_downloaded)
    }

    /// Where the next byte should land in the output file (absolute offset).
    pub fn write_offset(&self) -> u64 {
        self.segment.start + self.bytes_downloaded
    }
}

/// Persisted sidecar contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    pub url: String,
    pub output_path: PathBuf,
    pub total_bytes: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub accept_ranges: bool,
    pub segments: Vec<SegmentState>,
}

impl Meta {
    pub fn new(
        url: impl Into<String>,
        output_path: impl Into<PathBuf>,
        total_bytes: Option<u64>,
        etag: Option<String>,
        last_modified: Option<String>,
        accept_ranges: bool,
        segments: Vec<Segment>,
    ) -> Self {
        Self {
            version: META_VERSION,
            url: url.into(),
            output_path: output_path.into(),
            total_bytes,
            etag,
            last_modified,
            accept_ranges,
            segments: segments
                .into_iter()
                .map(|s| SegmentState {
                    segment: s,
                    bytes_downloaded: 0,
                })
                .collect(),
        }
    }

    /// Total bytes downloaded across all segments.
    pub fn downloaded_total(&self) -> u64 {
        self.segments.iter().map(|s| s.bytes_downloaded).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.segments.iter().all(|s| s.is_complete())
    }

    /// Path the sidecar should live at, given an output path.
    pub fn sidecar_path(output: &Path) -> PathBuf {
        let mut s = output.as_os_str().to_owned();
        s.push(META_SUFFIX);
        PathBuf::from(s)
    }

    pub async fn load(path: &Path) -> Result<Self> {
        let data = fs::read(path)
            .await
            .map_err(|e| EngineError::io(Some(path.to_path_buf()), e))?;
        let meta: Meta = serde_json::from_slice(&data)
            .map_err(|e| EngineError::meta(format!("parse {}: {e}", path.display())))?;
        if meta.version != META_VERSION {
            return Err(EngineError::meta(format!(
                "unsupported sidecar version {} (expected {})",
                meta.version, META_VERSION
            )));
        }
        Ok(meta)
    }

    /// Atomic save: write to `<path>.tmp` then rename. Safe under concurrent
    /// callers because we use a per-meta `.tmp` filename rather than a
    /// process-shared one.
    pub async fn save(&self, path: &Path) -> Result<()> {
        let tmp = {
            let mut s = path.as_os_str().to_owned();
            s.push(".tmp");
            PathBuf::from(s)
        };
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| EngineError::meta(format!("serialize: {e}")))?;
        {
            let mut f = fs::File::create(&tmp)
                .await
                .map_err(|e| EngineError::io(Some(tmp.clone()), e))?;
            f.write_all(&bytes)
                .await
                .map_err(|e| EngineError::io(Some(tmp.clone()), e))?;
            f.flush()
                .await
                .map_err(|e| EngineError::io(Some(tmp.clone()), e))?;
        }
        fs::rename(&tmp, path)
            .await
            .map_err(|e| EngineError::io(Some(path.to_path_buf()), e))?;
        Ok(())
    }

    pub async fn delete(path: &Path) -> Result<()> {
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(EngineError::io(Some(path.to_path_buf()), e)),
        }
    }

    /// Returns true if `remote` matches the validators stored in this meta.
    /// A missing validator on either side is treated as "no information,"
    /// not a mismatch — most range-supporting servers send at least one of
    /// ETag or Last-Modified, but not always both.
    pub fn matches_remote(
        &self,
        etag: Option<&str>,
        last_modified: Option<&str>,
        total_bytes: Option<u64>,
    ) -> bool {
        if let (Some(a), Some(b)) = (self.etag.as_deref(), etag) {
            if a != b {
                return false;
            }
        }
        if let (Some(a), Some(b)) = (self.last_modified.as_deref(), last_modified) {
            if a != b {
                return false;
            }
        }
        if let (Some(a), Some(b)) = (self.total_bytes, total_bytes) {
            if a != b {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Meta {
        Meta::new(
            "https://example.com/file.bin",
            PathBuf::from("/tmp/file.bin"),
            Some(1000),
            Some("\"abc123\"".into()),
            Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
            true,
            vec![
                Segment {
                    index: 0,
                    start: 0,
                    end: 500,
                },
                Segment {
                    index: 1,
                    start: 500,
                    end: 1000,
                },
            ],
        )
    }

    #[test]
    fn round_trip_through_json() {
        let mut m = sample();
        m.segments[0].bytes_downloaded = 250;
        let json = serde_json::to_string(&m).unwrap();
        let back: Meta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, META_VERSION);
        assert_eq!(back.url, m.url);
        assert_eq!(back.segments.len(), 2);
        assert_eq!(back.segments[0].bytes_downloaded, 250);
        assert_eq!(back.downloaded_total(), 250);
    }

    #[test]
    fn complete_detection() {
        let mut m = sample();
        assert!(!m.is_complete());
        m.segments[0].bytes_downloaded = 500;
        m.segments[1].bytes_downloaded = 500;
        assert!(m.is_complete());
    }

    #[test]
    fn matches_remote_strict_when_known() {
        let m = sample();
        assert!(m.matches_remote(
            Some("\"abc123\""),
            Some("Wed, 21 Oct 2026 07:28:00 GMT"),
            Some(1000)
        ));
        assert!(!m.matches_remote(Some("\"other\""), None, None));
        assert!(!m.matches_remote(None, None, Some(1001)));
    }

    #[test]
    fn matches_remote_lenient_when_missing() {
        let m = sample();
        // Server forgot to send ETag this time but Last-Modified still matches.
        assert!(m.matches_remote(None, Some("Wed, 21 Oct 2026 07:28:00 GMT"), Some(1000)));
        // Both missing — can't disprove, so allow.
        assert!(m.matches_remote(None, None, None));
    }

    #[test]
    fn sidecar_path_appends_suffix() {
        let p = Path::new("/downloads/movie.mp4");
        assert_eq!(
            Meta::sidecar_path(p),
            PathBuf::from("/downloads/movie.mp4.unduhin-meta")
        );
    }

    #[tokio::test]
    async fn save_and_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.bin.unduhin-meta");
        let m = sample();
        m.save(&path).await.unwrap();
        let back = Meta::load(&path).await.unwrap();
        assert_eq!(back.url, m.url);
        assert_eq!(back.segments.len(), m.segments.len());
    }
}
