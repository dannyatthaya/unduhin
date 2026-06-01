//! Prove the host's `pump_pipe_to_stdout` forwards an
//! unsolicited frame on the named pipe straight to the host's stdout.
//!
//! Setup:
//!   1. Spawn a pipe server on a unique name.
//!   2. Spawn the native host binary pointed at that pipe.
//!   3. Drive the host with a non-`Ping` frame on stdin so the
//!      lazy-connect path runs and the pipe→stdout pump comes alive.
//!   4. The fake server accepts the host's connection, reads the
//!      forwarded frame, writes an unsolicited frame back, and the
//!      test asserts that frame surfaces on the host's stdout.

#![cfg(windows)]

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ServerOptions;
use tokio::process::Command;
use tokio::time::timeout;

const HOST_EXE: &str = env!("CARGO_BIN_EXE_unduhin-native-host");

async fn read_frame_from_child<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame_to_child<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = (payload.len() as u32).to_le_bytes();
    writer.write_all(&len).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

#[tokio::test]
async fn unsolicited_pipe_frame_lands_in_host_stdout() {
    let pipe_name = format!(
        r"\\.\pipe\unduhin-pump-pipe-to-stdout-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    // Spawn the fake server that the host will connect to.
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .expect("create pipe server");

    // Spawn the native host bound to the fake pipe name.
    let mut child = Command::new(HOST_EXE)
        .env("UNDUHIN_PIPE_NAME", &pipe_name)
        .env("UNDUHIN_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn native host");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let mut stdout = child.stdout.take().expect("stdout pipe");

    // Drive the host with a non-Ping frame so the lazy connect runs
    // and `pump_pipe_to_stdout` comes alive. `status` is the cheapest
    // request that crosses the pipe (no real downloads needed).
    let stim = br#"{"type":"status"}"#;
    write_frame_to_child(&mut stdin, stim)
        .await
        .expect("write status");

    // Accept the host's connection on the fake server.
    server
        .connect()
        .await
        .expect("host failed to connect to fake server");

    // Drain the host's forwarded `status` frame from the pipe so the
    // host's stdin loop isn't blocked on a full pipe buffer.
    let len = {
        let mut len_buf = [0u8; 4];
        server
            .read_exact(&mut len_buf)
            .await
            .expect("server: read len");
        u32::from_le_bytes(len_buf) as usize
    };
    let mut body = vec![0u8; len];
    server
        .read_exact(&mut body)
        .await
        .expect("server: read body");
    assert!(body.starts_with(b"{\"type\":\"status\"}"));

    // Now write an unsolicited frame from server → host. The host's
    // pump_pipe_to_stdout task should hand it straight to stdout.
    let unsolicited = br#"{"type":"settingsChanged","full":{"enabled":true,"nativeHostName":"com.unduhin.host","minSizeMb":7,"extensionAllowlist":[],"extensionBlocklist":["html","pdf","txt","json"],"blockedHosts":[],"alwaysInterceptHosts":[],"detectHls":true,"detectDash":true,"verboseLogging":false,"mode":"ask-first","installContextMenu":true,"hideShelf":true,"forwardCookies":true,"fileTypes":[]}}"#;
    let len = (unsolicited.len() as u32).to_le_bytes();
    server.write_all(&len).await.expect("server: write len");
    server
        .write_all(unsolicited)
        .await
        .expect("server: write body");
    server.flush().await.expect("server: flush");

    // Assert the frame surfaces on the host's stdout intact.
    let frame = timeout(Duration::from_secs(3), read_frame_from_child(&mut stdout))
        .await
        .expect("unsolicited frame timed out")
        .expect("read stdout frame");
    assert_eq!(frame, unsolicited.as_slice());

    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("host did not exit")
        .expect("wait failed");
    assert!(status.success(), "host exited with {status:?}");
}
