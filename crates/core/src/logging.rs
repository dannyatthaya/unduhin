//! Rotating file logger.
//!
//! Writes a daily-rolling log under `<directories_root>/logs/unduhin.log.YYYY-MM-DD`
//! and keeps the last [`RETAIN_DAYS`] days of them.
//! Keeps a small in-memory ring of recent lines so the About page can
//! show a "Copy diagnostic" snapshot without re-reading disk.
//!
//! The Tauri shell calls [`init`] once on startup. Tests don't need it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{NaiveDate, Utc};
use tracing::Level;

/// Days of daily log files kept. Older ones are deleted at start-up and at
/// each day rollover: logs carry URLs and file names, so they should not
/// pile up forever, and nothing needs more than a couple of weeks of them.
pub const RETAIN_DAYS: i64 = 14;

const LOG_PREFIX: &str = "unduhin.log.";

/// Folder that contains the daily-rolling log files. Created on first
/// write. Returns `None` if the directory root is unavailable (no
/// `%LOCALAPPDATA%` or `$HOME`).
pub fn logs_dir() -> Option<PathBuf> {
    crate::directories_root().map(|d| d.join("logs"))
}

/// Initialize tracing with a file appender + a stderr layer.
///
/// Returns the resolved log directory on success. Subsequent calls are
/// no-ops — initialization is global.
///
/// Privacy: the file logger captures every `tracing` span/event at INFO
/// or above. URLs, filenames, and the like reach the file the same way
/// they reach stderr today. Users can scrub the directory at any time.
pub fn init() -> io::Result<Option<PathBuf>> {
    static GUARD: OnceLock<Option<PathBuf>> = OnceLock::new();
    if let Some(path) = GUARD.get() {
        return Ok(path.clone());
    }

    let dir = logs_dir();
    if let Some(ref d) = dir {
        fs::create_dir_all(d)?;
        prune_old_logs(d, Utc::now().date_naive(), RETAIN_DAYS);
    }

    let appender = dir.as_ref().map(|d| FileAppender::new(d.clone()));

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(io::stderr);
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer);

    let result = if let Some(app) = appender {
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(app);
        registry.with(file_layer).try_init()
    } else {
        registry.try_init()
    };

    if let Err(e) = result {
        // Already initialized by a test or a sibling — that's fine.
        tracing::debug!(error = %e, "tracing was already initialized");
    }

    let _ = GUARD.set(dir.clone());
    Ok(dir)
}

/// Format a log line at INFO level so callers don't need to import
/// tracing macros just to push diagnostic info.
#[inline]
pub fn record(level: Level, msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    match level {
        Level::ERROR => tracing::error!(target: "unduhin", "{}", msg),
        Level::WARN => tracing::warn!(target: "unduhin", "{}", msg),
        Level::INFO => tracing::info!(target: "unduhin", "{}", msg),
        Level::DEBUG => tracing::debug!(target: "unduhin", "{}", msg),
        Level::TRACE => tracing::trace!(target: "unduhin", "{}", msg),
    }
}

/// Delete this logger's daily files dated more than `keep_days` before
/// `today`. Only names of the form `unduhin.log.YYYY-MM-DD` are touched;
/// anything else in the folder is left alone. Best-effort.
fn prune_old_logs(dir: &std::path::Path, today: NaiveDate, keep_days: i64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(date) = name
            .to_str()
            .and_then(|n| n.strip_prefix(LOG_PREFIX))
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if (today - date).num_days() > keep_days {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A minimal day-rolling appender. The current day's file stays open
/// between lines — opening and closing it for every line was a blocking
/// file-system round trip on whatever async worker thread logged — and is
/// swapped for the next day's on the first line after midnight (UTC), which
/// also prunes old days. Avoids pulling in `tracing-appender` to keep the
/// dependency surface small.
struct FileAppender {
    dir: PathBuf,
    /// The open file and the day it belongs to.
    state: Mutex<Option<(NaiveDate, fs::File)>>,
}

impl FileAppender {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            state: Mutex::new(None),
        }
    }

    fn path_for(&self, day: NaiveDate) -> PathBuf {
        self.dir
            .join(format!("{LOG_PREFIX}{}", day.format("%Y-%m-%d")))
    }

    fn write_line(&self, buf: &[u8], today: NaiveDate) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.as_ref().map(|(day, _)| *day) != Some(today) {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path_for(today))?;
            if state.is_some() {
                prune_old_logs(&self.dir, today, RETAIN_DAYS);
            }
            *state = Some((today, file));
        }
        let (_, file) = state.as_mut().expect("opened above");
        file.write_all(buf)
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileAppender {
    type Writer = FileAppenderWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        FileAppenderWriter { appender: self }
    }
}

struct FileAppenderWriter<'a> {
    appender: &'a FileAppender,
}

impl<'a> Write for FileAppenderWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.appender.write_line(buf, Utc::now().date_naive())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn prune_keeps_recent_days_and_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "unduhin.log.2026-09-25",
            "unduhin.log.2026-09-11", // exactly 14 days back: kept
            "unduhin.log.2026-09-10", // 15 days back: deleted
            "unduhin.log.2025-01-01",
            "unduhin.log.not-a-date",
            "notes.txt",
        ] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }

        prune_old_logs(dir.path(), day("2026-09-25"), 14);

        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "notes.txt",
                "unduhin.log.2026-09-11",
                "unduhin.log.2026-09-25",
                "unduhin.log.not-a-date",
            ]
        );
    }

    #[test]
    fn appender_rolls_over_to_a_new_file_each_day() {
        let dir = tempfile::tempdir().unwrap();
        let app = FileAppender::new(dir.path().to_path_buf());
        app.write_line(b"one\n", day("2026-09-24")).unwrap();
        app.write_line(b"two\n", day("2026-09-24")).unwrap();
        app.write_line(b"three\n", day("2026-09-25")).unwrap();

        let read =
            |d: &str| fs::read_to_string(dir.path().join(format!("unduhin.log.{d}"))).unwrap();
        assert_eq!(read("2026-09-24"), "one\ntwo\n");
        assert_eq!(read("2026-09-25"), "three\n");
    }
}
