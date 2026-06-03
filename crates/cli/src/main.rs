//! `unduhin` — the command-line interface.
//!
//! The `download`, `resume`, and `info` verbs drive the engine directly
//! and bypass the database. The `add`, `list`, `pause`, …, `daemon` verbs
//! go through `core::Core`, which means they share state across
//! invocations via SQLite.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::{Args, Parser, Subcommand};
use engine::{
    download, probe, resume_at, CancellationToken, DownloadOptions, ProgressEvent, RemoteInfo,
};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use tokio::sync::broadcast;
use unduhin_core::{
    AddDownload, CategorySelector, Core, DownloadFilter, DownloadSource, NewCategory, SettingValue,
    Status,
};
use url::Url;

#[derive(Parser, Debug)]
#[command(
    name = "unduhin",
    version,
    about = "Unduhin: a multi-segment HTTP download manager."
)]
struct Cli {
    /// Path to the SQLite database. Defaults to %LOCALAPPDATA%/unduhin/unduhin.db
    /// (or ~/.local/share/unduhin/unduhin.db elsewhere).
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    // Direct engine, no DB.
    /// Start a brand-new one-shot download bypassing the queue.
    Download(DownloadArgs),
    /// Resume a one-shot download from its `.unduhin-meta` sidecar.
    Resume(ResumeArgs),
    /// Probe a URL and print what the server says about it.
    Info(InfoArgs),

    // Through `core`.
    /// Add a URL to the persistent queue.
    Add(AddArgs),
    /// List downloads from the database.
    List(ListArgs),
    /// Pause a queued or active download.
    Pause(IdArg),
    /// Re-queue a paused or failed download.
    Continue(IdArg),
    /// Cancel a download (preserves the sidecar; use `remove` to delete).
    Cancel(IdArg),
    /// Re-queue a failed or cancelled download.
    Retry(IdArg),
    /// Remove a download from the database.
    Remove(RemoveArgs),
    /// Manage categories.
    Category {
        #[command(subcommand)]
        command: CategoryCmd,
    },
    /// Read or write configuration values.
    Settings {
        #[command(subcommand)]
        command: SettingsCmd,
    },
    /// Run the queue manager: pulls queued rows, downloads them, prints events.
    Daemon(DaemonArgs),
}

#[derive(Args, Debug)]
struct DownloadArgs {
    url: Url,
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,
    #[arg(short = 'n', long = "segments", default_value_t = 8)]
    segments: usize,
    #[arg(long)]
    resume: bool,
    #[arg(long, default_value_t = 15)]
    connect_timeout: u64,
    #[arg(long, default_value_t = 60)]
    read_timeout: u64,
}

#[derive(Args, Debug)]
struct ResumeArgs {
    meta_path: PathBuf,
    #[arg(long, default_value_t = 15)]
    connect_timeout: u64,
    #[arg(long, default_value_t = 60)]
    read_timeout: u64,
}

#[derive(Args, Debug)]
struct InfoArgs {
    url: Url,
    #[arg(long, default_value_t = 15)]
    connect_timeout: u64,
    #[arg(long, default_value_t = 60)]
    read_timeout: u64,
}

#[derive(Args, Debug)]
struct AddArgs {
    url: Url,
    #[arg(long)]
    category: Option<String>,
    #[arg(long, default_value_t = 0)]
    priority: i64,
    #[arg(long)]
    segments: Option<u32>,
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    #[arg(long)]
    filename: Option<String>,
}

#[derive(Args, Debug)]
struct ListArgs {
    /// Filter by status: queued|active|paused|completed|failed|cancelled.
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    category: Option<String>,
    /// Print JSON instead of a table.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct IdArg {
    id: i64,
}

#[derive(Args, Debug)]
struct RemoveArgs {
    id: i64,
    /// Also delete the downloaded file from disk.
    #[arg(long = "with-data")]
    with_data: bool,
}

#[derive(Subcommand, Debug)]
enum CategoryCmd {
    List,
    Add { name: String },
    Rm { name: String },
}

#[derive(Subcommand, Debug)]
enum SettingsCmd {
    Get { key: String },
    Set { key: String, value: String },
    List,
}

