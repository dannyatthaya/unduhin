//! Headless stand-in for the app, for driving the real extension in a real
//! browser (`scripts/e2e-browser.mjs`).
//!
//! It is everything the extension talks to — the pipe server, the queue, the
//! HTTP engine and the capture rescue — with no window and an in-memory
//! database, so a run cannot touch an installed app or its history.
//!
//! Ignored by default: it does nothing useful under `cargo test`, and runs
//! until its stop file appears. Started by the e2e script with:
//!
//! ```text
//! UNDUHIN_E2E_PIPE=<pipe name> UNDUHIN_E2E_LOG=<file> UNDUHIN_E2E_STOP=<file> \
//!   cargo test -p unduhin-app --test browser_e2e_harness -- --ignored
//! ```
//!
//! Every download event is appended to the log as one JSON line, which is
//! how the script sees what the app did.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use unduhin_app_lib::pipe;
use unduhin_core::{Core, CoreEvent};

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set")))
}

fn append(log: &PathBuf, line: &serde_json::Value) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .expect("open e2e log");
    writeln!(file, "{line}").expect("write e2e log");
}

#[tokio::test]
#[ignore = "long-running harness for scripts/e2e-browser.mjs"]
async fn serve_until_stopped() {
    let pipe_name = std::env::var("UNDUHIN_E2E_PIPE").expect("UNDUHIN_E2E_PIPE must be set");
    let log = env_path("UNDUHIN_E2E_LOG");
    let stop = env_path("UNDUHIN_E2E_STOP");

    let core = Core::open_in_memory().await.expect("open in-memory core");
    core.start().await.expect("start queue");

    {
        let core = core.clone();
        let log = log.clone();
        let mut events = core.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                match &event {
                    CoreEvent::DownloadAdded { id, snapshot } => append(
                        &log,
                        &serde_json::json!({ "event": "added", "id": id, "url": snapshot.url }),
                    ),
                    CoreEvent::Failed { id, error, .. } => {
                        let url = core.get_download(*id).await.map(|r| r.url).ok();
                        append(
                            &log,
                            &serde_json::json!({ "event": "failed", "id": id, "url": url, "error": error }),
                        );
                    }
                    CoreEvent::Completed { id, .. } => {
                        append(&log, &serde_json::json!({ "event": "completed", "id": id }));
                    }
                    _ => {}
                }
                // What `forward_core_events` does in the running app.
                if pipe::rescue::wants(&event) {
                    pipe::rescue::on_core_event(&core, &event).await;
                }
            }
        });
    }

    {
        let core = core.clone();
        tokio::spawn(async move {
            if let Err(e) = pipe::run_server(pipe_name, core).await {
                eprintln!("pipe server exited: {e}");
            }
        });
    }
    append(&log, &serde_json::json!({ "event": "ready" }));

    while !stop.exists() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
