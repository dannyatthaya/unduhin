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
//! The extension matches replies to requests by order alone, so the host
//! keeps the app's replies in step with the requests it forwarded: when the
//! connection to the app drops, every request still waiting is answered
//! locally with an `Error` (see [`Owed`]) rather than left hanging.
//!
//! Cross-platform note: the bridge runs on Windows and macOS, over a
//! named pipe and a Unix domain socket respectively (see
//! `unduhin_core::wire::transport`). The binary still compiles elsewhere
//! so the workspace stays portable; on those targets it serves one `Ping`
//! and exits.

#[cfg(any(windows, target_os = "macos"))]
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

    #[cfg(any(windows, target_os = "macos"))]
    return run_bridge().await;

    #[cfg(not(any(windows, target_os = "macos")))]
    return run_stub().await;
}

/// Stdout writer task channel capacity. 32 frames is enough to absorb a
/// burst of `SettingsChanged` pushes while a slow stdout consumer
/// catches up; back-pressure naturally throttles the upstream pumps
/// past that.
#[cfg(any(windows, target_os = "macos"))]
const STDOUT_QUEUE_CAPACITY: usize = 32;

/// What the host tells the extension for each request whose reply was lost
/// with the connection to the app.
#[cfg(any(windows, target_os = "macos"))]
const CONNECTION_LOST: &str = "the connection to the Unduhin app was lost before it replied";

/// Replies the app still owes on one connection.
///
/// The extension matches replies to requests purely by order. When the app
/// drops the connection (quit, crash) with requests outstanding, those
/// replies never come, and the extension would hand every later reply to the
/// wrong request. So the host counts each frame it forwards and, the moment
/// the connection is known to be dead, answers every outstanding one itself
/// with an `Error` — in order, before anything that follows.
///
/// Both pumps take this lock around "decide + enqueue to stdout", so a real
/// reply and a synthesized one can never both answer the same request.
#[cfg(any(windows, target_os = "macos"))]
#[derive(Debug)]
struct Owed {
    count: usize,
    /// Cleared once the owed replies were answered locally. From then on the
    /// connection is only drained: a late real reply would be a second
    /// answer, so it is dropped.
    alive: bool,
}

#[cfg(any(windows, target_os = "macos"))]
impl Owed {
    fn new() -> Self {
        Self {
            count: 0,
            alive: true,
        }
    }

    /// Mark the connection dead and answer everything it still owed. A
    /// second call finds nothing owed and does nothing, so whichever side
    /// notices the failure first does the answering.
    async fn fail_all(&mut self, stdout_tx: &tokio::sync::mpsc::Sender<Vec<u8>>) {
        self.alive = false;
        for _ in 0..std::mem::take(&mut self.count) {
            send_outbound(
                stdout_tx,
                &Outbound::Error {
                    message: CONNECTION_LOST.into(),
                },
            )
            .await;
        }
    }
}

/// One live connection to the app.
#[cfg(any(windows, target_os = "macos"))]
struct Link {
    writer: tokio::io::WriteHalf<unduhin_core::wire::transport::ClientStream>,
    owed: std::sync::Arc<tokio::sync::Mutex<Owed>>,
    /// The pipe→stdout pump reading this connection's other half.
    pump: tokio::task::JoinHandle<()>,
}

#[cfg(any(windows, target_os = "macos"))]
async fn run_bridge() -> anyhow::Result<()> {
    use tokio::sync::mpsc;

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

    let result = pump_stdin_to_pipe(stdin(), stdout_tx.clone(), host_exe.as_deref()).await;

    // `pump_stdin_to_pipe` aborted the pipe→stdout pump on its way out, so
    // that pump's stdout-sender clone is gone; drop ours and the writer
    // task drains and exits.
    drop(stdout_tx);
    let _ = stdout_task.await;
    info!("native host exiting cleanly");
    result
}

