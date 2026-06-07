//! Global speed limiter.
//!
//! The token-bucket implementation now lives in the `engine` crate
//! ([`engine::TokenBucket`]) so the engine's transfer loop can consume it
//! directly — the engine cannot depend on `core`. This module re-exports it so
//! existing `core::speed::TokenBucket` / `core::TokenBucket` paths keep
//! resolving. The `core` queue owns one process-wide bucket, feeds a clone to
//! every HTTP worker, and updates its rate from `global_speed_limit_bps`.

pub use engine::TokenBucket;
