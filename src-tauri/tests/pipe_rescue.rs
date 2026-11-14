//! End-to-end test for the capture rescue chain (`pipe::rescue`): a real
//! pipe client standing in for the extension, a real `Core` with its queue
//! and HTTP engine, and a local HTTP server standing in for the file host.
//!
//! One test function on purpose. The rescue bookkeeping and the list of
//! connected pipe clients are per-process, and every in-memory `Core` starts
//! its row ids at 1 — two tests in this binary would trample each other.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use unduhin_app_lib::pipe;
use unduhin_core::wire::framing::{read_frame, write_frame};
use unduhin_core::wire::transport::ClientStream;
use unduhin_core::wire::{DownloadJob, Inbound, Outbound};
use unduhin_core::{Core, DownloadId, Status};

const WAIT: Duration = Duration::from_secs(20);

/// A file host with three kinds of link:
///
/// - `/spent.zip`   — always 403: a single-use URL the browser already used.
/// - `/clicked.zip` — the link behind it. Answers the probe (HEAD) but 404s
///   the GET, so the retry is observable without a file ever landing in the
///   user's real Downloads folder.
/// - `/browser-only.zip` — always 403: a host that only serves a browser.
async fn start_file_host() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf);
                let mut parts = head.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                log.lock().unwrap().push(format!("{method} {path}"));

                let status = match (method.as_str(), path.as_str()) {
                    ("HEAD", "/clicked.zip") => "200 OK",
                    (_, "/clicked.zip") => "404 Not Found",
                    _ => "403 Forbidden",
                };
                let response =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (base, seen)
}

fn job(final_url: String, original_url: String) -> DownloadJob {
    DownloadJob {
        final_url,
        original_url,
        referrer: None,
        filename: Some("file.zip".into()),
        mime: Some("application/zip".into()),
        size: None,
        cookie_header: Some("session=old".into()),
        user_agent: Some("UnduhinRescueTest/1.0".into()),
        request_headers: Vec::new(),
        tab_id: None,
        page_url: None,
    }
}

async fn send(client: &mut ClientStream, msg: &Inbound) {
    let buf = serde_json::to_vec(msg).expect("serialize");
    write_frame(client, &buf).await.expect("write frame");
}

async fn next_frame(client: &mut ClientStream) -> Outbound {
    let buf = timeout(WAIT, read_frame(client))
        .await
        .expect("timed out waiting for a frame")
        .expect("read frame")
        .expect("pipe closed");
    serde_json::from_slice(&buf).expect("parse Outbound")
}

/// Hand `job` to the app the way the extension does and return the row id.
async fn capture(client: &mut ClientStream, job: DownloadJob) -> DownloadId {
    send(client, &Inbound::Download { job }).await;
    match next_frame(client).await {
        Outbound::Ack { id } => id,
        other => panic!("expected an ack, got {other:?}"),
    }
}

async fn wait_until_failed(core: &Core, id: DownloadId, url_suffix: &str) {
    let start = Instant::now();
    loop {
        let rec = core.get_download(id).await.expect("row");
        if rec.status == Status::Failed && rec.url.ends_with(url_suffix) {
            return;
        }
        assert!(
            start.elapsed() < WAIT,
            "row {id} never failed at {url_suffix}: status {:?}, url {}",
            rec.status,
            rec.url
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_failed_capture_is_retried_from_the_clicked_link_then_handed_back() {
    let core = Core::open_in_memory().await.expect("open in-memory core");
    core.start().await.expect("start queue");

    // What `forward_core_events` does in the running app.
    {
        let core = core.clone();
        let mut events = core.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                if pipe::rescue::wants(&event) {
                    pipe::rescue::on_core_event(&core, &event).await;
                }
            }
        });
    }

    let name = common::unique_endpoint("pipe-rescue");
    {
        let core = core.clone();
        let name = name.clone();
        tokio::spawn(async move {
            if let Err(e) = pipe::run_server(name, core).await {
                eprintln!("server exited: {e}");
            }
        });
    }
    let mut client = common::connect_client(&name).await;
    let (host, seen) = start_file_host().await;

    // --- A single-use link: restart from the URL the user clicked ----------

    let id = capture(
        &mut client,
        job(format!("{host}/spent.zip"), format!("{host}/clicked.zip")),
    )
    .await;

    // The row moves to the clicked link and is tried again. That link 404s
    // its GET, which is a plain failure — not "expired" — so the chain stops.
    wait_until_failed(&core, id, "/clicked.zip").await;
    let requests = seen.lock().unwrap().clone();
    assert!(
        requests.iter().any(|r| r == "GET /spent.zip"),
        "the captured URL was never tried: {requests:?}"
    );
    assert!(
        requests.iter().any(|r| r == "GET /clicked.zip"),
        "the download was not restarted from the clicked link: {requests:?}"
    );

    // --- A host that only serves a browser: hand the download back ---------

    let blocked = format!("{host}/browser-only.zip");
    let id = capture(&mut client, job(blocked.clone(), blocked.clone())).await;

    // No other link to fall back on, so the app first asks for fresh cookies.
    let token = match next_frame(&mut client).await {
        Outbound::RefreshCredentials {
            token,
            download_id,
            url,
        } => {
            assert_eq!(download_id, id);
            assert_eq!(url, blocked);
            token
        }
        other => panic!("expected refreshCredentials, got {other:?}"),
    };
    send(
        &mut client,
        &Inbound::CredentialsRefreshed {
            token,
            download_id: id,
            cookie_header: Some("session=fresh".into()),
            user_agent: None,
            request_headers: Vec::new(),
        },
    )
    .await;

    // Fresh cookies do not open it either. The app says so in its reply and
    // gives the download back; the two frames may arrive in either order.
    let mut handed_back = None;
    let mut refused = false;
    while handed_back.is_none() || !refused {
        match next_frame(&mut client).await {
            Outbound::DownloadInBrowser { download_id, url } => {
                assert_eq!(download_id, id);
                handed_back = Some(url);
            }
            Outbound::Error { .. } => refused = true,
            other => panic!("unexpected frame: {other:?}"),
        }
    }
    assert_eq!(handed_back.as_deref(), Some(blocked.as_str()));

    // And only once: nothing further is pushed for this row.
    let extra = timeout(Duration::from_millis(750), read_frame(&mut client)).await;
    assert!(extra.is_err(), "an unexpected extra frame arrived");
    let rec = core.get_download(id).await.expect("row");
    assert_eq!(rec.status, Status::Failed);
}
