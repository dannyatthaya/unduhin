//! Bridge client used by the native messaging host to forward `Inbound`
//! messages to the long-running Unduhin app.
//!
//! Strategy: first try to open the endpoint (fast path when the app is
//! already running). On failure, launch the app detached and retry the
//! connect with backoff before giving up.
//!
//! The transport itself — a named pipe on Windows, a Unix domain socket
//! elsewhere — lives in `unduhin_core::wire::transport`, shared with the
//! app so the two ends can never disagree about where to meet.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tracing::{debug, info};
use unduhin_core::wire::transport::{self, ClientStream};

/// Backoff schedule for the connect-or-launch path.
///
/// Windows totals ~5.1 s, which comfortably covers a cold
/// `unduhin-app.exe` start. macOS gets a longer budget: a cold launch
/// goes through LaunchServices, WKWebView init, and the SQLite migrations
/// before the listener binds, and on a cold filesystem cache that can
/// exceed five seconds.
#[cfg(windows)]
const RETRY_DELAYS_MS: &[u64] = &[100, 200, 400, 800, 1600, 2000];
#[cfg(not(windows))]
const RETRY_DELAYS_MS: &[u64] = &[200, 400, 800, 1600, 3000, 3000, 3000];

/// Open a connection to the running Unduhin app, launching it detached if
/// it is not running yet. Returns the raw stream so the caller can split it
/// into independent read/write halves — required by the bidirectional pump
/// (`pump_pipe_to_stdout` reads while `pump_stdin_to_pipe` writes).
pub async fn connect_or_launch(host_exe: &Path) -> Result<ClientStream> {
    let name = transport::endpoint();

    // Fast path — app already alive.
    match transport::connect(&name).await {
        Ok(stream) => {
            debug!(endpoint = %name, "connected to running app");
            return Ok(stream);
        }
        Err(e) => {
            debug!(error = %e, endpoint = %name, "initial connect failed; will launch app");
        }
    }

    spawn_app(host_exe).context("launch unduhin-app for native messaging")?;

    for &delay_ms in RETRY_DELAYS_MS {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if let Ok(stream) = transport::connect(&name).await {
            info!(endpoint = %name, "connected after launch");
            return Ok(stream);
        }
    }

    Err(anyhow!(
        "bridge at {name} did not come up within the retry window"
    ))
}

/// Spawn `unduhin-app.exe` from the same directory as the host binary,
/// fully detached so the host can exit cleanly when the browser closes
/// the port without dragging the main app down with it.
#[cfg(windows)]
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

/// Launch `Unduhin.app` through LaunchServices.
///
/// `open` rather than a direct spawn, for three reasons. LaunchServices
/// becomes the parent, so the app survives this host process exiting when
/// the browser closes the port — the detachment the Windows creation flags
/// buy explicitly. Addressing the app by bundle identifier survives the
/// user moving it, which a sibling-path lookup would not. And `open`
/// returns a real exit status, so a failure surfaces immediately instead of
/// wasting the whole retry schedule.
///
/// `-g` keeps the app in the background. The user clicked a download link
/// in their browser; stealing focus would be wrong, and it matches the
/// window's `visible: false` start.
#[cfg(target_os = "macos")]
fn spawn_app(host_exe: &Path) -> Result<()> {
    use std::process::{Command, Stdio};

    const BUNDLE_ID: &str = "com.unduhin.app";

    let by_id = Command::new("/usr/bin/open")
        .args(["-g", "-b", BUNDLE_ID])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run /usr/bin/open")?;
    if by_id.success() {
        info!(bundle = BUNDLE_ID, "launched Unduhin for native messaging");
        return Ok(());
    }

    // LaunchServices only knows a bundle id it has seen registered, which
    // does not happen until the app is first launched from Finder. The
    // app records its own location on every run for exactly this case.
    let recorded = unduhin_core::directories_root()
        .map(|root| root.join("native-host").join("app-path.txt"))
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(path) = recorded {
        let by_path = Command::new("/usr/bin/open")
            .args(["-g", "-a", &path])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("run /usr/bin/open with a recorded path")?;
        if by_path.success() {
            info!(path, "launched Unduhin from its recorded location");
            return Ok(());
        }
        return Err(anyhow!(
            "could not launch Unduhin by bundle id {BUNDLE_ID} or by path {path}"
        ));
    }

    let _ = host_exe;
    Err(anyhow!(
        "could not launch Unduhin by bundle id {BUNDLE_ID}, and no recorded \
         app path exists yet — open Unduhin once so it can register itself"
    ))
}

/// Unduhin ships on Windows and macOS. This arm keeps a contributor's
/// Linux build compiling; the bridge still works there if the app is
/// already running, it just cannot cold-start it.
#[cfg(not(any(windows, target_os = "macos")))]
fn spawn_app(_host_exe: &Path) -> Result<()> {
    Err(anyhow!(
        "launching the app is not implemented on this platform; start Unduhin first"
    ))
}
