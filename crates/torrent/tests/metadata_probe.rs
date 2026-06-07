//! Phase 2a facade integration tests — live metadata probe + full download.
//!
//! Both exercise the public `TorrentEngine` facade (NOT raw librqbit), proving
//! the abstraction in design §3.D works end to end:
//!   - `metadata_probe_list_only` — `fetch_metadata` (`list_only`) returns the
//!     file list without downloading, bounded by a timeout (design §5.5).
//!   - `full_download_completes` — `run` downloads a small well-seeded torrent to
//!     completion, emits a `Completed` `ProgressEvent`, and stops seeding (Q1).
//!
//! These tests are NETWORK-GATED. They need DHT/peers to resolve a magnet, so
//! they are `#[ignore]`d by default and only run when opted in:
//!
//! ```text
//! set UNDUHIN_TORRENT_NET_TEST=1
//! cargo test -p torrent --test metadata_probe -- --ignored --nocapture
//! ```
//!
//! The BUILD of this file is the hard gate; a live resolve/download is a bonus.

use std::time::Duration;

use tokio::sync::broadcast;
use torrent::{
    TorrentCancellationToken, TorrentConfig, TorrentEngine, TorrentInput, TorrentMetadata,
};

/// Magnet for the live tests. Defaults to the current, well-seeded Arch Linux
/// ISO — the authoritative magnet from <https://archlinux.org/download/>, which
/// is DHT-seeded (no trackers). This default ROTS: Arch ships a new ISO monthly
/// and old swarms thin out, so refresh it from that page, or override at runtime
/// with `UNDUHIN_TORRENT_TEST_MAGNET=<magnet>` (a distro magnet that also lists
/// HTTP/UDP trackers exercises the tracker path, not just DHT).
const DEFAULT_TEST_MAGNET: &str =
    "magnet:?xt=urn:btih:777695049623a1cd052bd6b175b40e6540ce74ca&dn=archlinux-2026.06.01-x86_64.iso";

fn test_magnet() -> String {
    std::env::var("UNDUHIN_TORRENT_TEST_MAGNET").unwrap_or_else(|_| DEFAULT_TEST_MAGNET.to_string())
}

/// Source for the live download test: a `.torrent` file path
/// (`UNDUHIN_TORRENT_TEST_FILE`, preferred — its metadata + trackers are in the
/// file, so no magnet metadata-fetch is needed) if set, else [`test_magnet`].
fn test_input() -> TorrentInput {
    match std::env::var("UNDUHIN_TORRENT_TEST_FILE") {
        Ok(p) if !p.trim().is_empty() => TorrentInput::TorrentFile(std::path::PathBuf::from(p)),
        _ => TorrentInput::Magnet(test_magnet()),
    }
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

fn net_enabled() -> bool {
    std::env::var("UNDUHIN_TORRENT_NET_TEST").is_ok()
}

async fn make_engine(suffix: &str) -> anyhow::Result<TorrentEngine> {
    let base = std::env::temp_dir().join(format!("unduhin-torrent-{suffix}"));
    let cfg = TorrentConfig::new(base.join("content"), base.join("state"));
    Ok(TorrentEngine::new(cfg).await?)
}

#[tokio::test]
#[ignore = "network-gated: requires DHT/peers; opt in with UNDUHIN_TORRENT_NET_TEST=1 and --ignored"]
async fn metadata_probe_list_only() -> anyhow::Result<()> {
    if !net_enabled() {
        eprintln!("skipping: set UNDUHIN_TORRENT_NET_TEST=1 to run the live probe");
        return Ok(());
    }

    // Surface librqbit's own DHT/peer/tracker logs when diagnosing "no peers"
    // failures: run with e.g. RUST_LOG=info,librqbit=debug,librqbit_dht=debug.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,librqbit=debug")),
        )
        .with_test_writer()
        .try_init();

    let engine = make_engine("probe").await?;
    let input = TorrentInput::Magnet(test_magnet());
    let cancel = TorrentCancellationToken::new();

    // Use the facade's own (longer) timeout so a cold DHT bootstrap has room to
    // populate its routing table and find peers before we give up.
    let meta: TorrentMetadata = engine
        .fetch_metadata_with_timeout(&input, cancel, PROBE_TIMEOUT)
        .await?;

    assert!(!meta.info_hash.is_empty(), "expected an infohash");
    assert!(!meta.files.is_empty(), "expected a non-empty file list");
    assert!(meta.total_bytes() > 0, "expected total bytes > 0");

    eprintln!(
        "ListOnly OK — info_hash={} name={} files={} total_bytes={}",
        meta.info_hash,
        meta.name,
        meta.files.len(),
        meta.total_bytes()
    );
    Ok(())
}

