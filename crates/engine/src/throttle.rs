//! Token-bucket byte-rate limiter.
//!
//! Shared (via `Arc`) across every in-flight segment and download so the sum
//! of their throughput is capped: each segment's write loop calls
//! [`TokenBucket::acquire`] for the bytes it is about to write, and blocks
//! until the bucket has refilled enough. A rate of `0` disables throttling —
//! `acquire` returns immediately.
//!
//! The `core` queue owns one bucket for the whole process, feeds a clone to
//! every worker via [`crate::DownloadOptions::rate_limiter`], and calls
//! [`TokenBucket::set_rate`] when the user changes `global_speed_limit_bps`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Refill-by-elapsed-time token bucket. Tokens are bytes; the rate is
/// bytes per second.
#[derive(Debug)]
pub struct TokenBucket {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    rate_bps: u64,
    capacity: u64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(rate_bps: u64) -> Arc<Self> {
        let capacity = rate_bps.max(1);
        Arc::new(Self {
            state: Mutex::new(State {
                rate_bps,
                capacity,
                tokens: capacity as f64,
                last: Instant::now(),
            }),
        })
    }

    /// Update the configured rate. `0` disables throttling — `acquire`
    /// returns immediately without sleeping.
    pub async fn set_rate(&self, rate_bps: u64) {
        let mut s = self.state.lock().await;
        s.rate_bps = rate_bps;
        s.capacity = rate_bps.max(1);
        s.tokens = s.tokens.min(s.capacity as f64);
        s.last = Instant::now();
    }

    pub async fn rate(&self) -> u64 {
        self.state.lock().await.rate_bps
    }

    /// Wait until enough tokens accumulate to cover `bytes`. If the rate
    /// is 0 the call is a no-op.
    pub async fn acquire(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        loop {
            let wait_for = {
                let mut s = self.state.lock().await;
                if s.rate_bps == 0 {
                    return;
                }
                let now = Instant::now();
                let elapsed = now.duration_since(s.last).as_secs_f64();
                s.tokens = (s.tokens + elapsed * s.rate_bps as f64).min(s.capacity as f64);
                s.last = now;
                if s.tokens >= bytes as f64 {
                    s.tokens -= bytes as f64;
                    return;
                }
                let deficit = bytes as f64 - s.tokens;
                Duration::from_secs_f64(deficit / s.rate_bps as f64)
            };
            tokio::time::sleep(wait_for).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn zero_rate_does_not_block() {
        let bucket = TokenBucket::new(0);
        let start = Instant::now();
        bucket.acquire(10_000).await;
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn rate_limits_throughput() {
        let bucket = TokenBucket::new(1_000);
        // Drain the initial capacity, then ask for another 500 bytes —
        // should take about 500ms.
        bucket.acquire(1_000).await;
        let start = Instant::now();
        bucket.acquire(500).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(400), "{elapsed:?}");
    }

    #[tokio::test]
    async fn set_rate_zero_unblocks() {
        let bucket = TokenBucket::new(100);
        bucket.acquire(100).await; // drain
        bucket.set_rate(0).await;
        let start = Instant::now();
        bucket.acquire(1_000_000).await; // would block for ages at 100 B/s
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
