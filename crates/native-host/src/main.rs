//! Unduhin native messaging host.
//!
//! Spawned per session by the browser via the Native Messaging protocol
//! (`chrome.runtime.connectNative("com.unduhin.host")`). Reads framed
//! JSON from stdin, writes framed JSON to stdout, and forwards every
//! non-`Ping` message to the long-running Unduhin app over the
//! `\\.\pipe\unduhin` named pipe.
//!
//! Logs go to **stderr** — browsers discard anything mixed into stdout,
//! and the framing strictly owns stdout. Set `UNDUHIN_LOG=debug` for
//! verbose tracing during development.
//!
//! ## Bidirectional pumps
//!
//! Two concurrent tasks share the connected pipe so the Tauri side can
//! push unsolicited frames (`SettingsChanged`) down to the extension
//! without the host having to poll:
//!
//!   - [`pump_stdin_to_pipe`] reads framed JSON from stdin, intercepts
//!     `Ping` locally (answers with `Pong` so the extension can
//!     health-check the host even when the app is down), and forwards
//!     everything else to the pipe write-half.
//!   - [`pump_pipe_to_stdout`] reads framed JSON from the pipe and
//!     forwards it to stdout. This is what carries unsolicited
//!     `SettingsChanged` pushes through to the extension's
//!     `connectNative` port.
//!
//! All stdout writes funnel through a single mpsc-backed writer task so
//! the two pumps can't interleave bytes mid-frame.
//!
//! Cross-platform note: Native Messaging on Chrome/Edge is Windows-only
//! in our target matrix. The binary still compiles on other targets so
//! the workspace stays portable; on non-Windows it serves one `Ping`
//! and exits.

#[cfg(windows)]
mod pipe_client;

use std::io::IsTerminal;

use tokio::io::{stdin, stdout};
use tracing::{info, warn};
use unduhin_core::wire::framing::{read_frame, write_frame};
use unduhin_core::wire::{Inbound, Outbound};

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    // Stderr-only — stdout is reserved for the Native Messaging frame
    // stream. Default to `warn` so a normally-running host is silent.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_env("UNDUHIN_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .try_init();
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    if std::io::stdin().is_terminal() {
        // Manual invocation — no browser. Print a short hint to
        // stderr so a confused user understands why nothing happens.
        eprintln!(
            "unduhin-native-host: this binary is invoked by Chromium-based browsers via\n\
             Native Messaging (chrome.runtime.connectNative). It is not meant to be\n\
             run interactively."
        );
    }

    info!("unduhin-native-host starting");

    #[cfg(windows)]
    return run_windows().await;

    #[cfg(not(windows))]
    return run_stub().await;
}

/// Stdout writer task channel capacity. 32 frames is enough to absorb a
/// burst of `SettingsChanged` pushes while a slow stdout consumer
/// catches up; back-pressure naturally throttles the upstream pumps
/// past that.
#[cfg(windows)]
const STDOUT_QUEUE_CAPACITY: usize = 32;

#[cfg(windows)]
async fn run_windows() -> anyhow::Result<()> {
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex};

    let host_exe = std::env::current_exe().ok();

    // Single owner of stdout — both pumps write through this channel
    // so a partial write from one can't interleave with the other.
    let (stdout_tx, mut stdout_rx) = mpsc::channel::<Vec<u8>>(STDOUT_QUEUE_CAPACITY);
    let stdout_task = tokio::spawn(async move {
        let mut out = stdout();
        while let Some(buf) = stdout_rx.recv().await {
            if let Err(e) = write_frame(&mut out, &buf).await {
                warn!(error = %e, "stdout write failed — exiting writer");
                break;
            }
        }
    });

    // The pipe's write half is created lazily on first non-Ping frame;
    // an empty slot keeps Ping fast-pathed without a live pipe.
    // Shared because the lazy-connect path also spawns the
    // pipe→stdout pump, which holds the matching read half.
    let pipe_writer: Arc<
        Mutex<Option<tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>>>,
    > = Arc::new(Mutex::new(None));

    // Handle to the pipe→stdout pump (None until first lazy connect).
    // Aborted on stdin EOF so the pump's stdout-sender clone drops and
    // the stdout writer task can drain and exit.
    let pipe_pump: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> = Arc::new(Mutex::new(None));

    let result = pump_stdin_to_pipe(
        stdin(),
        stdout_tx.clone(),
        pipe_writer.clone(),
        pipe_pump.clone(),
        host_exe.as_deref(),
    )
    .await;

    // Stdin EOF (or a fatal read error) closes the host. Abort the
    // pipe→stdout pump so its stdout-sender clone drops; then drop the
    // one we hold here; the writer task drains and exits.
    if let Some(handle) = pipe_pump.lock().await.take() {
        handle.abort();
    }
    drop(stdout_tx);
    let _ = stdout_task.await;
    info!("native host exiting cleanly");
    result
}

