//! Spawn the actual `unduhin-native-host.exe` binary, write a
//! framed `ping`, and assert a framed `pong` comes back on stdout
//! within one second. Locks the most basic guarantee: the host
//! responds on its own without needing the long-running app
//! (`Ping` is handled locally).

#![cfg(windows)]

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
async fn ping_pong_over_stdio() {
    // Point the host at a stub pipe name so any accidental
    // non-ping forwarding can't accidentally hit a real running
    // Unduhin instance.
    let mut child = Command::new(HOST_EXE)
        .env(
            "UNDUHIN_PIPE_NAME",
            format!(r"\\.\pipe\unduhin-process-smoke-{}", std::process::id()),
        )
        .env("UNDUHIN_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn native host");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let mut stdout = child.stdout.take().expect("stdout pipe");

    // Send `{"type":"ping"}`.
    let ping = br#"{"type":"ping"}"#;
    write_frame_to_child(&mut stdin, ping)
        .await
        .expect("write ping");

    let frame = timeout(Duration::from_secs(2), read_frame_from_child(&mut stdout))
        .await
        .expect("pong timed out")
        .expect("pong read");
    let parsed: serde_json::Value = serde_json::from_slice(&frame).expect("json");
    assert_eq!(parsed["type"], "pong", "expected pong, got {parsed:?}");

    // Closing stdin signals EOF — the host should exit cleanly.
    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("child timed out exiting")
        .expect("wait failed");
    assert!(status.success(), "host exited with {status:?}");
}
