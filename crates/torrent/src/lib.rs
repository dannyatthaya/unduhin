//! Unduhin torrent backend — a thin facade over [`librqbit`].
//!
//! This crate is the single boundary where BitTorrent lives. `core` depends on
//! it but never sees a librqbit type: every public surface here
//! ([`TorrentEngine`], [`TorrentConfig`], [`TorrentInput`], [`TorrentMetadata`],
//! [`TorrentRunSummary`], [`TorrentError`]) is our own. It depends on `engine`
//! for exactly one type — [`engine::ProgressEvent`] — so torrent downloads flow
//! through `core`'s existing progress pump (design §3.D) instead of a second
//! event path.
//!
//! ## What the facade does
//!
//! - [`TorrentEngine::new`] builds a process-wide `Arc<Session>` (one DHT, one
//!   listen socket) with fastresume + JSON persistence.
//! - [`TorrentEngine::fetch_metadata`] probes a torrent's file list WITHOUT
//!   downloading (`list_only`), wrapped in a timeout (librqbit has no internal
//!   one — design §5.5).
//! - [`TorrentEngine::run`] downloads to completion honoring a
//!   [`CancellationToken`], emitting [`engine::ProgressEvent`]s, then STOPS
//!   seeding (librqbit keeps uploading after completion — design §5.1).
//!
//! See `.claude/torrent-support-design.md` §3.D and `.claude/PHASE_2a.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use std::num::NonZeroU32;

use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions,
    SessionPersistenceConfig, TorrentStatsState,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub use tokio_util::sync::CancellationToken as TorrentCancellationToken;

/// Default ceiling for resolving torrent metadata (magnet → file list). With
/// DHT on but no responsive peers, librqbit awaits forever (design §5.5), so
/// every metadata-bearing add must be bounded. 60s mirrors the spike default.
/// Magnet metadata can be slow to resolve when the torrent's trackers are dead
/// (common for public magnets) and it falls back to DHT alone — DHT lookups +
/// the `ut_metadata` exchange routinely take longer than a minute. 120s gives a
/// DHT-only magnet a fair chance; a `.torrent` (metadata in-file) never waits.
pub const DEFAULT_METADATA_TIMEOUT: Duration = Duration::from_secs(120);

/// How often [`TorrentEngine::run`] samples `stats()` to emit progress.
const STATS_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Port range handed to librqbit when the user hasn't pinned a listen port
/// (`torrent_listen_port == 0`). It must be a range of *real* ports: librqbit
/// announces the port value it bound *from the range*, so a `0` is announced
/// literally as `port=0` — which trackers reject ("Port can't be 0") and peers
/// can't dial. High/dynamic ports also dodge classic BitTorrent-port
/// (6881-6889) throttling. See [`TorrentEngine::new`].
const DEFAULT_LISTEN_PORTS: std::ops::Range<u16> = 49152..49172;

/// Convert a librqbit `Speed` to bytes/sec. Despite the `mbps` name, librqbit's
/// value is MiB/s — `librqbit-core` computes it as `bytes_per_sec / 1024 / 1024`
/// and `Display`s it as `"{:.2} MiB/s"` — so the multiplier is 1 MiB, not 1 Mbit.
fn mbps_to_bytes_per_sec(mbps: f64) -> u64 {
    if !mbps.is_finite() || mbps <= 0.0 {
        return 0;
    }
    (mbps * 1_048_576.0).round() as u64
}

/// Estimate time-to-completion from remaining bytes and current speed. librqbit
/// exposes an ETA, but its `DurationWithHumanReadable` wraps a private
/// `Duration`, so we recompute it ourselves (same semantics as the engine's
/// `eta` helper). `None` when speed is zero/non-finite or nothing remains.
fn eta_from(remaining: u64, speed_bps: f64) -> Option<Duration> {
    if !speed_bps.is_finite() || speed_bps <= 0.0 || remaining == 0 {
        return None;
    }
    let secs = remaining as f64 / speed_bps;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(secs.min(u64::MAX as f64 / 2.0)))
}

/// Errors the facade surfaces to `core`. Mapped to [`engine::EngineError`] at
/// the boundary (see [`From<TorrentError> for engine::EngineError`]) so the
/// worker's existing `match result` tail needs no new arms.
#[derive(Debug, thiserror::Error)]
pub enum TorrentError {
    /// Metadata resolution or the initial add exceeded the timeout — usually
    /// "no peers / DHT off" for a trackerless magnet (design §5.5).
    #[error("couldn't fetch torrent metadata (no peers/DHT) after {0:?}")]
    Timeout(Duration),

