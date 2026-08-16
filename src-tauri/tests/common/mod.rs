//! Shared helpers for the bridge integration tests.
//!
//! These tests are the only place the Unix-socket transport gets exercised
//! before it reaches a user, because the maintainer develops on Windows and
//! macOS builds only ever run in CI. Keeping the endpoint and connect logic
//! here means all three test binaries cover the same code path.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use unduhin_core::wire::transport::{self, ClientStream};

/// Distinguishes two endpoints minted in the same clock tick.
///
/// The pid and the timestamp alone are not enough. Every test in a binary
/// shares the pid, and libtest starts them concurrently, so two tests can
/// read the same `SystemTime::now()` — the clock's granularity, not the
/// nanosecond unit it is reported in, is what decides that. This counter
/// removes the question: within a process the suffix is unique by
/// construction.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// A per-test endpoint that cannot collide with a live app or with another
/// test running in the same binary.
///
/// The "same binary" half of that is load-bearing and was once broken. Two
/// colliding names do not fail loudly: the first server binds, the second
/// gets `EEXIST` (macOS `bind(2)` for the Unix domain; Linux says
/// `EADDRINUSE`) and dies, and the second test's client then connects to
/// the *first* test's listener. It hangs there until its own read timeout,
/// pointing at everything except the name.
///
/// On Unix this deliberately avoids `tempfile::tempdir()`. macOS puts
/// `TMPDIR` under `/var/folders/<xx>/<24 chars>/T/`, which is around fifty
/// bytes before a random subdirectory and a filename are added — close
/// enough to the 104-byte `sun_path` limit to fail unpredictably. A short
/// `/tmp` path is well clear of it.
pub fn unique_endpoint(tag: &str) -> String {
    let pid = std::process::id();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    // Still timestamped, which is what keeps a recycled pid from matching a
    // socket some earlier run left in `/tmp`.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    #[cfg(windows)]
    {
        format!(r"\\.\pipe\unduhin-{tag}-{pid}-{nanos}-{seq}")
    }
    #[cfg(unix)]
    {
        // Truncate the tag: the whole path must stay under the limit.
        let short: String = tag.chars().take(12).collect();
        format!("/tmp/udh-{short}-{pid}-{nanos}-{seq}.sock")
    }
}

/// The collision the counter exists to stop.
///
/// The tight loop is the whole point: it mints names far faster than any
/// clock advances, so a name built from pid and timestamp alone repeats
/// here on exactly the machines where it repeated in CI.
#[test]
fn endpoints_are_unique_within_a_process() {
    const N: usize = 10_000;
    let names: std::collections::HashSet<String> =
        (0..N).map(|_| unique_endpoint("dup-check")).collect();
    assert_eq!(names.len(), N, "unique_endpoint handed out a duplicate");
}

/// Guards the other half of the endpoint contract. An over-long path is
/// rejected by `bind` as a bare `InvalidInput` with no hint about length,
/// so keep the generated shape provably inside the limit.
#[cfg(unix)]
#[test]
fn endpoints_fit_in_sun_path() {
    let ep = unique_endpoint("a-tag-far-longer-than-the-truncation-point");
    transport::check_endpoint_len(&ep).expect("generated endpoint must fit in sun_path");
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
