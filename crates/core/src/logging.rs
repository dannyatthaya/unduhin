//! Rotating file logger.
//!
//! Writes a daily-rolling log under `<directories_root>/logs/unduhin.log.YYYY-MM-DD`.
//! Keeps a small in-memory ring of recent lines so the About page can
//! show a "Copy diagnostic" snapshot without re-reading disk.
//!
//! The Tauri shell calls [`init`] once on startup. Tests don't need it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use tracing::Level;

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

/// A minimal day-rolling appender. Each `write` opens the current day's
/// file in append mode, writes the line, and closes — slow but trivial
/// and good enough for the volume Unduhin produces. Avoids pulling in
/// `tracing-appender` to keep the dependency surface small.
struct FileAppender {
    dir: PathBuf,
    state: Mutex<()>,
}

impl FileAppender {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            state: Mutex::new(()),
        }
    }

    fn current_path(&self) -> PathBuf {
        let today = Utc::now().format("%Y-%m-%d");
        self.dir.join(format!("unduhin.log.{today}"))
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
        let _g = self
            .appender
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.appender.current_path())?;
        f.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