#[cfg(any(windows, target_os = "macos"))]
async fn pump_stdin_to_pipe<R>(
    mut stdin_reader: R,
    stdout_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    host_exe: Option<&std::path::Path>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // Created lazily on the first non-Ping frame, so Ping stays fast-pathed
    // without a live connection.
    let mut link: Option<Link> = None;

    let result = loop {
        let frame = match read_frame(&mut stdin_reader).await {
            Ok(Some(buf)) => buf,
            Ok(None) => {
                info!("stdin EOF — exiting cleanly");
                break Ok(());
            }
            Err(e) => {
                warn!(error = %e, "stdin read failure — exiting");
                break Ok(());
            }
        };

        // `Ping` is answered locally so the extension can health-check
        // the host *without* depending on the main app being up. The
        // extension routes `pong` outside its reply queue, so answering it
        // ahead of replies still owed by the app is fine.
        let parsed = serde_json::from_slice::<Inbound>(&frame);
        if matches!(parsed, Ok(Inbound::Ping)) {
            send_outbound(&stdout_tx, &Outbound::Pong).await;
            continue;
        }

        // A connection whose owed replies were already answered locally is
        // only good for draining; drop it so this frame reconnects.
        if let Some(l) = link.as_ref() {
            if !l.owed.lock().await.alive {
                if let Some(old) = link.take() {
                    old.pump.abort();
                }
            }
        }

        if let Err(e) = parsed {
            // Answer locally only when nothing is owed ahead of this frame —
            // otherwise the error would overtake those replies. With a live
            // connection, forward it and let the app answer in turn (it
            // replies to malformed JSON the same way). Never launch the app
            // just to report bad input.
            if link.is_none() {
                send_outbound(
                    &stdout_tx,
                    &Outbound::Error {
                        message: format!("invalid json: {e}"),
                    },
                )
                .await;
                continue;
            }
        } else if link.is_none() {
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
                    let (read_half, writer) = tokio::io::split(stream);
                    let owed = std::sync::Arc::new(tokio::sync::Mutex::new(Owed::new()));
                    let pump = tokio::spawn(pump_pipe_to_stdout(
                        read_half,
                        stdout_tx.clone(),
                        owed.clone(),
                    ));
                    link = Some(Link { writer, owed, pump });
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

        let l = link.as_mut().expect("connected above");
        // Count the reply BEFORE writing. The lock is not held across the
        // write: the app may be blocked writing a reply the pump has to read
        // first, and holding it would deadlock the two.
        l.owed.lock().await.count += 1;
        if let Err(e) = write_frame(&mut l.writer, &frame).await {
            warn!(error = %e, "pipe write failed; dropping connection");
            // Answers this frame too — it was counted above.
            l.owed.lock().await.fail_all(&stdout_tx).await;
            if let Some(old) = link.take() {
                old.pump.abort();
            }
        }
    };

    // Stdin EOF (or a fatal read error) closes the host. Abort the
    // pipe→stdout pump so its stdout-sender clone drops and the writer task
    // can drain and exit.
    if let Some(l) = link.take() {
        l.pump.abort();
    }
    result
}

/// True when `frame` is a reply to a forwarded request rather than a push
/// the app sent on its own. Anything unparseable counts as a reply: the app
/// only ever sends typed frames, so that can only be an answer to something.
#[cfg(any(windows, target_os = "macos"))]
fn is_reply_frame(frame: &[u8]) -> bool {
    serde_json::from_slice::<Outbound>(frame)
        .map(|msg| !msg.is_unsolicited())
        .unwrap_or(true)
}

/// Drain the pipe's read half into stdout. Every frame is forwarded
/// verbatim — the extension's `connectNative` port deserialises it — except
/// a reply that arrives after the connection's owed replies were already
/// answered locally.
///
/// On EOF (the app closed the connection) or a read error, answers every
/// request the app still owed and exits; the next non-Ping frame from the
/// extension reconnects.
#[cfg(any(windows, target_os = "macos"))]
async fn pump_pipe_to_stdout(
    mut read_half: tokio::io::ReadHalf<unduhin_core::wire::transport::ClientStream>,
    stdout_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    owed: std::sync::Arc<tokio::sync::Mutex<Owed>>,
) {
    loop {
        match read_frame(&mut read_half).await {
            Ok(Some(buf)) => {
                let mut owed = owed.lock().await;
                if is_reply_frame(&buf) {
                    if !owed.alive {
                        tracing::debug!("dropping a reply that was already answered locally");
                        continue;
                    }
                    owed.count = owed.count.saturating_sub(1);
                }
                if stdout_tx.send(buf).await.is_err() {
                    // Stdout writer is gone — host is shutting down.
                    return;
                }
            }
            Ok(None) => {
                tracing::debug!("pipe EOF — pump_pipe_to_stdout exiting");
                owed.lock().await.fail_all(&stdout_tx).await;
                return;
            }
            Err(e) => {
                tracing::debug!(error = %e, "pipe read failure — pump_pipe_to_stdout exiting");
                owed.lock().await.fail_all(&stdout_tx).await;
                return;
            }
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
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

#[cfg(not(any(windows, target_os = "macos")))]
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
                message: "the native host bridge is available on Windows and macOS only".into(),
            }
        };
        let _ = write_frame(&mut stdout, &serde_json::to_vec(&response)?).await;
    }
}