    /// The caller's [`CancellationToken`] fired (pause / shutdown). Maps to
    /// [`engine::EngineError::Cancelled`].
    #[error("torrent download cancelled")]
    Cancelled,

    /// librqbit reported a torrent-level error (disk write failure, corrupt
    /// metadata, …). For sparse-file disk failures this surfaces mid-run as the
    /// torrent transitions to the error state (design §5.6).
    #[error("torrent backend error: {0}")]
    Backend(String),
}

impl TorrentError {
    fn backend(e: impl std::fmt::Display) -> Self {
        TorrentError::Backend(e.to_string())
    }
}

/// Map facade errors onto the shared engine error so `run_torrent` (P2b) can
/// reuse the worker's uniform completion tail — the same trick `run_ytdlp` uses.
impl From<TorrentError> for engine::EngineError {
    fn from(e: TorrentError) -> Self {
        match e {
            TorrentError::Cancelled => engine::EngineError::Cancelled,
            other => engine::EngineError::other(other.to_string()),
        }
    }
}

/// Session-level configuration. Mirrors the `torrent_*` settings keys seeded by
/// the migration (design §3.G); `core` builds this from the DB on first use.
#[derive(Debug, Clone)]
pub struct TorrentConfig {
    /// Default output folder for content (a per-download dir may override it at
    /// `run` time).
    pub download_dir: PathBuf,
    /// Where librqbit persists its session JSON + fastresume state. The torrent
    /// analogue of the engine's `.unduhin-meta` sidecar (design §3.D); keep it
    /// distinct from the content dir so resume state survives content moves.
    pub state_dir: PathBuf,
    /// Listen port; `0` = OS-assigned ephemeral.
    pub listen_port: u16,
    /// DHT must stay on to resolve trackerless magnets (design §5.5/§3.G).
    pub enable_dht: bool,
    /// UPnP port-mapping for inbound peers.
    pub enable_upnp: bool,
    /// Seed target as a ratio in thousandths (uploaded:downloaded). `0`
    /// means "stop at 100 %, no seeding" — the torrent is forgotten the
    /// moment it completes (the long-standing default). A positive value
    /// keeps the torrent uploading after completion until
    /// `uploaded / downloaded * 1000 >= seed_ratio_milli`, then forgets it
    /// (design §3.G `torrent_seed_ratio_milli`).
    pub seed_ratio_milli: u32,
    /// Global download speed cap in bytes/sec applied to the whole librqbit
    /// session (all torrents share it). `0` = unlimited. Mirrors the HTTP
    /// engine's `global_speed_limit_bps`; updated live via
    /// [`TorrentEngine::set_download_limit`].
    pub download_limit_bps: u64,
}

impl TorrentConfig {
    /// Sensible defaults matching the seeded settings (design §3.G): DHT + UPnP
    /// on, OS-assigned port, no seeding. Caller must still set `download_dir` /
    /// `state_dir`.
    pub fn new(download_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            download_dir,
            state_dir,
            listen_port: 0,
            enable_dht: true,
            enable_upnp: true,
            seed_ratio_milli: 0,
            download_limit_bps: 0,
        }
    }
}

/// Build a librqbit [`LimitsConfig`] from a bytes/sec download cap. `0` (or a
/// value that doesn't fit `u32`, clamped) maps to "no download limit"; upload
/// is left uncapped (seeding is governed by `seed_ratio_milli`, not bandwidth).
fn limits_from_bps(download_bps: u64) -> LimitsConfig {
    LimitsConfig {
        download_bps: NonZeroU32::new(download_bps.min(u32::MAX as u64) as u32),
        upload_bps: None,
    }
}

/// What to add. Hides librqbit's `AddTorrent` (and its lifetime) from `core`.
#[derive(Debug, Clone)]
pub enum TorrentInput {
    /// `magnet:?xt=urn:btih:…` URI.
    Magnet(String),
    /// Path to a `.torrent` file already copied into the managed dir.
    TorrentFile(PathBuf),
    /// Bare BitTorrent v1 infohash (40-hex). Resolved via DHT.
    InfoHash(String),
}