#[derive(Args, Debug)]
struct DaemonArgs {
    /// Exit after this many seconds of running. 0 means run until Ctrl-C.
    #[arg(long, default_value_t = 0)]
    duration: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let db = cli.db.clone();
    match cli.command {
        Command::Info(args) => run_info(args).await,
        Command::Download(args) => run_download(args).await,
        Command::Resume(args) => run_resume(args).await,
        Command::Add(args) => run_add(db, args).await,
        Command::List(args) => run_list(db, args).await,
        Command::Pause(args) => with_core(db, |c| async move { c.pause(args.id).await }).await,
        Command::Continue(args) => with_core(db, |c| async move { c.resume(args.id).await }).await,
        Command::Cancel(args) => with_core(db, |c| async move { c.cancel(args.id).await }).await,
        Command::Retry(args) => with_core(db, |c| async move { c.retry(args.id).await }).await,
        Command::Remove(args) => {
            let id = args.id;
            let with_data = args.with_data;
            with_core(db, move |c| async move { c.remove(id, with_data).await }).await
        }
        Command::Category { command } => run_category(db, command).await,
        Command::Settings { command } => run_settings(db, command).await,
        Command::Daemon(args) => run_daemon(db, args).await,
    }
}

// Direct-engine verbs

async fn run_info(args: InfoArgs) -> anyhow::Result<()> {
    let client = engine::http::build_client(
        Duration::from_secs(args.connect_timeout),
        Duration::from_secs(args.read_timeout),
        None,
        &[],
    )?;
    let info = probe(&client, &args.url).await?;
    println!("{}", serde_json::to_string_pretty(&InfoJson::from(&info))?);
    Ok(())
}

async fn run_download(args: DownloadArgs) -> anyhow::Result<()> {
    let client = engine::http::build_client(
        Duration::from_secs(args.connect_timeout),
        Duration::from_secs(args.read_timeout),
        None,
        &[],
    )?;
    let info = probe(&client, &args.url).await?;
    let output = resolve_output(&args, &info)?;
    let meta_path = engine::Meta::sidecar_path(&output);

    if args.resume && meta_path.exists() {
        eprintln!("resuming existing transfer at {}", output.display());
        return execute(true, move |cancel, tx| {
            resume_at(
                meta_path,
                engine::Backoff::default(),
                Duration::from_secs(args.connect_timeout),
                Duration::from_secs(args.read_timeout),
                None,
                Vec::new(),
                cancel,
                Some(tx),
            )
        })
        .await;
    }

    let mut opts = DownloadOptions::new(args.url.clone(), output.clone());
    opts.segments = args.segments;
    opts.connect_timeout = Duration::from_secs(args.connect_timeout);
    opts.read_timeout = Duration::from_secs(args.read_timeout);

    execute(false, move |cancel, tx| download(opts, cancel, Some(tx))).await
}

async fn run_resume(args: ResumeArgs) -> anyhow::Result<()> {
    execute(true, move |cancel, tx| {
        resume_at(
            args.meta_path,
            engine::Backoff::default(),
            Duration::from_secs(args.connect_timeout),
            Duration::from_secs(args.read_timeout),
            None,
            Vec::new(),
            cancel,
            Some(tx),
        )
    })
    .await
}

// Queue-backed verbs

async fn open_core(db: Option<PathBuf>) -> anyhow::Result<Core> {
    let path = match db {
        Some(p) => p,
        None => unduhin_core::default_db_path()
            .context("cannot determine a default DB path; pass --db <PATH>")?,
    };
    Ok(Core::open(&path).await?)
}

async fn with_core<F, Fut>(db: Option<PathBuf>, f: F) -> anyhow::Result<()>
where
    F: FnOnce(Core) -> Fut,
    Fut: std::future::Future<Output = unduhin_core::Result<()>>,
{
    let core = open_core(db).await?;
    f(core).await?;
    Ok(())
}