#[cfg(windows)]
async fn pump_stdin_to_pipe<R>(
    mut stdin_reader: R,
    stdout_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    pipe_writer: std::sync::Arc<
        tokio::sync::Mutex<
            Option<tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>>,
        >,
    >,
    pipe_pump: std::sync::Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    host_exe: Option<&std::path::Path>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        let frame = match read_frame(&mut stdin_reader).await {
            Ok(Some(buf)) => buf,
            Ok(None) => {
                info!("stdin EOF — exiting cleanly");
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "stdin read failure — exiting");
                return Ok(());
            }
        };

        let inbound: Inbound = match serde_json::from_slice(&frame) {
            Ok(msg) => msg,
            Err(e) => {
                send_outbound(
                    &stdout_tx,
                    &Outbound::Error {
                        message: format!("invalid json: {e}"),
                    },
                )
                .await;
                continue;
            }
        };

        // `Ping` is answered locally so the extension can health-check
        // the host *without* depending on the main app being up.
        if matches!(inbound, Inbound::Ping) {
            send_outbound(&stdout_tx, &Outbound::Pong).await;
            continue;
        }

        // Lazy-connect on first non-Ping. The pipe→stdout pump is
        // spawned on the same connect so unsolicited frames can flow
        // back the moment the connection is live.
        if pipe_writer.lock().await.is_none() {
            let exe = match host_exe {
                Some(p) => p,
                None => {
                    send_outbound(
                        &stdout_tx,
                        &Outbound::Error {
                            message: "current_exe unavailable; cannot locate app binary".into(),
                        },
                    )
                    .await;
                    continue;
                }
            };
            match pipe_client::connect_or_launch(exe).await {
                Ok(stream) => {
                    let (read_half, write_half) = tokio::io::split(stream);
                    *pipe_writer.lock().await = Some(write_half);
                    // Abort any prior pump task before replacing its handle.
                    // After a failed write we clear the writer but leave the
                    // pump running; without this abort, the next reconnect
                    // overwrites the `JoinHandle` (detach, not abort),
                    // leaking the old task — which keeps reading the dead
                    // pipe and could forward a stray late frame to stdout.
                    if let Some(old) = pipe_pump.lock().await.take() {
                        old.abort();
                    }
                    let handle = tokio::spawn(pump_pipe_to_stdout(read_half, stdout_tx.clone()));
                    *pipe_pump.lock().await = Some(handle);
                }
                Err(e) => {
                    send_outbound(
                        &stdout_tx,
                        &Outbound::Error {
                            message: format!("{e:#}"),
                        },
                    )
                    .await;
                    continue;
                }
            }
        }

        // Forward the frame to the pipe; on failure tear the writer
        // down so the next message reconnects.
        let mut guard = pipe_writer.lock().await;
        let writer = guard.as_mut().expect("connected");
        if let Err(e) = write_frame(writer, &frame).await {
            warn!(error = %e, "pipe write failed; dropping connection");
            *guard = None;
            send_outbound(
                &stdout_tx,
                &Outbound::Error {
                    message: format!("pipe write failed: {e}"),
                },
            )
            .await;
        }
    }
}

/// Drain the pipe's read half into stdout. Every frame is forwarded
/// verbatim — the extension's `connectNative` port deserialises it.
/// Exits cleanly on EOF (the main app closed the connection) or on a
/// read error; the next `pump_stdin_to_pipe` write will reconnect and
/// respawn this task.
#[cfg(windows)]
async fn pump_pipe_to_stdout(
    mut read_half: tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeClient>,
    stdout_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    loop {
        match read_frame(&mut read_half).await {
            Ok(Some(buf)) => {
                if stdout_tx.send(buf).await.is_err() {
                    // Stdout writer is gone — host is shutting down.
                    return;
                }
            }
            Ok(None) => {
                tracing::debug!("pipe EOF — pump_pipe_to_stdout exiting");
                return;
            }
            Err(e) => {
                tracing::debug!(error = %e, "pipe read failure — pump_pipe_to_stdout exiting");
                return;
            }
        }
    }
}

#[cfg(windows)]
async fn send_outbound(stdout_tx: &tokio::sync::mpsc::Sender<Vec<u8>>, msg: &Outbound) {
    match serde_json::to_vec(msg) {
        Ok(buf) => {
            if stdout_tx.send(buf).await.is_err() {
                tracing::warn!("stdout queue closed; dropping outbound");
            }
        }
        Err(e) => tracing::warn!(error = %e, "serialize Outbound failed"),
    }
}

#[cfg(not(windows))]
async fn run_stub() -> anyhow::Result<()> {
    let mut stdin = stdin();
    let mut stdout = stdout();
    loop {
        let frame = match read_frame(&mut stdin).await {
            Ok(Some(buf)) => buf,
            _ => return Ok(()),
        };
        let inbound: Inbound = match serde_json::from_slice(&frame) {
            Ok(m) => m,
            Err(e) => {
                let _ = write_frame(
                    &mut stdout,
                    &serde_json::to_vec(&Outbound::Error {
                        message: format!("invalid json: {e}"),
                    })?,
                )
                .await;
                continue;
            }
        };
        let response = if matches!(inbound, Inbound::Ping) {
            Outbound::Pong
        } else {
            Outbound::Error {
                message: "native host is Windows-only in this build".into(),
            }
        };
        let _ = write_frame(&mut stdout, &serde_json::to_vec(&response)?).await;
    }
}
