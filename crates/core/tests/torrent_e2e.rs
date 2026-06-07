//! Phase 2b end-to-end: a magnet runs to a completed file on disk through the
//! REAL queue worker (`run_torrent` → the shared `TorrentEngine` facade →
//! librqbit), not the facade in isolation. This is the P2b verification gate
//! (plan §6): "Magnet → completed file(s) on disk through the real worker."
//!
//! NETWORK-GATED + HEAVY. It needs DHT/peers to resolve a magnet and downloads
//! a real torrent, so it is `#[ignore]`d and only runs when opted in:
//!
//! ```text
//! set UNDUHIN_TORRENT_NET_TEST=1
//! set UNDUHIN_TORRENT_FULL_DOWNLOAD=1
//! cargo test -p unduhin-core --test torrent_e2e -- --ignored --nocapture
//! ```
//!
//! The BUILD of this file is the hard gate (it exercises the real
//! `Core::add_download` → worker path with a torrent row); a live download is a
//! bonus.

use std::time::Duration;

use anyhow::Result;
use unduhin_core::{
    AddDownload, Core, DownloadKind, DownloadSource, Status, TorrentMeta, TorrentSource,
};

/// Magnet for the live download. Defaults to the current, well-seeded Arch
/// Linux ISO — the authoritative magnet from <https://archlinux.org/download/>,
/// which is DHT-seeded (no trackers). This default ROTS: Arch ships a new ISO
/// monthly and old swarms thin out, so refresh it from that page, or override
/// at runtime with `UNDUHIN_TORRENT_TEST_MAGNET=<magnet>` (a distro magnet that
/// also lists HTTP/UDP trackers exercises the tracker path, not just DHT).
const DEFAULT_TEST_MAGNET: &str =
    "magnet:?xt=urn:btih:777695049623a1cd052bd6b175b40e6540ce74ca&dn=archlinux-2026.06.01-x86_64.iso";

fn test_magnet() -> String {
    std::env::var("UNDUHIN_TORRENT_TEST_MAGNET").unwrap_or_else(|_| DEFAULT_TEST_MAGNET.to_string())
}

fn net_enabled() -> bool {
    std::env::var("UNDUHIN_TORRENT_NET_TEST").is_ok()
        && std::env::var("UNDUHIN_TORRENT_FULL_DOWNLOAD").is_ok()
}

async fn wait_for_status(core: &Core, id: i64, target: Status, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let rec = core.get_download(id).await?;
        if rec.status == target {
            return Ok(());
        }
        if rec.status == Status::Failed {
            anyhow::bail!("download failed: {:?}", rec.error);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let rec = core.get_download(id).await?;
    anyhow::bail!(
        "timed out waiting for {target:?}; current = {:?} ({} bytes)",
        rec.status,
        rec.downloaded_bytes
    );
}

#[tokio::test]
#[ignore = "network-gated + heavy: set UNDUHIN_TORRENT_NET_TEST=1 and UNDUHIN_TORRENT_FULL_DOWNLOAD=1"]
async fn magnet_completes_through_worker() -> Result<()> {
    if !net_enabled() {
        eprintln!(
            "skipping: set UNDUHIN_TORRENT_NET_TEST=1 and UNDUHIN_TORRENT_FULL_DOWNLOAD=1 to run"
        );
        return Ok(());
    }

    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("test.db");
    let content = dir.path().join("torrent-content");
    tokio::fs::create_dir_all(&content).await?;

    let core = Core::open(&db_path).await?;
    // Point the torrent download dir at our temp folder so the row's content
    // root lands somewhere we can assert on.
    core.set_setting(
        "torrent_download_dir",
        unduhin_core::SettingValue::from_string(content.to_string_lossy().to_string()),
    )
    .await?;
    core.start().await?;

    let magnet = test_magnet();
    let meta = TorrentMeta {
        info_hash: String::new(), // derived from the magnet xt= at insert time
        source: TorrentSource::Magnet {
            uri: magnet.clone(),
        },
        selected_files: None,
        files: None,
        swarm: None,
    };
    let id = core
        .add_download(AddDownload {
            url: magnet.parse()?,
            filename: None,
            output_path: Some(content.join("debian")),
            category: None,
            priority: 0,
            segments: None,
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
            kind: DownloadKind::Torrent,
            torrent: Some(meta),
        })
        .await?;

    // Generous: metadata resolution + a multi-hundred-MiB download.
    wait_for_status(&core, id, Status::Completed, Duration::from_secs(900)).await?;

    let rec = core.get_download(id).await?;
    assert!(rec.downloaded_bytes > 0, "expected bytes downloaded");
    assert!(
        rec.output_path.exists(),
        "expected content root on disk at {}",
        rec.output_path.display()
    );

    eprintln!(
        "Worker download OK — root={} bytes={}",
        rec.output_path.display(),
        rec.downloaded_bytes
    );

    core.shutdown().await?;
    Ok(())
}