async fn run_add(db: Option<PathBuf>, args: AddArgs) -> anyhow::Result<()> {
    let core = open_core(db).await?;
    let id = core
        .add_download(AddDownload {
            url: args.url,
            filename: args.filename,
            output_path: args.output,
            category: args.category.map(CategorySelector::Name),
            priority: args.priority,
            segments: args.segments,
            media_info: None,
            headers: None,
            source: DownloadSource::Cli,
        })
        .await?;
    let record = core.get_download(id).await?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

async fn run_list(db: Option<PathBuf>, args: ListArgs) -> anyhow::Result<()> {
    let core = open_core(db).await?;
    let status = match args.status.as_deref() {
        None => None,
        Some(s) => Some(
            s.parse::<Status>()
                .map_err(|e| anyhow!("bad --status: {e}"))?,
        ),
    };
    let category_id = match args.category {
        Some(name) => Some(
            core.find_category_by_name(&name)
                .await?
                .with_context(|| format!("no category named {name:?}"))?
                .id,
        ),
        None => None,
    };
    let rows = core
        .list_downloads(DownloadFilter {
            status,
            category_id,
        })
        .await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("(no downloads)");
        return Ok(());
    }
    #[allow(clippy::print_literal)]
    {
        println!(
            "{:>4}  {:<10}  {:>12}  {:>12}  {:<8}  {}",
            "ID", "STATUS", "SIZE", "DONE", "PRIO", "FILENAME"
        );
    }
    for r in rows {
        println!(
            "{:>4}  {:<10}  {:>12}  {:>12}  {:<8}  {}",
            r.id,
            r.status,
            r.total_bytes.map(human_bytes).unwrap_or_else(|| "?".into()),
            human_bytes(r.downloaded_bytes),
            r.priority,
            r.filename,
        );
    }
    Ok(())
}

async fn run_category(db: Option<PathBuf>, cmd: CategoryCmd) -> anyhow::Result<()> {
    let core = open_core(db).await?;
    match cmd {
        CategoryCmd::List => {
            let cats = core.list_categories().await?;
            println!("{}", serde_json::to_string_pretty(&cats)?);
        }
        CategoryCmd::Add { name } => {
            let id = core
                .add_category(NewCategory {
                    name,
                    icon: None,
                    default_output_path: None,
                    extension_rules: vec![],
                })
                .await?;
            println!("added category id={id}");
        }
        CategoryCmd::Rm { name } => {
            let cat = core
                .find_category_by_name(&name)
                .await?
                .with_context(|| format!("no category named {name:?}"))?;
            core.remove_category(cat.id).await?;
            println!("removed category id={}", cat.id);
        }
    }
    Ok(())
}

async fn run_settings(db: Option<PathBuf>, cmd: SettingsCmd) -> anyhow::Result<()> {
    let core = open_core(db).await?;
    match cmd {
        SettingsCmd::Get { key } => match core.get_setting(&key).await? {
            None => println!("(unset)"),
            Some(v) => println!("{v}"),
        },
        SettingsCmd::Set { key, value } => {
            let parsed = unduhin_core::parse_user_value(&value);
            core.set_setting(&key, parsed).await?;
            println!("ok");
        }
        SettingsCmd::List => {
            let all = core.all_settings().await?;
            // Sort for deterministic output.
            let mut entries: Vec<_> = all.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let map: serde_json::Map<String, serde_json::Value> =
                entries.into_iter().map(|(k, v)| (k, v.0)).collect();
            println!("{}", serde_json::to_string_pretty(&map)?);
        }
    }
    Ok(())
}

async fn run_daemon(db: Option<PathBuf>, args: DaemonArgs) -> anyhow::Result<()> {
    let core = open_core(db).await?;
    let mut events = core.subscribe();
    core.start().await?;
    eprintln!("daemon running — Ctrl-C to stop");

    let stop = CancellationToken::new();
    let stop_clone = stop.clone();
    let ctrl_c = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\ninterrupt received — shutting down");
            stop_clone.cancel();
        }
    });
    let stop_dur = if args.duration > 0 {
        let st = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(args.duration)).await;
            st.cancel();
        });
        Some(())
    } else {
        None
    };
    let _ = stop_dur;

    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            recv = events.recv() => match recv {
                Ok(ev) => {
                    let line = serde_json::to_string(&ev).unwrap_or_else(|_| format!("{ev:?}"));
                    println!("{line}");
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("(lagged {n} events)");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    core.shutdown().await?;
    ctrl_c.abort();
    Ok(())
}

// Helpers (shared)

async fn execute<F, Fut>(_resuming: bool, build: F) -> anyhow::Result<()>
where
    F: FnOnce(CancellationToken, broadcast::Sender<ProgressEvent>) -> Fut,
    Fut: std::future::Future<Output = engine::Result<engine::DownloadSummary>>,
{
    let cancel = CancellationToken::new();
    let (tx, rx) = broadcast::channel::<ProgressEvent>(engine::DEFAULT_CHANNEL_CAPACITY);

    let bar_handle = spawn_progress_bar(rx);

    let ctrl_c_cancel = cancel.clone();
    let ctrl_c = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\ninterrupt received — flushing state…");
            ctrl_c_cancel.cancel();
        }
    });

    let fut = build(cancel.clone(), tx.clone());
    let result = fut.await;

    drop(tx);
    let _ = bar_handle.await;
    ctrl_c.abort();

    match result {
        Ok(summary) => {
            let out = SummaryJson::from(&summary);
            println!("{}", serde_json::to_string_pretty(&out)?);
            Ok(())
        }
        Err(e) => Err(anyhow!(e)),
    }
}

