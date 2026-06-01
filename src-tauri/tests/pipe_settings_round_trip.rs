//! Settings round-trip across the in-app pipe server.
//!
//! Two scenarios in one test because the settings cache is a process-
//! global; splitting into separate `#[tokio::test]`s would race under
//! cargo's default parallel runner.
//!
//! - Empty cache: `Inbound::GetSettings` returns the canonical defaults
//!   (matches the extension's `DEFAULT_SETTINGS` byte-for-byte).
//! - One client sends `Inbound::SetSettings { patch }`; the server
//!   caches and replies `Outbound::Settings { full }` to the sender,
//!   and a second connected client receives an unsolicited
//!   `Outbound::SettingsChanged { full }` broadcast.
//! - Follow-up `Inbound::GetSettings` from a fresh connection returns
//!   the cached snapshot (proves the cache survives across requests).

#![cfg(windows)]

use std::time::Duration;

use tokio::time::timeout;

use unduhin_app_lib::pipe;
use unduhin_core::wire::framing::{read_frame, write_frame};
use unduhin_core::wire::{HandoffMode, Inbound, Outbound, SettingsPatch};
use unduhin_core::Core;

async fn open_core() -> Core {
    pipe::reset_settings_cache_for_tests().await;
    let core = Core::open_in_memory().await.expect("open in-memory core");
    core.start().await.expect("start queue");
    core
}

fn unique_pipe_name() -> String {
    let pid = std::process::id();
    let counter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(r"\\.\pipe\unduhin-settings-rt-{pid}-{counter}")
}

async fn connect_client(name: &str) -> tokio::net::windows::named_pipe::NamedPipeClient {
    use tokio::net::windows::named_pipe::ClientOptions;
    for _ in 0..40 {
        if let Ok(c) = ClientOptions::new().open(name) {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("client failed to connect to {name}");
}

#[tokio::test]
async fn settings_get_set_and_broadcast_round_trip() {
    let core = open_core().await;
    let name = unique_pipe_name();
    let server_task = {
        let core = core.clone();
        let name = name.clone();
        tokio::spawn(async move {
            let _ = pipe::run_server(name, core).await;
        })
    };

    // Empty cache → GetSettings returns defaults.
    let mut empty = connect_client(&name).await;
    let buf = serde_json::to_vec(&Inbound::GetSettings).unwrap();
    write_frame(&mut empty, &buf)
        .await
        .expect("write GetSettings");
    let frame = timeout(Duration::from_secs(2), read_frame(&mut empty))
        .await
        .expect("defaults reply timed out")
        .expect("read frame")
        .expect("frame not None");
    let outbound: Outbound = serde_json::from_slice(&frame).expect("parse Outbound");
    match outbound {
        Outbound::Settings { full } => {
            assert!(full.enabled);
            assert_eq!(full.mode, HandoffMode::CatchAll);
            assert!(full.install_context_menu);
            assert!(full.hide_shelf);
            assert!(full.forward_cookies);
            assert_eq!(full.min_size_mb, 1);
            assert_eq!(full.extension_blocklist, vec!["html", "pdf", "txt", "json"]);
        }
        other => panic!("expected Settings, got {other:?}"),
    }
    drop(empty);

    // SetSettings caches, replies, and broadcasts.
    let mut a = connect_client(&name).await;
    let mut b = connect_client(&name).await;
    // Server's accept loop registers each writer before spawning the
    // read loop; give both connections time to land in CONNECTED_CLIENTS
    // before triggering the broadcast.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let patch = SettingsPatch {
        mode: Some(HandoffMode::AskFirst),
        min_size_mb: Some(42),
        install_context_menu: Some(false),
        ..Default::default()
    };
    let buf = serde_json::to_vec(&Inbound::SetSettings { patch }).unwrap();
    write_frame(&mut a, &buf).await.expect("write SetSettings");

    // Direct reply to A.
    let frame = timeout(Duration::from_secs(2), read_frame(&mut a))
        .await
        .expect("A: reply timed out")
        .expect("A: read frame")
        .expect("A: frame not None");
    let outbound: Outbound = serde_json::from_slice(&frame).expect("A: parse");
    match outbound {
        Outbound::Settings { full } => {
            assert_eq!(full.mode, HandoffMode::AskFirst);
            assert_eq!(full.min_size_mb, 42);
            assert!(!full.install_context_menu);
            assert!(full.enabled);
            assert!(full.forward_cookies);
        }
        other => panic!("A: expected Settings, got {other:?}"),
    }

    // Unsolicited broadcast on B.
    let frame = timeout(Duration::from_secs(2), read_frame(&mut b))
        .await
        .expect("B: broadcast timed out")
        .expect("B: read frame")
        .expect("B: frame not None");
    let outbound: Outbound = serde_json::from_slice(&frame).expect("B: parse");
    match outbound {
        Outbound::SettingsChanged { full } => {
            assert_eq!(full.mode, HandoffMode::AskFirst);
            assert_eq!(full.min_size_mb, 42);
        }
        other => panic!("B: expected SettingsChanged, got {other:?}"),
    }

    // Cache survived; fresh connection sees AskFirst.
    let mut c = connect_client(&name).await;
    let buf = serde_json::to_vec(&Inbound::GetSettings).unwrap();
    write_frame(&mut c, &buf).await.expect("write GetSettings");
    let frame = timeout(Duration::from_secs(2), read_frame(&mut c))
        .await
        .expect("C: reply timed out")
        .expect("C: read frame")
        .expect("C: frame not None");
    let outbound: Outbound = serde_json::from_slice(&frame).expect("C: parse");
    match outbound {
        Outbound::Settings { full } => {
            assert_eq!(full.mode, HandoffMode::AskFirst);
            assert_eq!(full.min_size_mb, 42);
            assert!(!full.install_context_menu);
        }
        other => panic!("C: expected Settings, got {other:?}"),
    }

    let cached = pipe::cached_extension_settings()
        .await
        .expect("cache populated");
    assert_eq!(cached.mode, HandoffMode::AskFirst);

    server_task.abort();
}