impl TorrentInput {
    /// Build the librqbit add request. `.torrent` files are read into bytes here
    /// (librqbit's `from_local_filename` does the same) so the facade owns the
    /// only filesystem read.
    fn to_add_torrent(&self) -> Result<AddTorrent<'static>, TorrentError> {
        match self {
            TorrentInput::Magnet(uri) => Ok(AddTorrent::from_url(uri.clone())),
            TorrentInput::InfoHash(hash) => {
                // librqbit accepts a bare 40-hex magnet target as a URL form.
                Ok(AddTorrent::from_url(format!("magnet:?xt=urn:btih:{hash}")))
            }
            TorrentInput::TorrentFile(path) => {
                let bytes = std::fs::read(path).map_err(|e| {
                    TorrentError::Backend(format!(
                        "reading .torrent file {}: {e}",
                        path.display()
                    ))
                })?;
                Ok(AddTorrent::from_bytes(bytes))
            }
        }
    }
}

/// One file inside a torrent, as surfaced to the add-time file picker.
#[derive(Debug, Clone)]
pub struct TorrentFileEntry {
    /// Relative path within the torrent (forward-slash joined).
    pub path: String,
    /// File length in bytes.
    pub length: u64,
}

/// Metadata resolved without downloading — backs the add-time file picker.
///
/// NOTE: librqbit also exports a type named `TorrentMetadata`; this is our own
/// facade type and intentionally shadows it. We never re-export theirs.
#[derive(Debug, Clone)]
pub struct TorrentMetadata {
    /// Lowercase hex BTv1 infohash — the stable de-dup key (design §3.B).
    pub info_hash: String,
    /// Display name (`dn=` for a magnet, else the torrent's `name`).
    pub name: String,
    /// Every file in the torrent, in torrent order (index == position).
    pub files: Vec<TorrentFileEntry>,
}

impl TorrentMetadata {
    /// Total bytes across all files.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.length).sum()
    }
}

/// Result of a completed [`TorrentEngine::run`]. P2b maps this onto an
/// `engine::DownloadSummary` for the worker's completion tail.
#[derive(Debug, Clone)]
pub struct TorrentRunSummary {
    /// Content root on disk — the per-download directory the caller passed to
    /// [`TorrentEngine::run`], into which librqbit wrote all (selected) files.
    /// Single- and multi-file torrents both land under this one directory
    /// (we pass an explicit `output_folder`, so no per-torrent name subfolder is
    /// added), so `core`'s directory-aware `remove` (Q4) can `remove_dir_all` it.
    pub output_root: PathBuf,
    /// Verified bytes written.
    pub bytes: u64,
    /// Whether fastresume picked up an existing partial.
    pub resumed: bool,
    /// Metadata learned during the run (name + file list).
    pub metadata: TorrentMetadata,
}

/// Process-wide librqbit session, wrapped so `core` never touches a librqbit
/// type. Construct once (lazily) and share via `Arc` — it owns one DHT, one
/// listen socket, one peer budget (design §3.D session-lifecycle note).
pub struct TorrentEngine {
    session: Arc<Session>,
    download_dir: PathBuf,
    /// Seed-until ratio (thousandths); `0` = forget at 100 %. Read once from
    /// the session config — like `listen_port` / DHT / UPnP, this is fixed for
    /// the life of the process-wide session, so a change takes effect on the
    /// next app start.
    seed_ratio_milli: u32,
}

