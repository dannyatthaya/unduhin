//! Shared helpers for the bridge integration tests.
//!
//! These tests are the only place the Unix-socket transport gets exercised
//! before it reaches a user, because the maintainer develops on Windows and
//! macOS builds only ever run in CI. Keeping the endpoint and connect logic
//! here means all three test binaries cover the same code path.

#![allow(dead_code)]

use std::time::Duration;

use unduhin_core::wire::transport::{self, ClientStream};

/// A per-test endpoint that cannot collide with a live app or with another
/// test running in the same binary.
///
/// On Unix this deliberately avoids `tempfile::tempdir()`. macOS puts
/// `TMPDIR` under `/var/folders/<xx>/<24 chars>/T/`, which is around fifty
/// bytes before a random subdirectory and a filename are added — close
/// enough to the 104-byte `sun_path` limit to fail unpredictably. A short
/// `/tmp` path is well clear of it.
pub fn unique_endpoint(tag: &str) -> String {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    #[cfg(windows)]
    {
        format!(r"\\.\pipe\unduhin-{tag}-{pid}-{nanos}")
    }
    #[cfg(unix)]
    {
        // Truncate the tag: the whole path must stay under the limit.
        let short: String = tag.chars().take(12).collect();
        format!("/tmp/udh-{short}-{pid}-{nanos}.sock")
    }
}

/// Connect to a server that may still be starting up.
///
/// Retries because both transports have a startup race: a named pipe
/// instance is not connectable until it is created, and a Unix socket is
/// not connectable until `bind` completes.
pub async fn connect_client(endpoint: &str) -> ClientStream {
    for _ in 0..80 {
        if let Ok(client) = transport::connect(endpoint).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("bridge server never became connectable at {endpoint}");
}
