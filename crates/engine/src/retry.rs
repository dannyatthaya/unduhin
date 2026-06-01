//! Retry classification and exponential backoff.

use std::time::Duration;

/// Classification of an attempt's outcome with respect to retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    /// Worth retrying after a backoff (network blip, 5xx, 408, 429).
    Transient,
    /// Stop. 4xx (except 408/429), 3xx without a final URL, malformed responses.
    Terminal,
}

/// Classify a raw HTTP status code.
pub fn classify_status(status: u16) -> RetryClass {
    match status {
        408 | 425 | 429 => RetryClass::Transient,
        500..=599 => RetryClass::Transient,
        400..=499 => RetryClass::Terminal,
        // 2xx and 3xx aren't errors at this level — but if the caller is asking,
        // treat anything unexpected as transient and let upper layers decide.
        _ => RetryClass::Transient,
    }
}

/// Classify a [`reqwest::Error`].
pub fn classify_reqwest(err: &reqwest::Error) -> RetryClass {
    if let Some(status) = err.status() {
        return classify_status(status.as_u16());
    }
    if err.is_timeout() || err.is_connect() || err.is_request() || err.is_body() || err.is_decode()
    {
        return RetryClass::Transient;
    }
    if err.is_builder() {
        return RetryClass::Terminal;
    }
    RetryClass::Transient
}

/// Exponential backoff schedule.
///
/// Delay for attempt `n` (1-indexed) is `min(base * 2^(n-1), cap)`.
/// Cancellation is the caller's responsibility — this is a pure function.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub base: Duration,
    pub cap: Duration,
    pub max_attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            cap: Duration::from_secs(30),
            max_attempts: 5,
        }
    }
}

impl Backoff {
    pub fn delay_for(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let shift = (attempt - 1).min(20);
        let raw = self.base.saturating_mul(1u32 << shift);
        raw.min(self.cap)
    }

    pub fn is_exhausted(&self, attempt: u32) -> bool {
        attempt >= self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_terminal_4xx() {
        assert_eq!(classify_status(400), RetryClass::Terminal);
        assert_eq!(classify_status(403), RetryClass::Terminal);
        assert_eq!(classify_status(404), RetryClass::Terminal);
    }

    #[test]
    fn classify_transient_5xx_and_throttling() {
        assert_eq!(classify_status(500), RetryClass::Transient);
        assert_eq!(classify_status(502), RetryClass::Transient);
        assert_eq!(classify_status(503), RetryClass::Transient);
        assert_eq!(classify_status(408), RetryClass::Transient);
        assert_eq!(classify_status(429), RetryClass::Transient);
    }

    #[test]
    fn backoff_grows_then_caps() {
        let b = Backoff {
            base: Duration::from_millis(100),
            cap: Duration::from_secs(1),
            max_attempts: 10,
        };
        assert_eq!(b.delay_for(0), Duration::ZERO);
        assert_eq!(b.delay_for(1), Duration::from_millis(100));
        assert_eq!(b.delay_for(2), Duration::from_millis(200));
        assert_eq!(b.delay_for(3), Duration::from_millis(400));
        assert_eq!(b.delay_for(4), Duration::from_millis(800));
        // Capped.
        assert_eq!(b.delay_for(5), Duration::from_secs(1));
        assert_eq!(b.delay_for(20), Duration::from_secs(1));
    }

    #[test]
    fn backoff_exhaustion() {
        let b = Backoff {
            max_attempts: 3,
            ..Backoff::default()
        };
        assert!(!b.is_exhausted(0));
        assert!(!b.is_exhausted(2));
        assert!(b.is_exhausted(3));
        assert!(b.is_exhausted(4));
    }
}