/// Full download through the facade. Heavy (downloads the whole torrent), so it
/// is doubly gated: network AND an explicit `UNDUHIN_TORRENT_FULL_DOWNLOAD=1`.
#[tokio::test]
#[ignore = "network-gated + heavy: set UNDUHIN_TORRENT_NET_TEST=1 and UNDUHIN_TORRENT_FULL_DOWNLOAD=1"]
async fn full_download_completes() -> anyhow::Result<()> {
    if !net_enabled() || std::env::var("UNDUHIN_TORRENT_FULL_DOWNLOAD").is_err() {
        eprintln!(
            "skipping: set UNDUHIN_TORRENT_NET_TEST=1 and UNDUHIN_TORRENT_FULL_DOWNLOAD=1 to run"
        );
        return Ok(());
    }

    let engine = make_engine("download").await?;
    let input = TorrentInput::Magnet(test_magnet());
    let cancel = TorrentCancellationToken::new();
    let dest = std::env::temp_dir().join("unduhin-torrent-download-out");
    tokio::fs::create_dir_all(&dest).await?;

    let (tx, mut rx) = broadcast::channel::<engine::ProgressEvent>(1024);

    // Drain progress events on a side task, recording whether we saw Completed.
    let watcher = tokio::spawn(async move {
        let mut completed = false;
        while let Ok(ev) = rx.recv().await {
            if let engine::ProgressEvent::Completed { bytes } = ev {
                eprintln!("Completed event: {bytes} bytes");
                completed = true;
            }
        }
        completed
    });

    let summary = engine
        .run(input, dest.clone(), None, cancel, Some(tx))
        .await?;

    // Closing the sender ends the watcher loop.
    let saw_completed = watcher.await.unwrap_or(false);

    assert!(saw_completed, "expected a Completed ProgressEvent");
    assert!(summary.bytes > 0, "expected bytes downloaded > 0");
    assert!(
        summary.output_root.exists(),
        "expected content root on disk at {}",
        summary.output_root.display()
    );

    eprintln!(
        "Download OK — root={} bytes={} resumed={}",
        summary.output_root.display(),
        summary.bytes,
        summary.resumed
    );
    Ok(())
}

/// Proves the REAL download path (NOT `list_only`) finds peers and pulls bytes
/// via trackers, then cancels — so it never downloads the whole torrent. Unlike
/// the `list_only` probe, `run()` announces the real listen port, so
/// port-validating trackers accept the announce even when DHT is unavailable
/// (e.g. librqbit's DHT framer dies on Windows with WSAECONNRESET / os 10054).
///
/// Point it at a tracker-rich, well-seeded source:
///   - `UNDUHIN_TORRENT_TEST_FILE=<path to .torrent>` — PREFERRED: the metadata
///     and trackers live in the file, so there's no magnet metadata-fetch to
///     stall on. A current Debian/Ubuntu netinst `.torrent` works well (HTTP
///     tracker + webseed), or
///   - `UNDUHIN_TORRENT_TEST_MAGNET=<tracker-rich magnet>`.
/// The default Arch magnet is DHT-only, so it will NOT pass here.
#[tokio::test]
#[ignore = "network-gated: needs peers via trackers; opt in with UNDUHIN_TORRENT_NET_TEST=1 and --ignored"]
async fn download_starts_via_trackers() -> anyhow::Result<()> {
    if !net_enabled() {
        eprintln!("skipping: set UNDUHIN_TORRENT_NET_TEST=1 to run");
        return Ok(());
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,librqbit=info")),
        )
        .with_test_writer()
        .try_init();

    let engine = make_engine("starts").await?;
    let input = test_input();
    let dest = std::env::temp_dir().join("unduhin-torrent-starts-out");
    tokio::fs::create_dir_all(&dest).await?;

    let cancel = TorrentCancellationToken::new();
    let (tx, mut rx) = broadcast::channel::<engine::ProgressEvent>(1024);

    // Cancel as soon as a few MiB have arrived — enough to prove peers connected
    // and data is flowing, without pulling the whole torrent.
    const ENOUGH: u64 = 4 * 1024 * 1024;
    let cancel_watch = cancel.clone();
    let watcher = tokio::spawn(async move {
        let mut max_downloaded = 0u64;
        loop {
            match rx.recv().await {
                Ok(engine::ProgressEvent::Tick { downloaded, .. }) => {
                    max_downloaded = max_downloaded.max(downloaded);
                    if downloaded >= ENOUGH {
                        cancel_watch.cancel();
                        break;
                    }
                }
                Ok(engine::ProgressEvent::Completed { bytes }) => {
                    max_downloaded = max_downloaded.max(bytes);
                    break;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        max_downloaded
    });

    let run_fut = engine.run(input, dest, None, cancel.clone(), Some(tx));
    let run_res = tokio::time::timeout(Duration::from_secs(180), run_fut).await;
    cancel.cancel(); // ensure the session stops if we hit the outer timeout
    let max_downloaded = watcher.await.unwrap_or(0);

    eprintln!("run result: {run_res:?}; max_downloaded={max_downloaded} bytes");
    assert!(
        max_downloaded > 0,
        "no bytes flowed within 180s — peers never delivered data via trackers \
         (use a tracker-rich, well-seeded source: UNDUHIN_TORRENT_TEST_FILE=<.torrent> \
         or UNDUHIN_TORRENT_TEST_MAGNET=<magnet with trackers>)"
    );
    Ok(())
}