fn resolve_output(args: &DownloadArgs, info: &RemoteInfo) -> anyhow::Result<PathBuf> {
    if let Some(out) = &args.output {
        if out.is_dir() {
            let name = info
                .filename_hint
                .clone()
                .context("no filename in URL or Content-Disposition; pass -o <file>")?;
            return Ok(out.join(name));
        }
        return Ok(out.clone());
    }
    let name = info
        .filename_hint
        .clone()
        .context("no filename in URL or Content-Disposition; pass -o <file>")?;
    Ok(PathBuf::from(name))
}

fn spawn_progress_bar(mut rx: broadcast::Receiver<ProgressEvent>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let bar = ProgressBar::hidden();
        let mut total: Option<u64> = None;
        let mut resumed: u64 = 0;

        loop {
            match rx.recv().await {
                Ok(ProgressEvent::Started {
                    total: t,
                    segments,
                    resumed_bytes,
                }) => {
                    total = t;
                    resumed = resumed_bytes;
                    if let Some(t) = t {
                        bar.set_length(t);
                        bar.set_position(resumed_bytes);
                        bar.set_draw_target(indicatif::ProgressDrawTarget::stderr());
                        bar.set_style(
                            ProgressStyle::with_template(
                                "{bar:40.cyan/blue} {bytes}/{total_bytes} \
                                 ({bytes_per_sec}, {eta}) [{msg}]",
                            )
                            .unwrap()
                            .progress_chars("=>-"),
                        );
                        bar.set_message(format!("{segments} segments"));
                    } else {
                        bar.set_draw_target(indicatif::ProgressDrawTarget::stderr());
                        bar.set_style(
                            ProgressStyle::with_template(
                                "{spinner} {bytes} ({bytes_per_sec}) [{msg}]",
                            )
                            .unwrap(),
                        );
                        bar.set_message(format!("{segments} segments"));
                        bar.enable_steady_tick(Duration::from_millis(100));
                    }
                }
                Ok(ProgressEvent::Tick {
                    downloaded,
                    total: t,
                    ..
                }) => {
                    if let Some(t) = t {
                        bar.set_length(t);
                    }
                    bar.set_position(downloaded);
                }
                Ok(ProgressEvent::SegmentProgress { .. }) => {}
                Ok(ProgressEvent::FilenameLearned { hint }) => {
                    // The CLI is given an explicit output path, so the learned
                    // name doesn't rename anything here; surface it on the bar.
                    bar.set_message(hint);
                }
                Ok(ProgressEvent::Completed { bytes }) => {
                    bar.set_length(bytes.max(total.unwrap_or(bytes)));
                    bar.set_position(bytes);
                    bar.finish_with_message("done");
                    break;
                }
                Ok(ProgressEvent::Failed { error }) => {
                    bar.abandon_with_message(format!("failed: {error}"));
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    bar.finish_and_clear();
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
            }
        }
        let _ = resumed;
    })
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[idx])
    }
}

#[derive(Debug, Serialize)]
struct SummaryJson {
    url: String,
    output: PathBuf,
    bytes: u64,
    segments: usize,
    resumed: bool,
}

impl From<&engine::DownloadSummary> for SummaryJson {
    fn from(s: &engine::DownloadSummary) -> Self {
        Self {
            url: s.url.to_string(),
            output: s.output.clone(),
            bytes: s.bytes,
            segments: s.segments,
            resumed: s.resumed,
        }
    }
}

#[derive(Debug, Serialize)]
struct InfoJson {
    url: String,
    content_length: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
    accept_ranges: bool,
    filename_hint: Option<String>,
}

impl From<&RemoteInfo> for InfoJson {
    fn from(r: &RemoteInfo) -> Self {
        Self {
            url: r.url.to_string(),
            content_length: r.content_length,
            etag: r.etag.clone(),
            last_modified: r.last_modified.clone(),
            accept_ranges: r.accept_ranges,
            filename_hint: r.filename_hint.clone(),
        }
    }
}

// Make `_` for unused `SettingValue` import (we expose it via the API
// from `core::set_setting`; tests/CLI shouldn't need it directly).
#[allow(dead_code)]
fn _dummy(_v: SettingValue) {}
