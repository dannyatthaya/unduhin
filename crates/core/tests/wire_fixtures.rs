//! Round-trip every committed wire-message fixture through serde to lock
//! the JSON shape in place. The extension and the Rust native
//! host both consume this shape — drift here is an instant cross-process
//! protocol break.

use unduhin_core::wire::{Inbound, Outbound};

fn read(name: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wire")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn round_trip_inbound(name: &str) {
    let raw = read(name);
    let parsed: Inbound =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    // Re-serializing then re-parsing must yield the same value — guards
    // against accidental field renames or removed variants.
    let again = serde_json::to_string(&parsed).expect("serialize back");
    let twice: Inbound = serde_json::from_str(&again).expect("re-parse");
    assert_eq!(parsed, twice, "fixture {name}: re-parse mismatch");
}

fn round_trip_outbound(name: &str) {
    let raw = read(name);
    let parsed: Outbound =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    let again = serde_json::to_string(&parsed).expect("serialize back");
    let twice: Outbound = serde_json::from_str(&again).expect("re-parse");
    assert_eq!(parsed, twice, "fixture {name}: re-parse mismatch");
}

#[test]
fn inbound_ping_fixture() {
    round_trip_inbound("inbound_ping.json");
}

#[test]
fn inbound_download_fixture() {
    round_trip_inbound("inbound_download.json");
}

/// The refresh pair. These are the first variants with multi-word fields, and
/// `rename_all` on the enum renames variants rather than fields — so each
/// carries its own per-variant `rename_all`. These fixtures are what catches a
/// regression back to `download_id`.
#[test]
fn inbound_refresh_download_fixture() {
    round_trip_inbound("inbound_refresh_download.json");
}

#[test]
fn inbound_credentials_refreshed_fixture() {
    round_trip_inbound("inbound_credentials_refreshed.json");
}

#[test]
fn outbound_arm_refresh_fixture() {
    round_trip_outbound("outbound_arm_refresh.json");
}

#[test]
fn outbound_refresh_credentials_fixture() {
    round_trip_outbound("outbound_refresh_credentials.json");
}

/// The round-trip helpers would pass even if every field silently vanished
/// into a default, so assert the camelCase keys are actually consumed.
#[test]
fn refresh_fixtures_use_camel_case_keys() {
    let raw = read("inbound_refresh_download.json");
    let parsed: Inbound = serde_json::from_str(&raw).expect("parse");
    match parsed {
        Inbound::RefreshDownload { download_id, job } => {
            assert_eq!(download_id, 42);
            assert_eq!(
                job.final_url,
                "https://cdn.example.com/file.zip?token=fresh"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }

    let raw = read("outbound_arm_refresh.json");
    let parsed: Outbound = serde_json::from_str(&raw).expect("parse");
    match parsed {
        Outbound::ArmRefresh {
            download_id,
            size_bytes,
            expires_at_ms,
            ..
        } => {
            assert_eq!(download_id, 42);
            assert_eq!(size_bytes, Some(123_456));
            assert_eq!(expires_at_ms, 1_786_000_000_000);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn inbound_download_media_fixture() {
    round_trip_inbound("inbound_download_media.json");
}

#[test]
fn inbound_status_fixture() {
    round_trip_inbound("inbound_status.json");
}

#[test]
fn outbound_pong_fixture() {
    round_trip_outbound("outbound_pong.json");
}

#[test]
fn outbound_ack_fixture() {
    round_trip_outbound("outbound_ack.json");
}

#[test]
fn outbound_status_fixture() {
    round_trip_outbound("outbound_status.json");
}

#[test]
fn outbound_error_fixture() {
    round_trip_outbound("outbound_error.json");
}
