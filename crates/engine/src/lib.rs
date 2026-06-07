//! # Unduhin download engine
//!
//! Multi-segment HTTP downloader with resume support. This crate intentionally
//! depends on nothing from the UI or Tauri layers — its public surface is the
//! API that the `core` queue/persistence crate wraps, and that the
//! CLI binary in this workspace already drives end-to-end.
//!
//! ## Quick tour
//!
//! - [`probe`] — issue a HEAD (or fallback ranged GET) to discover total
//!   length, validators (ETag / Last-Modified), and whether ranges are
//!   supported.
//! - [`download`] — start a brand-new transfer. Writes a `.unduhin-meta`
//!   sidecar next to the output file so the transfer can be picked up
//!   later by [`resume_at`].
//! - [`resume_at`] — continue a previously-interrupted transfer. Validates
//!   the remote file's ETag / Last-Modified / Content-Length before
//!   continuing; if they changed, returns [`EngineError::RemoteChanged`]
//!   so the caller can decide to restart from zero.
//! - [`progress::ProgressEvent`] — events broadcast on a
//!   `tokio::sync::broadcast` channel; the CLI uses these to drive a
//!   progress bar, the future UI will use the same events.
//! - [`tokio_util::sync::CancellationToken`] — re-exported as
//!   [`CancellationToken`] for cancellation. Calling `cancel()` flushes
//!   the sidecar so [`resume_at`] still works.
//!
//! ## Example
//!
//! ```no_run
//! use std::path::PathBuf;
//! use engine::{download, DownloadOptions, CancellationToken, ProgressEvent};
//! use url::Url;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let opts = DownloadOptions::new(
//!     Url::parse("https://example.com/file.bin")?,
//!     PathBuf::from("file.bin"),
//! );
//! let cancel = CancellationToken::new();
//! let summary = download(opts, cancel, None).await?;
//! println!("wrote {} bytes to {}", summary.bytes, summary.output.display());
//! # Ok(())
//! # }
//! ```

pub mod download;
pub mod error;
pub mod filename;
pub mod http;
pub mod meta;
pub mod progress;
pub mod retry;
pub mod segment;
pub mod throttle;
pub mod transfer;

pub use download::{
    download, download_with_control, resume_at, resume_at_with_control, DownloadOptions,
    DownloadSummary,
};
pub use error::{EngineError, Result};
pub use filename::{derive_filename, from_url as filename_from_url};
pub use http::{probe, RemoteInfo};
pub use meta::{Meta, SegmentState, META_SUFFIX};
pub use progress::{ProgressEvent, SegmentRuntimeState, DEFAULT_CHANNEL_CAPACITY};
pub use retry::{Backoff, RetryClass};
pub use segment::Segment;
pub use throttle::TokenBucket;
pub use transfer::{Control, MAX_SEGMENTS, MIN_SEGMENTS};

pub use tokio_util::sync::CancellationToken;
