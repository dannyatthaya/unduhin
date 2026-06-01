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
