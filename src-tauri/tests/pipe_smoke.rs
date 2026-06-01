//! End-to-end smoke test for the in-app pipe server: open an in-memory
//! `Core`, install the pipe on a unique per-test name, connect a
//! `NamedPipeClient`, send one framed `download` message, and assert
//! the resulting `DownloadAdded` event surfaces the captured headers.
//!
//! Locks the contract: extension → host → pipe → engine
//! payload all carry Cookie / Referer / User-Agent + observed
//! `webRequest` headers verbatim.

#![cfg(windows)]

use std::time::Duration;

use tokio::time::timeout;

use unduhin_app_lib::pipe;
use unduhin_core::wire::framing::{read_frame, write_frame};
use unduhin_core::wire::{DownloadJob, Inbound, Outbound, RequestHeader};
use unduhin_core::{Core, CoreEvent};

async fn open_core() -> Core {
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
    format!(r"\\.\pipe\unduhin-pipe-smoke-{pid}-{counter}")
}

#[tokio::test]
async fn download_message_lands_with_captured_headers() {
    use tokio::net::windows::named_pipe::ClientOptions;

    let core = open_core().await;
    let mut events = core.subscribe();

    let name = unique_pipe_name();
    // Spawn the server on the same runtime; abort handle drops at
    // end of test (the pipe handle goes with it).
    let server_task = {
        let core = core.clone();
        let name = name.clone();
        tokio::spawn(async move {
            if let Err(e) = pipe::run_server(name, core).await {
                eprintln!("server exited: {e}");
            }
        })
    };

    // Wait for the server to be listening — retry the connect a few
    // times to avoid a race on the create / accept boundary.
    let mut client = None;
    for _ in 0..40 {
        match ClientOptions::new().open(&name) {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    let mut client = client.expect("client connected");

    let job = DownloadJob {
        final_url: "https://example.invalid/test-file.bin".into(),
        original_url: "https://example.invalid/test-file.bin".into(),
        referrer: Some("https://example.invalid/page".into()),
        filename: Some("test-file.bin".into()),
        mime: Some("application/octet-stream".into()),
        size: Some(1024),
        cookie_header: Some("session=smoketest".into()),
        user_agent: Some("UnduhinPipeSmoke/1.0".into()),
        request_headers: vec![RequestHeader {
            name: "X-Captured".into(),
            value: "captured-value".into(),
        }],
        tab_id: Some(7),
        page_url: Some("https://example.invalid/page".into()),
    };
    let msg = Inbound::Download { job };
    let buf = serde_json::to_vec(&msg).unwrap();
    write_frame(&mut client, &buf).await.expect("write frame");

    let resp_buf = timeout(Duration::from_secs(2), read_frame(&mut client))
        .await
        .expect("response timed out")
        .expect("read frame")
        .expect("frame not None");
    let outbound: Outbound = serde_json::from_slice(&resp_buf).expect("parse Outbound");
    let id = match outbound {
        Outbound::Ack { id } => id,
        other => panic!("expected Ack, got {other:?}"),
    };
    assert!(id > 0, "ack id should be positive");

    // Drain events until we see DownloadAdded for our id.
    let event = timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await {
                Ok(CoreEvent::DownloadAdded {
                    id: ev_id,
                    snapshot,
                }) if ev_id == id => {
                    return *snapshot;
                }
                Ok(_) => continue,
                Err(_) => panic!("event stream closed before DownloadAdded"),
            }
        }
    })
    .await
    .expect("DownloadAdded timed out");

    let headers = event.headers.expect("captured headers stored");
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "Cookie" && v == "session=smoketest"),
        "Cookie header missing: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "Referer" && v == "https://example.invalid/page"),
        "Referer header missing: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "User-Agent" && v == "UnduhinPipeSmoke/1.0"),
        "User-Agent header missing: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "X-Captured" && v == "captured-value"),
        "observed webRequest header missing: {headers:?}"
    );

    server_task.abort();
}

#[tokio::test]
async fn ping_pong_over_pipe() {
    use tokio::net::windows::named_pipe::ClientOptions;

    let core = open_core().await;
    let name = unique_pipe_name();
    let server_task = {
        let core = core.clone();
        let name = name.clone();
        tokio::spawn(async move {
            let _ = pipe::run_server(name, core).await;
        })
    };

    let mut client = None;
    for _ in 0..40 {
        if let Ok(c) = ClientOptions::new().open(&name) {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("client connected");

    let ping = serde_json::to_vec(&Inbound::Ping).unwrap();
    write_frame(&mut client, &ping).await.unwrap();
    let frame = timeout(Duration::from_secs(1), read_frame(&mut client))
        .await
        .expect("pong timed out")
        .expect("read frame")
        .expect("frame not None");
    let outbound: Outbound = serde_json::from_slice(&frame).unwrap();
    assert!(matches!(outbound, Outbound::Pong));

    server_task.abort();
}
