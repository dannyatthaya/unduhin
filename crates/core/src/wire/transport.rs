//! Where the app and the native-messaging host meet.
//!
//! The bridge is a byte stream carrying the framed messages defined in
//! [`super::framing`]. Which kind of byte stream differs by platform —
//! a named pipe on Windows, a Unix domain socket elsewhere — and this
//! module is the only place that difference is spelled out. Everything
//! above it works against `AsyncRead + AsyncWrite` and needs no `cfg`.
//!
//! This lives in `unduhin-core` rather than in the Tauri shell because
//! three separate callers need to resolve the same endpoint and open the
//! same kind of stream: the app's `test_pipe_handoff` command, the
//! `unduhin-native-host` binary, and the integration tests. The host crate
//! cannot depend on the app crate, and a duplicated endpoint string that
//! drifts would break the bridge silently.

use std::io;

/// The well-known endpoint both ends connect to.
///
/// `UNDUHIN_PIPE_NAME` overrides it on every platform, which is how the
/// integration tests avoid colliding with a live app and how
/// `scripts/dev.ps1` runs a dev build beside an installed release.
///
/// On Windows this is a pipe name. Elsewhere it is an absolute socket
/// path, which brings a constraint the pipe name does not have — see
/// [`check_endpoint_len`].
pub fn endpoint() -> String {
    if let Ok(name) = std::env::var("UNDUHIN_PIPE_NAME") {
        if !name.is_empty() {
            return name;
        }
    }
    default_endpoint()
}

#[cfg(windows)]
fn default_endpoint() -> String {
    r"\\.\pipe\unduhin".to_string()
}

/// The socket lives in the app data root, beside the database and logs.
///
/// Not in `$TMPDIR`: macOS runs a periodic reaper over temporary
/// directories that would happily delete a live socket's inode. Not in
/// `/tmp` either, which is world-writable and so lets another user squat
/// the name in the window between our unlink and our bind. The data root
/// is single-user and already the home for every other piece of app state.
#[cfg(unix)]
fn default_endpoint() -> String {
    crate::directories_root()
        .map(|root| root.join("unduhin.sock"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/unduhin.sock"))
        .to_string_lossy()
        .into_owned()
}

/// The `sockaddr_un.sun_path` limit, including the trailing NUL. 104 on
/// macOS and the BSDs, 108 on Linux; the smaller value is the safe one to
/// enforce everywhere.
#[cfg(unix)]
pub const SUN_PATH_MAX: usize = 104;

/// Reject an over-long socket path with a diagnosis instead of leaving it
/// to `bind`.
///
/// This is not hypothetical: `UNDUHIN_DATA_ROOT` can point anywhere, and
/// the kernel's own answer is a bare `InvalidInput` with no hint that the
/// length is what went wrong.
#[cfg(unix)]
pub fn check_endpoint_len(path: &str) -> io::Result<()> {
    if path.len() >= SUN_PATH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path is {} bytes but the limit is {}: {path}",
                path.len(),
                SUN_PATH_MAX - 1
            ),
        ));
    }
    Ok(())
}

/// The client end of the bridge.
#[cfg(windows)]
pub type ClientStream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
pub type ClientStream = tokio::net::UnixStream;

/// One connect attempt, with no retry.
///
/// Callers own their own backoff because they want different schedules:
/// the native host waits out a cold app start, while `test_pipe_handoff`
/// only bridges the accept-boundary race.
pub async fn connect(endpoint: &str) -> io::Result<ClientStream> {
    #[cfg(windows)]
    {
        tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
    }
    #[cfg(unix)]
    {
        check_endpoint_len(endpoint)?;
        tokio::net::UnixStream::connect(endpoint).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_absolute_and_within_limits() {
        let ep = endpoint();
        assert!(!ep.is_empty());
        #[cfg(unix)]
        {
            assert!(ep.starts_with('/'), "socket path must be absolute: {ep}");
            check_endpoint_len(&ep).expect("default endpoint must fit in sun_path");
        }
        #[cfg(windows)]
        assert!(ep.starts_with(r"\\.\pipe\"), "must be a pipe name: {ep}");
    }

    #[cfg(unix)]
    #[test]
    fn over_long_socket_paths_are_rejected_with_a_useful_message() {
        let long = format!("/{}", "x".repeat(SUN_PATH_MAX));
        let err = check_endpoint_len(&long).expect_err("must reject");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        // The message has to name the limit; `bind` alone would only say
        // "invalid input".
        assert!(err.to_string().contains("limit"), "{err}");

        let ok = "/Users/someone/Library/Application Support/unduhin/unduhin.sock";
        assert!(ok.len() < SUN_PATH_MAX);
        check_endpoint_len(ok).expect("a realistic path must fit");
    }
}