impl TorrentEngine {
    /// Build the session. One DHT / listen socket / peer budget for the whole
    /// process — call this lazily on the first torrent (design §3.D).
    pub async fn new(cfg: TorrentConfig) -> Result<Self, TorrentError> {
        // Persist session JSON + fastresume into the managed state dir so resume
        // survives a relaunch (design §3.D). Create it up front; librqbit will
        // also create per-torrent content under `download_dir`.
        if let Err(e) = tokio::fs::create_dir_all(&cfg.state_dir).await {
            return Err(TorrentError::Backend(format!(
                "creating torrent state dir {}: {e}",
                cfg.state_dir.display()
            )));
        }
        if let Err(e) = tokio::fs::create_dir_all(&cfg.download_dir).await {
            return Err(TorrentError::Backend(format!(
                "creating torrent download dir {}: {e}",
                cfg.download_dir.display()
            )));
        }

        // librqbit's listener returns the port value it *tried from the range*,
        // not the OS-assigned one — so `0..1` binds an ephemeral socket yet
        // announces `port=0`, which trackers reject and peers can't dial. When
        // the user hasn't pinned a port (`listen_port == 0`), hand it a real
        // range to choose a free port from; otherwise bind exactly that port.
        let listen_port_range = if cfg.listen_port == 0 {
            DEFAULT_LISTEN_PORTS
        } else {
            cfg.listen_port..cfg.listen_port.saturating_add(1)
        };

        let opts = SessionOptions {
            disable_dht: !cfg.enable_dht,
            enable_upnp_port_forwarding: cfg.enable_upnp,
            listen_port_range: Some(listen_port_range),
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(cfg.state_dir.clone()),
            }),
            // Global download speed cap. librqbit applies this across the whole
            // session, so it bounds the sum of all active torrents.
            ratelimits: limits_from_bps(cfg.download_limit_bps),
            ..Default::default()
        };

        let session = Session::new_with_opts(cfg.download_dir.clone(), opts)
            .await
            .map_err(|e| TorrentError::Backend(format!("starting torrent session: {e}")))?;

        Ok(Self {
            session,
            download_dir: cfg.download_dir,
            seed_ratio_milli: cfg.seed_ratio_milli,
        })
    }

    /// Probe the file list WITHOUT downloading (`list_only`) — backs the
    /// add-time picker. Bounded by [`DEFAULT_METADATA_TIMEOUT`]; honors `cancel`.
    pub async fn fetch_metadata(
        &self,
        input: &TorrentInput,
        cancel: CancellationToken,
    ) -> Result<TorrentMetadata, TorrentError> {
        self.fetch_metadata_with_timeout(input, cancel, DEFAULT_METADATA_TIMEOUT)
            .await
    }

    /// Remove a torrent from the live session by its hex `info_hash`, so a later
    /// re-add starts FRESH. Without this, librqbit keeps the torrent managed in
    /// memory and `add_torrent` returns `AlreadyManaged` — resuming from where
    /// it left off (e.g. 50%) even after the app deleted the row and files.
    /// `delete_files` also deletes the on-disk content via librqbit; either way
    /// librqbit drops its fastresume (`.bitv`) and session.json entry. A no-op
    /// if the torrent isn't currently managed.
    pub async fn forget(&self, info_hash: &str, delete_files: bool) -> Result<(), TorrentError> {
        let id = librqbit::api::TorrentIdOrHash::parse(info_hash)
            .map_err(|e| TorrentError::Backend(format!("invalid info_hash {info_hash:?}: {e}")))?;
        if let Err(e) = self.session.delete(id, delete_files).await {
            // Not-managed (already gone) is fine — this is best-effort cleanup.
            tracing::debug!(info_hash, error = %e, "torrent: forget found nothing to delete");
        }
        Ok(())
    }

    /// As [`fetch_metadata`](Self::fetch_metadata) but with an explicit timeout
    /// (the integration test uses a longer one).
    pub async fn fetch_metadata_with_timeout(
        &self,
        input: &TorrentInput,
        cancel: CancellationToken,
        timeout: Duration,
    ) -> Result<TorrentMetadata, TorrentError> {
        let add = input.to_add_torrent()?;
        // Resolve metadata via a REAL add that downloads NOTHING (`only_files`
        // = []), NOT `list_only`. `list_only` makes librqbit announce port=0,
        // which trackers reject (`announce = !paused && !list_only`) — leaving
        // the probe DHT-ONLY and fragile for tracker-based magnets (a magnet may
        // resolve only via its trackers). A real add announces the real listen
        // port, so metadata resolves via the torrent's TRACKERS as well as DHT.
        // We forget the probe torrent once we've read its file list.
        let add_fut = self.session.add_torrent(
            add,
            Some(AddTorrentOptions {
                only_files: Some(Vec::new()),
                output_folder: Some(self.download_dir.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        );

        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(TorrentError::Cancelled),
            r = tokio::time::timeout(timeout, add_fut) => {
                r.map_err(|_| TorrentError::Timeout(timeout))?
                    .map_err(TorrentError::backend)?
            }
        };

        // `Added` is OUR probe — forget it after reading metadata. `AlreadyManaged`
        // means the user is already downloading this torrent: read its metadata
        // but NEVER disturb (forget) it.
        let (probe_added, handle) = match resp {
            AddTorrentResponse::Added(_, h) => (true, h),
            AddTorrentResponse::AlreadyManaged(_, h) => (false, h),
            AddTorrentResponse::ListOnly(_) => {
                return Err(TorrentError::Backend(
                    "metadata probe unexpectedly returned list_only".into(),
                ));
            }
        };

        let info_hash = handle.info_hash().as_string();
        let meta = self.snapshot_metadata(&handle, &info_hash);
        if probe_added {
            let _ = self
                .session
                .delete(handle.id().into(), /* delete_files = */ false)
                .await;
        }
        Ok(meta)
    }

    /// Run to completion, honoring `cancel`, emitting [`engine::ProgressEvent`]s
    /// on `tx`. On completion the torrent is STOPPED so it does not keep seeding
    /// (design §5.1 — librqbit does not stop on its own), UNLESS the session's
    /// `seed_ratio_milli` is positive, in which case it seeds up to that ratio
    /// first (see [`seed_to_ratio`](Self::seed_to_ratio)).
    ///
    /// `download_dir` overrides the session default for this torrent (per-download
    /// content folder). `only_files` selects a subset (`None` = all).
    pub async fn run(
        &self,
        input: TorrentInput,
        download_dir: PathBuf,
        only_files: Option<Vec<usize>>,
        cancel: CancellationToken,
        tx: Option<broadcast::Sender<engine::ProgressEvent>>,
    ) -> Result<TorrentRunSummary, TorrentError> {
        let add = input.to_add_torrent()?;

        // The add itself can block on metadata resolution for a magnet; bound it
        // the same way the probe is bounded (design §5.5) and honor cancel.
        let add_fut = self.session.add_torrent(
            add,
            Some(AddTorrentOptions {
                only_files,
                output_folder: Some(download_dir.to_string_lossy().into_owned()),
                // Allow resuming on top of existing partial content.
                overwrite: true,
                ..Default::default()
            }),
        );

        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(TorrentError::Cancelled),
            r = tokio::time::timeout(DEFAULT_METADATA_TIMEOUT, add_fut) => {
                r.map_err(|_| TorrentError::Timeout(DEFAULT_METADATA_TIMEOUT))?
                    .map_err(TorrentError::backend)?
            }
        };

        // Q7: a duplicate add returns AlreadyManaged — same handle, no second
        // copy. Treat Added and AlreadyManaged identically; both yield a handle.
        let (resumed, handle) = match resp {
            AddTorrentResponse::Added(_, h) => (false, h),
            AddTorrentResponse::AlreadyManaged(_, h) => (true, h),
            AddTorrentResponse::ListOnly(_) => {
                return Err(TorrentError::Backend(
                    "run add returned list_only (bug: list_only was not requested)".into(),
                ));
            }
        };

        let torrent_id = handle.id();
        let info_hash = handle.info_hash().as_string();

        // A torrent restored from librqbit's session persistence comes back
        // PAUSED (`is_paused:true` in session.json): librqbit won't connect to
        // peers or download while paused — it just sits "connecting to swarm".
        // Resume it before we wait on completion. Guarded on the paused state so
        // a fresh add (already live) is never double-started.
        if matches!(handle.stats().state, TorrentStatsState::Paused) {
            if let Err(e) = self.session.unpause(&handle).await {
                tracing::warn!(
                    info_hash = %info_hash,
                    error = %e,
                    "torrent: failed to resume a restored-paused torrent"
                );
            }
        }

        // Emit `Started` from the first stats snapshot that knows the total.
        let mut started_emitted = false;
        // Per-file byte lengths, cached once metadata resolves. `stats.file_progress`
        // gives downloaded bytes per file index; pairing it with these lengths
        // yields the `FileProgress { index, downloaded, total }` events (design
        // §3.C). `None` until metadata is available (a magnet pre-resolution).
        let mut file_lengths: Option<Vec<u64>> = None;
        // `FilenameLearned` is emitted at most twice: once for the provisional
        // magnet `dn=` name (if any, before metadata), then once more for the
        // authoritative resolved name. These flags prevent re-emitting the same
        // hint every tick.
        let mut provisional_name_emitted = false;
        let mut resolved_name_emitted = false;

        let completion = handle.wait_until_completed();
        tokio::pin!(completion);

        let mut ticker = tokio::time::interval(STATS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let run_result: Result<(), TorrentError> = loop {
            tokio::select! {
                biased;

                // Cancellation: caller paused / shut down. librqbit flushes
                // fastresume on pause; we leave the torrent in the session so a
                // later resume reuses its state (design §3.A pause-for-free note).
                _ = cancel.cancelled() => {
                    break Err(TorrentError::Cancelled);
                }

                // Completion: all selected pieces verified.
                done = &mut completion => {
                    break done.map_err(TorrentError::backend);
                }

                // Stats tick: translate librqbit stats → engine ProgressEvents.
                _ = ticker.tick() => {
                    let stats = handle.stats();

                    // Mid-run disk / metadata failure surfaces as the error state
                    // (design §5.6 — sparse files fail at write time).
                    if matches!(stats.state, TorrentStatsState::Error) {
                        let msg = stats
                            .error
                            .clone()
                            .unwrap_or_else(|| "torrent entered error state".to_string());
                        if let Some(tx) = tx.as_ref() {
                            let _ = tx.send(engine::ProgressEvent::Failed {
                                error: msg.clone(),
                            });
                        }
                        break Err(TorrentError::Backend(msg));
                    }

                    // Diagnostic heartbeat (INFO, run with RUST_LOG=info). Makes
                    // a "stuck at 0%" run debuggable: peers_seen==0 ⇒ discovery
                    // is failing (trackers/DHT returning nothing); peers_seen>0
                    // with peers_connected==0 ⇒ we find peers but can't connect.
                    {
                        let (seen, connected) = match stats.live.as_ref() {
                            Some(l) => {
                                (l.snapshot.peer_stats.seen, l.snapshot.peer_stats.live)
                            }
                            None => (0, 0),
                        };
                        tracing::info!(
                            target: "unduhin_torrent",
                            state = ?stats.state,
                            peers_seen = seen,
                            peers_connected = connected,
                            fetched = stats.progress_bytes,
                            total = stats.total_bytes,
                            "torrent heartbeat"
                        );
                    }

                    // During the initial checksum phase librqbit reports
                    // progress_bytes == total (it counts all pieces as in-flight
                    // before verifying them). Don't surface that as download
                    // progress — skip emitting until the torrent is actually
                    // live, so the UI doesn't flash 100% and snap back.
                    if matches!(stats.state, TorrentStatsState::Initializing) {
                        continue;
                    }

                    // `Started` once we know the total size.
                    if !started_emitted && stats.total_bytes > 0 {
                        started_emitted = true;
                        if let Some(tx) = tx.as_ref() {
                            let _ = tx.send(engine::ProgressEvent::Started {
                                total: Some(stats.total_bytes),
                                segments: 1,
                                resumed_bytes: stats.progress_bytes,
                            });
                        }
                    }

                    // `FilenameLearned`: prefer the authoritative resolved name
                    // (from metadata) so the pump re-categorizes off the real
                    // torrent name; emit the provisional magnet `dn=` name first
                    // if metadata hasn't resolved yet. Each is emitted at most once.
                    if !resolved_name_emitted {
                        match handle.with_metadata(|m| m.name.clone()) {
                            // Metadata resolved: emit the real name (or fall back
                            // to the magnet name if the torrent has no `name`).
                            Ok(meta_name) => {
                                resolved_name_emitted = true;
                                let name = meta_name.or_else(|| handle.name());
                                if let (Some(name), Some(tx)) = (name, tx.as_ref()) {
                                    let _ = tx.send(engine::ProgressEvent::FilenameLearned {
                                        hint: name,
                                    });
                                }
                            }
                            // Metadata not resolved yet: emit the magnet name once
                            // as a provisional hint while we wait.
                            Err(_) if !provisional_name_emitted => {
                                if let Some(name) = handle.name() {
                                    provisional_name_emitted = true;
                                    if let Some(tx) = tx.as_ref() {
                                        let _ = tx.send(engine::ProgressEvent::FilenameLearned {
                                            hint: name,
                                        });
                                    }
                                }
                            }
                            Err(_) => {}
                        }
                    }

                    // Per-tick heartbeat. Speed comes from live stats; a
                    // paused/initializing torrent reports zero speed. ETA is
                    // recomputed from remaining bytes (librqbit's ETA wraps a
                    // private Duration we can't read).
                    let speed_bps = stats
                        .live
                        .as_ref()
                        .map(|live| mbps_to_bytes_per_sec(live.download_speed.mbps) as f64)
                        .unwrap_or(0.0);
                    let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes);
                    let eta = eta_from(remaining, speed_bps);
                    if let Some(tx) = tx.as_ref() {
                        let _ = tx.send(engine::ProgressEvent::Tick {
                            downloaded: stats.progress_bytes,
                            total: Some(stats.total_bytes),
                            speed_bps,
                            eta,
                        });
                    }

                    // Swarm snapshot (design §3.C). librqbit's aggregate peer
                    // stats don't break out seeders, so `peers` is the count of
                    // currently-connected peers (`live`) and `seeds` is the total
                    // peers discovered in the swarm (`seen`) — the closest signal
                    // available without a per-peer scan. Up/down speeds reuse the
                    // same Mbps→bytes conversion as the `Tick` above so the two
                    // events agree. Ratio is uploaded/downloaded in thousandths.
                    if let Some(tx) = tx.as_ref() {
                        let (peers, seeds, up_bps) = match stats.live.as_ref() {
                            Some(live) => {
                                let peers = live.snapshot.peer_stats.live as u32;
                                let seeds = live.snapshot.peer_stats.seen as u32;
                                let up_bps = mbps_to_bytes_per_sec(live.upload_speed.mbps);
                                (peers, seeds, up_bps)
                            }
                            // Not live (initializing / paused): no peers, no upload.
                            None => (0, 0, 0),
                        };
                        let ratio_milli = if stats.progress_bytes == 0 {
                            0
                        } else {
                            ((stats.uploaded_bytes as u128 * 1000)
                                / stats.progress_bytes as u128)
                                .min(u32::MAX as u128) as u32
                        };
                        let _ = tx.send(engine::ProgressEvent::SwarmProgress {
                            peers,
                            seeds,
                            up_bps,
                            down_bps: speed_bps as u64,
                            ratio_milli,
                        });
                    }

                    // Per-file progress (design §3.C). Cache the file lengths once
                    // metadata resolves, then pair them with `file_progress`
                    // (downloaded bytes per file index) to emit one event per file.
                    if file_lengths.is_none() {
                        file_lengths = handle
                            .with_metadata(|m| {
                                m.info.iter_file_details().ok().map(|it| {
                                    it.map(|d| d.len).collect::<Vec<u64>>()
                                })
                            })
                            .ok()
                            .flatten();
                    }
                    if let (Some(tx), Some(lengths)) = (tx.as_ref(), file_lengths.as_ref()) {
                        for (index, &downloaded) in stats.file_progress.iter().enumerate() {
                            let total = lengths.get(index).copied().unwrap_or(0);
                            let _ = tx.send(engine::ProgressEvent::FileProgress {
                                index,
                                downloaded,
                                total,
                            });
                        }
                    }
                }
            }
        };

        // On any terminal outcome capture the final picture for the summary.
        let final_stats = handle.stats();
        let bytes = final_stats.progress_bytes;

        // Build the metadata snapshot for the summary (name + files), best-effort.
        let metadata = self.snapshot_metadata(&handle, &info_hash);

        match run_result {
            Ok(()) => {
                // Emit Completed before we tear the torrent down.
                if let Some(tx) = tx.as_ref() {
                    let _ = tx.send(engine::ProgressEvent::Completed { bytes });
                }

                // Seed phase. `seed_ratio_milli == 0` is the default — "stop at
                // 100 %, no seeding" — so we fall straight through to the forget
                // below. A positive ratio keeps the torrent in the session and
                // uploading until it hits the target (or the worker is
                // cancelled); the worker stays alive for the whole phase, so a
                // seeding torrent holds its `max_concurrent_downloads` slot.
                if self.seed_ratio_milli > 0 {
                    self.seed_to_ratio(&handle, &cancel, tx.as_ref()).await;
                }

                // Q1: librqbit keeps SEEDING after completion. Forget the torrent
                // (without deleting files) to stop all upload. `delete(.., false)`
                // removes it from the session but leaves content on disk.
                if let Err(e) = self
                    .session
                    .delete(torrent_id.into(), /* delete_files = */ false)
                    .await
                {
                    tracing::warn!(
                        info_hash = %info_hash,
                        error = %e,
                        "torrent: failed to stop/forget completed torrent (it may keep seeding)"
                    );
                }

                // We pass an explicit `output_folder = download_dir`, so librqbit
                // writes per-file `relative_filename`s DIRECTLY under it (the
                // per-torrent name subfolder is only added when no explicit
                // output_folder is given). The content root is therefore the
                // download dir itself — uniform for single- and multi-file, and
                // exactly what Q4's `remove_dir_all` keys off.
                Ok(TorrentRunSummary {
                    output_root: download_dir,
                    bytes,
                    resumed,
                    metadata,
                })
            }
            Err(TorrentError::Cancelled) => {
                // Pause (not delete) so fastresume is flushed and a later resume
                // reuses the session state.
                if let Err(e) = self.session.pause(&handle).await {
                    tracing::warn!(
                        info_hash = %info_hash,
                        error = %e,
                        "torrent: failed to pause on cancel"
                    );
                }
                Err(TorrentError::Cancelled)
            }
            Err(other) => {
                // Failure already emitted `Failed` if it came from the error
                // state; map and bubble up.
                Err(other)
            }
        }
    }

    /// Seed a *completed* torrent until its upload:download ratio reaches the
    /// session's [`seed_ratio_milli`](TorrentConfig::seed_ratio_milli)
    /// (thousandths), then return so the caller forgets it. The download is
    /// already 100 % here, so this only governs how long we keep uploading.
    ///
    /// Returns early when `cancel` fires (pause / remove / app shutdown). Emits
    /// `SwarmProgress` ticks (now with `down_bps = 0`) so the UI keeps showing
    /// live upload speed / ratio while seeding. Best-effort: a 0-byte torrent
    /// has nothing to seed and returns immediately.
    async fn seed_to_ratio(
        &self,
        handle: &Arc<librqbit::ManagedTorrent>,
        cancel: &CancellationToken,
        tx: Option<&broadcast::Sender<engine::ProgressEvent>>,
    ) {
        let mut ticker = tokio::time::interval(STATS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    let stats = handle.stats();
                    let downloaded = stats.progress_bytes;
                    // Nothing downloaded ⇒ nothing to seed; avoid divide-by-zero.
                    if downloaded == 0 {
                        return;
                    }
                    let ratio_milli = ((stats.uploaded_bytes as u128 * 1000)
                        / downloaded as u128)
                        .min(u32::MAX as u128) as u32;

                    if let Some(tx) = tx {
                        let (peers, seeds, up_bps) = match stats.live.as_ref() {
                            Some(live) => (
                                live.snapshot.peer_stats.live as u32,
                                live.snapshot.peer_stats.seen as u32,
                                mbps_to_bytes_per_sec(live.upload_speed.mbps),
                            ),
                            None => (0, 0, 0),
                        };
                        let _ = tx.send(engine::ProgressEvent::SwarmProgress {
                            peers,
                            seeds,
                            up_bps,
                            down_bps: 0,
                            ratio_milli,
                        });
                    }

                    if ratio_milli >= self.seed_ratio_milli {
                        return;
                    }
                }
            }
        }
    }

    /// Best-effort metadata snapshot (name + files) from a live handle, for the
    /// run summary. Falls back to the infohash for the name and an empty file
    /// list if metadata is not yet resolved.
    fn snapshot_metadata(
        &self,
        handle: &Arc<librqbit::ManagedTorrent>,
        info_hash: &str,
    ) -> TorrentMetadata {
        let name = handle
            .name()
            .unwrap_or_else(|| info_hash.to_string());
        let files = handle
            .with_metadata(|m| {
                m.info
                    .iter_file_details()
                    .map(|it| {
                        it.map(|d| TorrentFileEntry {
                            path: d
                                .filename
                                .to_string()
                                .unwrap_or_else(|_| "<unnamed>".to_string()),
                            length: d.len,
                        })
                        .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        TorrentMetadata {
            info_hash: info_hash.to_string(),
            name,
            files,
        }
    }

    /// Update the session-wide download speed cap live (bytes/sec; `0` =
    /// unlimited). The `core` queue calls this when the user changes
    /// `global_speed_limit_bps` so a running session re-paces without a
    /// restart. Upload stays uncapped — seeding is bounded by ratio, not rate.
    pub fn set_download_limit(&self, download_bps: u64) {
        self.session
            .ratelimits
            .set_download_bps(NonZeroU32::new(download_bps.min(u32::MAX as u64) as u32));
    }

    /// The session's default download dir, for diagnostics / tests.
    pub fn download_dir(&self) -> &std::path::Path {
        &self.download_dir
    }
}
