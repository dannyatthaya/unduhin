// Embeds build-time metadata into the core crate so the About page can
// show the exact commit + build timestamp without runtime guessing.
//
//   UNDUHIN_GIT_SHA          7-char short hash, or "unknown" outside a repo
//   UNDUHIN_BUILD_TIMESTAMP  yyyy-mm-dd HH:MM UTC
//
// Re-run on every build so the timestamp stays honest; the git command is
// cheap and short-circuits when the workspace isn't a checkout.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let sha = git_short_sha().unwrap_or_else(|| "unknown".to_string());
    let ts = build_timestamp();
    println!("cargo:rustc-env=UNDUHIN_GIT_SHA={sha}");
    println!("cargo:rustc-env=UNDUHIN_BUILD_TIMESTAMP={ts}");
    println!("cargo:rerun-if-changed=build.rs");
    // No `cargo:rerun-if-changed=.git/HEAD` here on purpose: the workspace
    // is also vendored into release tarballs without a .git dir. The env
    // value falls back to "unknown" in that case.
}

fn git_short_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_timestamp() -> String {
    // Avoid pulling chrono into build-deps; format an ISO-like string by hand.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc(secs)
}

fn format_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let hms = secs % 86_400;
    let hour = hms / 3600;
    let minute = (hms % 3600) / 60;
    let (y, m, d) = days_to_date(days as i64);
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02} UTC")
}

fn days_to_date(days_since_epoch: i64) -> (i32, u32, u32) {
    // 1970-01-01 = day 0. Civil-from-days, adapted from Hinnant's algorithm.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
