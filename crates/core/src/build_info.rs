//! Build-time metadata embedded by `build.rs`.
//!
//! The Tauri shell exposes these on the About page. The values are baked
//! into the binary at compile time — no runtime cost — so a packaged
//! release knows exactly which commit produced it.

/// Semver from `Cargo.toml`. Mirrors `env!("CARGO_PKG_VERSION")`; lives
/// here so the Tauri layer can ask one source of truth.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 7-character git short hash, or `"unknown"` when the workspace wasn't
/// a git checkout at build time (vendored release tarballs).
pub const GIT_SHA: &str = env!("UNDUHIN_GIT_SHA");

/// ISO-ish UTC timestamp produced by `build.rs`, e.g. `2026-05-23 11:42 UTC`.
pub const BUILD_TIMESTAMP: &str = env!("UNDUHIN_BUILD_TIMESTAMP");

/// Bundle the three constants for the Tauri command response.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildInfo {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub build_timestamp: &'static str,
}

pub const fn build_info() -> BuildInfo {
    BuildInfo {
        version: VERSION,
        git_sha: GIT_SHA,
        build_timestamp: BUILD_TIMESTAMP,
    }
}
