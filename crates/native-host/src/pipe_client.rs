//! Windows named-pipe client used by the native messaging host to
//! forward `Inbound` messages to the long-running Unduhin app.
//!
//! Strategy: first try to open the pipe (fast path when the app is
//! already running). On failure, spawn `unduhin-app.exe` detached and
//! retry the connect with backoff totalling ~5 s before giving up.

#![cfg(windows)]

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tracing::{debug, info};

/// Resolved pipe path. Honours `UNDUHIN_PIPE_NAME` so the
/// integration tests can avoid colliding with a live app instance.
fn pipe_name() -> String {
    std::env::var("UNDUHIN_PIPE_NAME").unwrap_or_else(|_| r"\\.\pipe\unduhin".to_string())
}

/// Backoff schedule for the connect-or-launch path. Totals ~5.1 s — a
/// fresh `unduhin-app.exe` cold-start on a typical Windows box lands
/// the pipe well within that budget.
const RETRY_DELAYS_MS: &[u64] = &[100, 200, 400, 800, 1600, 2000];

/// Open a pipe to the running Unduhin app, spawning it detached if it
/// isn't running yet. Returns the raw connected stream so the caller
/// can split it into independent read/write halves — required by the
/// 9d bidirectional pump (`pump_pipe_to_stdout` needs to read from the
/// pipe while `pump_stdin_to_pipe` is busy writing).
pub async fn connect_or_launch(host_exe: &Path) -> Result<NamedPipeClient> {
    let name = pipe_name();

    // Fast path — app already alive.
    match ClientOptions::new().open(&name) {
        Ok(stream) => {
            debug!(pipe = %name, "connected to existing pipe");
            return Ok(stream);
        }
        Err(e) => {
            debug!(error = %e, pipe = %name, "initial pipe connect failed; will spawn app");
        }
    }

    spawn_app(host_exe).context("spawn unduhin-app for native messaging")?;

    for &delay_ms in RETRY_DELAYS_MS {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if let Ok(stream) = ClientOptions::new().open(&name) {
            info!(pipe = %name, "pipe connected after spawn");
            return Ok(stream);
        }
    }

    Err(anyhow!(
        "named pipe at {name} did not come up within retry window"
    ))
}

/// Spawn `unduhin-app.exe` from the same directory as the host binary,
/// fully detached so the host can exit cleanly when the browser closes
/// the port without dragging the main app down with it.
fn spawn_app(host_exe: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    // The installer drops both binaries under `$INSTDIR\native-host\`,
    // so the app is a sibling of the host.
    let dir = host_exe
        .parent()
        .ok_or_else(|| anyhow!("host exe has no parent dir"))?;
    let app_exe = dir.join("unduhin-app.exe");

    // During dev the user runs the app
    // from `cargo run` and the host doesn't need to spawn it. If the
    // sibling isn't there we still try a plain "unduhin-app" lookup
    // on PATH as a last resort, then bail with a clear error.
    let target = if app_exe.exists() {
        app_exe
    } else {
        std::path::PathBuf::from("unduhin-app.exe")
    };

    // Win32 constants — kept inline so the crate doesn't pull in
    // `windows-sys` just for three integers.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    Command::new(&target)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", target.display()))?;

    info!(?target, "spawned unduhin-app for native messaging");
    Ok(())
}
