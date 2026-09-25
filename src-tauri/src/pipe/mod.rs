//! In-app bridge server. Accepts framed `Inbound` JSON messages from the
//! native-messaging host (`unduhin-native-host`) and dispatches them onto
//! the live [`Core`].
//!
//! The server runs on the same tokio runtime as the rest of the Tauri app.
//! Multiple host sessions can be in flight at once — each accepted
//! connection is handled on its own task. Single-instance enforcement
//! (wired in `lib.rs`) guarantees exactly one server is alive, so the
//! well-known endpoint never collides.
//!
//! Everything in this module is platform-neutral. The one thing that is
//! not — a named pipe on Windows against a Unix domain socket elsewhere —
//! lives behind [`transport`], which hands back split read/write halves
//! either way. That is why the dispatch, caching, and broadcast code below
//! carries no `cfg` at all.

mod transport;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use tauri::AppHandle;
use tokio::io::WriteHalf;
use tokio::sync::Mutex as AsyncMutex;
use unduhin_core::wire::ExtensionSettings;
use unduhin_core::wire::HandoffDecision;
use unduhin_core::wire::RuleMetric;
use unduhin_core::Core;

use tauri::Emitter;

use transport::ServerStream;

/// `true` once the in-app named-pipe server has bound the well-known
/// path and is accepting client connections. Set exactly once per
/// process; the Settings → Browser status card reads this through
/// [`crate::browser_integration::pipe_status`].
static PIPE_LISTENING: AtomicBool = AtomicBool::new(false);

/// Live write halves of every connected pipe client, used by the
/// settings-changed broadcast. Each entry is wrapped in its own
/// async mutex so [`broadcast_settings_changed`] can fan out
/// concurrently without two writers stepping on each other.
///
/// Per-process singleton: the accept loop pushes on each new
/// connection; the per-connection task removes its slot on hang-up.
type ClientWriter = AsyncMutex<WriteHalf<ServerStream>>;
static CONNECTED_CLIENTS: OnceLock<AsyncMutex<Vec<Arc<ClientWriter>>>> = OnceLock::new();

fn connected_clients() -> &'static AsyncMutex<Vec<Arc<ClientWriter>>> {
    CONNECTED_CLIENTS.get_or_init(|| AsyncMutex::new(Vec::new()))
}

/// Cached "last-known" extension settings. Populated whenever the
/// extension pushes via [`Inbound::SetSettings`] (or on the extension
/// bridge's connect-time full push). [`Inbound::GetSettings`] returns
/// this; if nothing has been pushed yet the default shape is returned
/// — it matches the extension's own `DEFAULT_SETTINGS` byte-for-byte
/// so the panel doesn't lie about the user's choices.
static SETTINGS_CACHE: OnceLock<AsyncMutex<Option<ExtensionSettings>>> = OnceLock::new();

fn settings_cache() -> &'static AsyncMutex<Option<ExtensionSettings>> {
    SETTINGS_CACHE.get_or_init(|| AsyncMutex::new(None))
}

/// Cached per-rule metrics snapshot pushed by the extension's alarm
/// tick (`Inbound::RuleMetrics`). The Tauri panel reads via
/// `get_rule_metrics`; the cache is replaced (not merged) on every
/// push so the extension's local store is the source of truth.
static RULE_METRICS_CACHE: OnceLock<AsyncMutex<Vec<RuleMetric>>> = OnceLock::new();

fn rule_metrics_cache() -> &'static AsyncMutex<Vec<RuleMetric>> {
    RULE_METRICS_CACHE.get_or_init(|| AsyncMutex::new(Vec::new()))
}

/// Version of the extension currently staged in the app-managed canonical
/// folder (`extension_sync`). Set once by the startup sync task — on
/// no-op runs too, because the connection greeting needs it either way.
/// `None` until the sync has run (or when no bundle ships with this
/// build, e.g. a dev shell that never built the extension).
static CANONICAL_EXT_VERSION: OnceLock<AsyncMutex<Option<String>>> = OnceLock::new();

fn canonical_ext_version() -> &'static AsyncMutex<Option<String>> {
    CANONICAL_EXT_VERSION.get_or_init(|| AsyncMutex::new(None))
}

/// Record the canonical extension version for connection greetings.
pub async fn set_canonical_extension_version(version: String) {
    *canonical_ext_version().lock().await = Some(version);
}

/// Read-only accessor for the cached rule-metrics snapshot. Returns
/// an empty vec until the first push arrives.
pub async fn cached_rule_metrics() -> Vec<RuleMetric> {
    rule_metrics_cache().lock().await.clone()
}

/// Read the cached settings if any. Used by future Tauri commands
/// (9e's `apply_extension_settings_patch`) and tests.
pub async fn cached_extension_settings() -> Option<ExtensionSettings> {
    settings_cache().lock().await.clone()
}

/// Replace the cached settings. Called by the panel-driven
/// `apply_extension_settings_patch` Tauri command so a subsequent
/// `GetSettings` returns the panel's new shape without waiting for the
/// extension's `chrome.storage.onChanged` echo.
pub async fn store_extension_settings(full: ExtensionSettings) {
    *settings_cache().lock().await = Some(full);
}

/// In-flight [`Outbound::RefreshCredentials`] requests, keyed by download id.
///
/// Two jobs. It correlates a reply to its request, so a late answer for a
/// superseded attempt gets dropped instead of re-queueing a row the user may
/// have since paused. And its presence marks "already tried", which is what
/// bounds the silent tier to one attempt: without that, a row whose cookies
/// are genuinely dead would loop fail → refresh → fail forever.
fn pending_credential_refresh() -> &'static AsyncMutex<std::collections::HashMap<i64, String>> {
    static PENDING: OnceLock<AsyncMutex<std::collections::HashMap<i64, String>>> = OnceLock::new();
    PENDING.get_or_init(|| AsyncMutex::new(std::collections::HashMap::new()))
}

/// Claim the right to make one silent credential-refresh attempt for `id`.
///
/// Returns the correlation token, or `None` when an attempt is already in
/// flight or no extension is connected to answer. The caller must clear the
/// entry via [`take_pending_credential_refresh`] or
/// [`forget_credential_refresh`], otherwise the download never gets another
/// automatic try.
///
/// The token only has to be unique among in-flight requests, so a monotonic
/// counter is enough — it is a correlation tag, not a secret.
pub async fn begin_credential_refresh(id: i64) -> Option<String> {
    // Nobody to ask. Claiming the slot here would burn the row's one
    // automatic attempt on a request that was never sent.
    if connected_clients().lock().await.is_empty() {
        return None;
    }
    let mut guard = pending_credential_refresh().lock().await;
    if guard.contains_key(&id) {
        return None;
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let token = format!(
        "cred-{id}-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    guard.insert(id, token.clone());
    Some(token)
}

/// How long a captured `ask-first` job waits for the user's answer before it
/// is dropped. The browser's own download stays blocked meanwhile, so a
/// prompt nobody answers for this long is abandoned anyway.
const PENDING_HANDOFF_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Most `ask-first` jobs held at once; the oldest goes first past this.
const MAX_PENDING_HANDOFFS: usize = 32;

/// `ask-first` jobs waiting for the user's choice in the app's dialog,
/// keyed by the handoff id.
///
/// The job carries the capture's cookies and request headers. Keeping it
/// here means the webview only ever sees a copy without them (see
/// [`redacted_for_prompt`]) and hands back just the id.
fn pending_handoffs() -> &'static AsyncMutex<
    std::collections::HashMap<String, (std::time::Instant, unduhin_core::wire::DownloadJob)>,
> {
    static PENDING: OnceLock<
        AsyncMutex<
            std::collections::HashMap<
                String,
                (std::time::Instant, unduhin_core::wire::DownloadJob),
            >,
        >,
    > = OnceLock::new();
    PENDING.get_or_init(|| AsyncMutex::new(std::collections::HashMap::new()))
}

/// Hold `job` until the user answers the prompt for `id`.
async fn hold_handoff(id: String, job: unduhin_core::wire::DownloadJob) {
    let mut pending = pending_handoffs().lock().await;
    let now = std::time::Instant::now();
    pending.retain(|_, (at, _)| now.duration_since(*at) < PENDING_HANDOFF_TTL);
    while pending.len() >= MAX_PENDING_HANDOFFS {
        let Some(oldest) = pending
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        pending.remove(&oldest);
    }
    pending.insert(id, (now, job));
}

/// Take the held job for `id`, if it is still there.
pub async fn take_handoff(id: &str) -> Option<unduhin_core::wire::DownloadJob> {
    let mut pending = pending_handoffs().lock().await;
    let (at, job) = pending.remove(id)?;
    (at.elapsed() < PENDING_HANDOFF_TTL).then_some(job)
}

/// What the prompt is shown: the job without its cookies and captured
/// request headers, which the dialog never needs.
fn redacted_for_prompt(job: &unduhin_core::wire::DownloadJob) -> unduhin_core::wire::DownloadJob {
    let mut shown = job.clone();
    shown.cookie_header = None;
    shown.request_headers = Vec::new();
    shown
}

/// Consume the pending entry for `id` when `token` matches. `false` means the
/// reply is stale and must be ignored.
async fn take_pending_credential_refresh(id: i64, token: &str) -> bool {
    let mut guard = pending_credential_refresh().lock().await;
    match guard.get(&id) {
        Some(t) if t == token => {
            guard.remove(&id);
            true
        }
        _ => false,
    }
}

/// Drop any pending entry for `id`, re-allowing a future automatic attempt.
/// Called when a download completes or is removed — the next failure is a new
/// situation, not a repeat of the one already tried.
pub async fn forget_credential_refresh(id: i64) {
    pending_credential_refresh().lock().await.remove(&id);
}

/// Fan one unsolicited frame out to every connected pipe client.
///
/// Best-effort: per-client write errors are logged and the broken
/// connection's writer eventually gets reaped by its owning task.
///
/// The client list is snapshotted so no per-client write happens while the
/// outer lock is held — a slow client must only block its own per-writer
/// mutex. The `Arc<Mutex<WriteHalf>>`s stay valid after the snapshot vec is
/// dropped, because broken connections are pruned by their own task.
///
/// `label` names the frame in the log lines only.
async fn broadcast(frame: unduhin_core::wire::Outbound, label: &str) {
    use unduhin_core::wire::framing::write_frame;

    let bytes = match serde_json::to_vec(&frame) {
        Ok(buf) => buf,
        Err(e) => {
            tracing::warn!(error = %e, label, "serialize outbound frame failed");
            return;
        }
    };
    let snapshot: Vec<Arc<ClientWriter>> = connected_clients().lock().await.clone();
    for client in snapshot {
        let mut writer = client.lock().await;
        if let Err(e) = write_frame(&mut *writer, &bytes).await {
            tracing::debug!(error = %e, label, "broadcast write failed (client likely gone)");
        }
    }
}

/// Broadcast a `SettingsChanged { full }` frame. Public so Tauri commands can
/// push panel-driven edits back out to the extension.
pub async fn broadcast_settings_changed(full: ExtensionSettings) {
    broadcast(
        unduhin_core::wire::Outbound::SettingsChanged { full },
        "SettingsChanged",
    )
    .await;
}

/// Broadcast a `HandoffDecision { id, decision }` frame. The extension routes
/// the unsolicited frame back to the matching `ask-first` waiter by `id`.
pub async fn broadcast_handoff_decision(id: String, decision: HandoffDecision) {
    broadcast(
        unduhin_core::wire::Outbound::HandoffDecision { id, decision },
        "HandoffDecision",
    )
    .await;
}

/// Broadcast an `ExtensionUpdated { version }` frame. Sent by the startup sync
/// after the canonical extension folder was replaced; the extension reloads
/// itself when its running version is older.
pub async fn broadcast_extension_updated(version: String) {
    broadcast(
        unduhin_core::wire::Outbound::ExtensionUpdated { version },
        "ExtensionUpdated",
    )
    .await;
}

/// Arm the extension to fold the next matching capture into `download_id`
/// rather than creating a new row. Sent when the user clicks "Refresh link".
///
/// See [`unduhin_core::wire::Outbound::ArmRefresh`] for why the match is on
/// file name and size rather than tab or page URL.
pub async fn broadcast_arm_refresh(
    download_id: i64,
    filename: Option<String>,
    size_bytes: Option<u64>,
    origin: Option<String>,
    referrer_origin: Option<String>,
    expires_at_ms: i64,
) {
    broadcast(
        unduhin_core::wire::Outbound::ArmRefresh {
            download_id,
            filename,
            size_bytes,
            origin,
            referrer_origin,
            expires_at_ms,
        },
        "ArmRefresh",
    )
    .await;
}

/// Ask the extension for a fresh cookie header for `url`. The reply arrives
/// later as an `Inbound::CredentialsRefreshed` carrying the same `token`.
pub async fn broadcast_refresh_credentials(token: String, download_id: i64, url: String) {
    broadcast(
        unduhin_core::wire::Outbound::RefreshCredentials {
            token,
            download_id,
            url,
        },
        "RefreshCredentials",
    )
    .await;
}

/// Test-only: wipe the settings cache so integration tests don't
/// leak state across `#[tokio::test]`s running in the same binary.
/// The cache is otherwise a per-process singleton (production only
/// runs one server per process).
#[doc(hidden)]
pub async fn reset_settings_cache_for_tests() {
    *settings_cache().lock().await = None;
}

/// The bound pipe name, captured at first-accept time. Used by
/// [`crate::browser_integration::pipe_status`] so the UI can surface
/// the real path (handy when `UNDUHIN_PIPE_NAME` is set for a dev
/// override).
static BOUND_PIPE_NAME: OnceLock<String> = OnceLock::new();

/// Snapshot of the pipe listener state read by the Settings → Browser
/// card. Returns `(name, listening)` — the name may still be set even
/// when `listening` is false on platforms that build with the no-op
/// stub.
pub fn listening_snapshot() -> (Option<String>, bool) {
    (
        BOUND_PIPE_NAME.get().cloned(),
        PIPE_LISTENING.load(Ordering::Acquire),
    )
}

/// Resolved bridge endpoint — a pipe name on Windows, a socket path
/// elsewhere. `UNDUHIN_PIPE_NAME` is honoured so the integration tests can
/// use a per-process random name and avoid colliding with a real running
/// app.
///
/// Kept as a re-export rather than a second copy: the native host resolves
/// the same endpoint from the same function, and two definitions that
/// drifted would break the bridge with no visible error.
pub fn pipe_name() -> String {
    unduhin_core::wire::transport::endpoint()
}

/// AppHandle stash so the `AskHandoff` dispatch can emit a frontend
/// event without threading the handle through every helper. Set once
/// in `install`; ignored in tests that drive `run_server` directly.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub(crate) fn app_handle() -> Option<&'static AppHandle> {
    APP_HANDLE.get()
}

/// Windows pipe security: build a restrictive DACL so only the current
/// user (and LocalSystem) can connect to the pipe. Without this the pipe
/// inherits a default descriptor that lets *any* same-user process open
/// it and inject download jobs — historically combinable with the
/// filename path-traversal bug into an arbitrary-file-write primitive.
///
/// The Unix side achieves the same thing with directory and socket
/// permissions instead; see [`transport`].
#[cfg(windows)]
pub(super) mod pipe_security {
    use std::io;

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Owns a security descriptor and a `SECURITY_ATTRIBUTES` that
    /// references it. Must outlive every pipe-instance creation that uses
    /// its pointer; freed when dropped.
    pub struct PipeSecurity {
        psd: PSECURITY_DESCRIPTOR,
        sa: SECURITY_ATTRIBUTES,
    }

    // SAFETY: the descriptor is allocated once at construction and only ever
    // read (by the kernel, synchronously, at pipe-creation time) thereafter.
    // The raw pointers are never mutated after construction, so moving the
    // owner between the async runtime's worker threads is sound.
    unsafe impl Send for PipeSecurity {}
    unsafe impl Sync for PipeSecurity {}

    impl PipeSecurity {
        /// Build a protected DACL granting full access to the current user
        /// and LocalSystem only.
        pub fn current_user_only() -> io::Result<Self> {
            let sid = current_user_sid_string()?;
            let sddl = format!("D:P(A;;FA;;;{sid})(A;;FA;;;SY)");
            let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
            let mut psd = PSECURITY_DESCRIPTOR::default();
            // SAFETY: `wide` is a valid NUL-terminated UTF-16 SDDL string;
            // `psd` receives a LocalAlloc'd descriptor freed in `Drop`.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(wide.as_ptr()),
                    SDDL_REVISION_1,
                    &mut psd,
                    None,
                )
                .map_err(|e| io::Error::other(format!("build security descriptor: {e}")))?;
            }
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: psd.0,
                bInheritHandle: false.into(),
            };
            Ok(Self { psd, sa })
        }

        /// Pointer to the `SECURITY_ATTRIBUTES` for tokio's
        /// `create_with_security_attributes_raw`.
        pub fn as_attrs_ptr(&self) -> *mut core::ffi::c_void {
            &self.sa as *const SECURITY_ATTRIBUTES as *mut core::ffi::c_void
        }
    }

    impl Drop for PipeSecurity {
        fn drop(&mut self) {
            if !self.psd.0.is_null() {
                // SAFETY: `psd` was allocated by the Convert* call above.
                unsafe {
                    let _ = LocalFree(Some(HLOCAL(self.psd.0)));
                }
            }
        }
    }

    fn current_user_sid_string() -> io::Result<String> {
        // SAFETY: standard token-query sequence; every out-param is sized
        // before use and handles are closed/freed before returning.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
                .map_err(|e| io::Error::other(format!("OpenProcessToken: {e}")))?;

            let mut len = 0u32;
            // First call sizes the buffer (expected to "fail" with
            // ERROR_INSUFFICIENT_BUFFER); ignore its result.
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let info = GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                len,
                &mut len,
            );
            let _ = CloseHandle(token);
            info.map_err(|e| io::Error::other(format!("GetTokenInformation: {e}")))?;

            let tu = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut sid_str = PWSTR::null();
            ConvertSidToStringSidW(tu.User.Sid, &mut sid_str)
                .map_err(|e| io::Error::other(format!("ConvertSidToStringSid: {e}")))?;
            let sid = sid_str
                .to_string()
                .map_err(|e| io::Error::other(format!("SID utf16: {e}")))?;
            let _ = LocalFree(Some(HLOCAL(sid_str.0 as *mut core::ffi::c_void)));
            Ok(sid)
        }
    }
}

/// Spawn the pipe server. Idempotent at the API level (the runtime
/// task survives until app shutdown). Errors are logged rather than
/// returned because a failing pipe must not prevent the rest of the
/// app from coming up — the user can still drive downloads from the
/// UI even if the extension bridge is dead.
pub fn install(app: AppHandle, core: Core) {
    let _ = APP_HANDLE.set(app);
    let name = pipe_name();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_server(name.clone(), core).await {
            tracing::warn!(error = %e, pipe = %name, "pipe server exited");
        }
    });
}

/// Unbind the bridge endpoint.
///
/// Called explicitly from the exit path because `lib.rs` finishes with
/// `std::process::exit(0)`, which runs no destructors. On Windows this is
/// a no-op — a named pipe has no filesystem entry outliving the process.
/// On Unix it unlinks the socket so the next launch does not have to
/// decide whether a leftover is stale.
pub fn shutdown() {
    #[cfg(unix)]
    if let Some(path) = BOUND_PIPE_NAME.get() {
        let _ = std::fs::remove_file(path);
    }
}

/// The actual accept loop. Exposed `pub` so the integration test in
/// `src-tauri/tests/pipe_smoke.rs` can drive it with a per-test endpoint
/// without needing a Tauri `AppHandle`. Production callers go through
/// [`install`].
pub async fn run_server(name: String, core: Core) -> std::io::Result<()> {
    let mut listener = transport::Listener::bind(&name).await?;

    tracing::info!(endpoint = %name, "bridge server listening");

    // Latch the listener-ready signal *before* the first accept so the
    // status card can flip to "connected" as soon as the listener is
    // bound, even if no client has connected yet. The OnceLock guards
    // against multiple `run_server` invocations on the same process
    // (the integration test spawns its own server with a unique name —
    // we only want to remember the first / real path).
    if BOUND_PIPE_NAME.set(name.clone()).is_ok() {
        PIPE_LISTENING.store(true, Ordering::Release);
        core.publish_event(unduhin_core::CoreEvent::PipeListening { name: name.clone() });
    }

    loop {
        let stream = listener.accept().await?;
        let core = core.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, core).await {
                tracing::debug!(error = %e, "bridge connection closed with error");
            }
        });
    }
}

async fn handle_connection(stream: ServerStream, core: Core) -> std::io::Result<()> {
    use unduhin_core::wire::framing::{read_frame, write_frame};
    use unduhin_core::wire::{Inbound, Outbound};

    // Split into independent read/write halves so the
    // settings-changed broadcast can write to this connection while
    // the per-connection loop is parked on `read_frame`. The write
    // half lives inside an async Mutex registered in
    // `connected_clients()` for the lifetime of the connection.
    let (mut reader, writer) = tokio::io::split(stream);
    let writer = Arc::new(AsyncMutex::new(writer));
    connected_clients().lock().await.push(writer.clone());

    // Greeting: announce the canonical extension version so a session
    // that connects *after* a folder swap (the pipe connects lazily on
    // the extension's first non-Ping send) still learns it should
    // reload. The frame is unsolicited; the extension bridge routes it
    // outside the reply FIFO and ignores it unless its running version
    // is older.
    if let Some(version) = canonical_ext_version().lock().await.clone() {
        let frame = serde_json::to_vec(&unduhin_core::wire::Outbound::ExtensionUpdated { version })
            .unwrap_or_default();
        if !frame.is_empty() {
            let mut w = writer.lock().await;
            if let Err(e) = unduhin_core::wire::framing::write_frame(&mut *w, &frame).await {
                tracing::debug!(error = %e, "extension-version greeting write failed");
            }
        }
    }

    let result: std::io::Result<()> = async {
        loop {
            let frame = match read_frame(&mut reader).await? {
                Some(buf) => buf,
                None => return Ok(()),
            };

            let inbound: Inbound = match serde_json::from_slice(&frame) {
                Ok(msg) => msg,
                Err(e) => {
                    let resp = Outbound::Error {
                        message: format!("invalid json: {e}"),
                    };
                    let out = serde_json::to_vec(&resp)
                        .unwrap_or_else(|_| br#"{"type":"error","message":"serialize"}"#.to_vec());
                    let mut w = writer.lock().await;
                    write_frame(&mut *w, &out).await?;
                    continue;
                }
            };

            // `SetSettings` is the one branch that touches the
            // broadcast surface: dispatch returns the local response
            // for the originating client, then we fan the resulting
            // full snapshot out to every other connected client so
            // their UI stays consistent.
            let broadcast_after = matches!(inbound, Inbound::SetSettings { .. });

            let response = dispatch(&core, inbound).await;
            let buf = serde_json::to_vec(&response).unwrap_or_else(|_| {
                br#"{"type":"error","message":"serialize Outbound failed"}"#.to_vec()
            });
            {
                let mut w = writer.lock().await;
                write_frame(&mut *w, &buf).await?;
            }

            // The dispatch already updated `settings_cache()`; pull
            // the freshly-cached value and fan it out. We use
            // `SettingsChanged` (not `Settings`) so receivers can
            // distinguish a request reply from an unsolicited push.
            if broadcast_after {
                if let Some(full) = cached_extension_settings().await {
                    broadcast_settings_changed(full).await;
                }
            }
        }
    }
    .await;

    // Unregister from the broadcast list on hang-up so we don't write
    // into a dead handle. Pointer equality is correct here — every
    // connection holds a unique `Arc`.
    connected_clients()
        .lock()
        .await
        .retain(|w| !Arc::ptr_eq(w, &writer));
    result
}

async fn dispatch(core: &Core, msg: unduhin_core::wire::Inbound) -> unduhin_core::wire::Outbound {
    use unduhin_core::wire::{Inbound, Outbound};

    match msg {
        Inbound::Ping => Outbound::Pong,
        Inbound::Download { job } => match handle_download(core, job).await {
            Ok(id) => Outbound::Ack { id },
            Err(e) => Outbound::Error {
                message: e.to_string(),
            },
        },
        Inbound::DownloadMedia { stream } => match handle_download_media(core, stream).await {
            Ok(id) => Outbound::Ack { id },
            Err(e) => Outbound::Error {
                message: e.to_string(),
            },
        },
        Inbound::ProbeMedia { url, referrer } => {
            match handle_probe_media(core, url, referrer).await {
                Ok(formats) => Outbound::MediaFormats { formats },
                Err(e) => Outbound::Error { message: e },
            }
        }
        Inbound::RefreshDownload { download_id, job } => {
            match handle_refresh_download(core, download_id, job).await {
                // Ack with the SAME id: no new row was created, the
                // existing one was re-pointed.
                Ok(()) => Outbound::Ack { id: download_id },
                Err(e) => Outbound::Error { message: e },
            }
        }
        Inbound::CredentialsRefreshed {
            token,
            download_id,
            cookie_header,
            user_agent,
            request_headers,
        } => {
            match handle_credentials_refreshed(
                core,
                token,
                download_id,
                cookie_header,
                user_agent,
                request_headers,
            )
            .await
            {
                Ok(()) => Outbound::Ack { id: download_id },
                Err(e) => Outbound::Error { message: e },
            }
        }
        Inbound::DownloadTorrent { job } => match handle_download_torrent(core, job).await {
            Ok(id) => Outbound::Ack { id },
            Err(e) => Outbound::Error {
                message: e.to_string(),
            },
        },
        Inbound::Status => match handle_status(core).await {
            Ok(downloads) => Outbound::Status { downloads },
            Err(e) => Outbound::Error {
                message: e.to_string(),
            },
        },
        Inbound::GetSettings => {
            let full = settings_cache()
                .lock()
                .await
                .clone()
                .unwrap_or_else(ExtensionSettings::defaults);
            Outbound::Settings { full }
        }
        Inbound::AskHandoff { id, job } => {
            // The Tauri frontend owns the actual prompt; we just relay
            // the request as an app event and ack the extension so the
            // pipe loop is free for the next inbound. The reply travels
            // later as an unsolicited `Outbound::HandoffDecision` via
            // `commands::respond_handoff`.
            #[derive(serde::Serialize, Clone)]
            struct AskHandoffPayload<'a> {
                id: &'a str,
                job: &'a unduhin_core::wire::DownloadJob,
            }
            let shown = redacted_for_prompt(&job);
            hold_handoff(id.clone(), job).await;
            if let Some(app) = app_handle() {
                if let Err(e) = app.emit(
                    "unduhin:ask-handoff",
                    AskHandoffPayload {
                        id: &id,
                        job: &shown,
                    },
                ) {
                    tracing::warn!(error = %e, "failed to emit ask-handoff event");
                }
            } else {
                tracing::warn!("ask-handoff fired with no AppHandle — frontend will not prompt");
            }
            Outbound::Ack { id: 0 }
        }
        Inbound::SetSettings { patch } => {
            // Apply the patch on top of whatever's cached (or the
            // canonical defaults if this is the first write). The
            // cached snapshot is what every future `GetSettings`
            // reads, and what the broadcast in `handle_connection`
            // fans out.
            let mut guard = settings_cache().lock().await;
            let mut next = guard.clone().unwrap_or_else(ExtensionSettings::defaults);
            next.apply(patch);
            *guard = Some(next.clone());
            Outbound::Settings { full: next }
        }
        Inbound::RuleMetrics { metrics } => {
            // Replace the cache wholesale; the extension always sends
            // a full snapshot so merging would risk leaking deleted
            // patterns. Fire a `RuleMetricsUpdated` event so the
            // panel's composable re-queries.
            *rule_metrics_cache().lock().await = metrics;
            core.publish_event(unduhin_core::CoreEvent::RuleMetricsUpdated);
            Outbound::Ack { id: 0 }
        }
    }
}

async fn handle_download(
    core: &Core,
    job: unduhin_core::wire::DownloadJob,
) -> Result<unduhin_core::DownloadId, String> {
    let url = url::Url::parse(&job.final_url).map_err(|e| format!("invalid URL: {e}"))?;
    let headers = unduhin_core::wire::headers_from_job(&job);

    let input = unduhin_core::AddDownload {
        url,
        filename: job.filename,
        output_path: None,
        output_dir: None,
        category: None,
        priority: 0,
        segments: None,
        media_info: None,
        headers: if headers.is_empty() {
            None
        } else {
            Some(headers)
        },
        source: unduhin_core::DownloadSource::ExtensionPipe,
        kind: unduhin_core::DownloadKind::Http,
        torrent: None,
    };
    core.add_download(input).await.map_err(|e| format!("{e}"))
}

/// Fold a re-captured job into an EXISTING row rather than adding one.
///
/// The extension only sends this after matching the capture against an armed
/// refresh, so the correlation has already been made browser-side. Everything
/// here is still untrusted: `Core::refresh_source` re-parses the URL and
/// rejects any scheme that is not `http`/`https`.
///
/// Deliberately does NOT fall back to creating a new row when the refresh is
/// rejected. A user who asked to refresh a specific download would otherwise
/// silently get a duplicate alongside the broken one.
async fn handle_refresh_download(
    core: &Core,
    download_id: unduhin_core::DownloadId,
    job: unduhin_core::wire::DownloadJob,
) -> Result<(), String> {
    let headers = unduhin_core::wire::headers_from_job(&job);
    let outcome = match core
        .refresh_source(
            download_id,
            &job.final_url,
            if headers.is_empty() {
                None
            } else {
                Some(headers)
            },
            false,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            let message = format!("{e}");
            // The refresh dialog is open and waiting on this capture. Tell it
            // why nothing happened, or it sits on "waiting" until the user
            // gives up.
            if let Some(app) = app_handle() {
                if let Err(e) = app.emit("unduhin:refresh-failed", (download_id, &message)) {
                    tracing::warn!(error = %e, "failed to emit refresh-failed event");
                }
            }
            return Err(message);
        }
    };

    // A size mismatch needs a human decision, so surface it to the frontend
    // rather than deciding here. The dialog is already open and waiting.
    //
    // The URL rides along because a `SourceChanged` outcome committed nothing:
    // to act on the user's "start again", the dialog has to re-send the very
    // URL that produced the mismatch, and it never saw this one — the capture
    // came from the browser, not from its paste field.
    if let Some(app) = app_handle() {
        if let Err(e) = app.emit(
            "unduhin:refresh-outcome",
            (download_id, &outcome, &job.final_url),
        ) {
            tracing::warn!(error = %e, "failed to emit refresh-outcome event");
        }
    }
    Ok(())
}

/// Apply a fresh cookie header to a row whose session expired, then let the
/// queue try again. The silent tier: no page, no tab, no user action.
///
/// Keeps the row's existing URL — this path exists precisely for the case
/// where the URL is fine and only the session died.
async fn handle_credentials_refreshed(
    core: &Core,
    token: String,
    download_id: unduhin_core::DownloadId,
    cookie_header: Option<String>,
    user_agent: Option<String>,
    request_headers: Vec<unduhin_core::wire::RequestHeader>,
) -> Result<(), String> {
    if !take_pending_credential_refresh(download_id, &token).await {
        // A stale reply for an attempt that was superseded or already
        // resolved. Dropping it is correct: applying it would re-queue a row
        // the user may have since paused or deleted.
        tracing::debug!(download_id, token, "ignoring stale credential refresh");
        return Ok(());
    }

    // Full: the row's original Referer is carried over below.
    let record = core
        .get_download_full(download_id)
        .await
        .map_err(|e| format!("{e}"))?;

    // Reuse the job header folding so ordering and the Cookie/Referer/UA
    // precedence match every other capture path exactly.
    let job = unduhin_core::wire::DownloadJob {
        final_url: record.url.clone(),
        original_url: record.url.clone(),
        // Keep whatever Referer the original capture carried; the extension
        // has no page context to supply a new one here.
        referrer: record.headers.as_ref().and_then(|hs| {
            hs.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("referer"))
                .map(|(_, v)| v.clone())
        }),
        filename: None,
        mime: None,
        size: None,
        cookie_header,
        user_agent,
        request_headers,
        tab_id: None,
        page_url: None,
    };
    let headers = unduhin_core::wire::headers_from_job(&job);

    core.refresh_source(
        download_id,
        &record.url,
        if headers.is_empty() {
            None
        } else {
            Some(headers)
        },
        false,
    )
    .await
    .map_err(|e| format!("{e}"))?;
    Ok(())
}

async fn handle_download_media(
    core: &Core,
    stream: unduhin_core::wire::MediaStream,
) -> Result<unduhin_core::DownloadId, String> {
    let url = url::Url::parse(&stream.manifest_url).map_err(|e| format!("invalid URL: {e}"))?;
    let headers = headers_from_media(&stream);

    // Stub a `MediaInfo` so the queue worker takes the yt-dlp branch.
    // yt-dlp accepts an `.m3u8` / `.mpd` URL directly and handles
    // segment assembly. `format_selector = "best"` matches the
    // popup's default "Download" button.
    let title = stream
        .suggested_filename
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "media".to_string());
    let media_info = unduhin_core::ytdlp::MediaInfo {
        extractor: "browser-capture".to_string(),
        format_selector: "best".to_string(),
        title,
        original_url: stream.manifest_url.clone(),
        needs_ffmpeg: true,
    };

    let input = unduhin_core::AddDownload {
        url,
        filename: stream.suggested_filename,
        output_path: None,
        output_dir: None,
        category: None,
        priority: 0,
        segments: None,
        media_info: Some(media_info),
        headers: if headers.is_empty() {
            None
        } else {
            Some(headers)
        },
        source: unduhin_core::DownloadSource::ExtensionPipe,
        kind: unduhin_core::DownloadKind::Media,
        torrent: None,
    };
    core.add_download(input).await.map_err(|e| format!("{e}"))
}

/// Extension probe of a URL (typically an HLS/DASH manifest, or a
/// Cloudflare-fronted direct-media URL the extension's own fetch 403'd
/// on) via yt-dlp. `url` and `referrer` arrive over the pipe from the
/// browser extension and are UNTRUSTED — both are validated as
/// `http`/`https` URLs before either reaches a yt-dlp argument. Rejecting
/// anything else here (rather than downstream in `ytdlp::probe_raw`)
/// matters because both strings are passed to yt-dlp as bare CLI
/// arguments: an absolute `http`/`https` URL can never start with `-`
/// (the URL spec requires the scheme prefix), so requiring that scheme is
/// what rules out a string yt-dlp's own arg parser could otherwise
/// mistake for a flag, on top of ruling out non-network schemes like
/// `file://` or `javascript:`.
async fn handle_probe_media(
    core: &Core,
    url: String,
    referrer: Option<String>,
) -> Result<Vec<unduhin_core::wire::MediaFormat>, String> {
    validate_http_url(&url).map_err(|e| format!("invalid url: {e}"))?;
    if let Some(r) = referrer.as_deref() {
        validate_http_url(r).map_err(|e| format!("invalid referrer: {e}"))?;
    }
    core.probe_media_formats(&url, referrer.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Parse `raw` as an absolute URL and require an `http`/`https` scheme.
/// Shared validation for [`handle_probe_media`]'s two untrusted-input
/// fields (`url` and `referrer`) — both get the same treatment since both
/// end up as yt-dlp CLI arguments.
fn validate_http_url(raw: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(raw).map_err(|e| e.to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!(
            "unsupported scheme {:?}, expected http/https",
            parsed.scheme()
        ));
    }
    Ok(parsed)
}

/// Extension torrent hand-off. The untrusted [`wire::TorrentJob`] (magnet URI
/// or base64 `.torrent` bytes) is validated and turned into an
/// `AddDownload { kind: Torrent, source: ExtensionPipe }` by
/// `core::torrent_handoff` — which size-limits / sanity-checks the payload and
/// writes any `.torrent` bytes into the managed dir under a content-hash name
/// (no caller-supplied path ever reaches the filesystem). We then hand it to
/// `Core::add_download`, which de-dups by info-hash and assigns the row id.
///
/// [`wire::TorrentJob`]: unduhin_core::wire::TorrentJob
async fn handle_download_torrent(
    core: &Core,
    job: unduhin_core::wire::TorrentJob,
) -> Result<unduhin_core::DownloadId, String> {
    let input = unduhin_core::torrent_handoff::add_download_from_torrent_job(
        job,
        unduhin_core::torrent_handoff::incoming_torrents_dir(),
    )
    .map_err(|e| format!("{e}"))?;
    core.add_download(input).await.map_err(|e| format!("{e}"))
}

async fn handle_status(core: &Core) -> Result<Vec<unduhin_core::wire::StatusEntry>, String> {
    let mut rows = core
        .list_downloads(unduhin_core::DownloadFilter::default())
        .await
        .map_err(|e| format!("{e}"))?;
    // Newest first; cap at 20 to keep the popup snappy.
    rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    rows.truncate(20);
    Ok(rows
        .into_iter()
        .map(|r| unduhin_core::wire::StatusEntry {
            id: r.id,
            url: r.url,
            filename: r.filename,
            status: r.status.to_string(),
            total_bytes: r.total_bytes,
            downloaded_bytes: r.downloaded_bytes,
        })
        .collect())
}

// `headers_from_job` now lives in `unduhin_core::wire` so the `ask-first`
// Tauri command (`commands::start_handoff_download`) folds headers
// identically; `handle_download` calls it directly via the fully-qualified
// path. `headers_from_media` stays here — it operates on a `MediaStream`.

fn headers_from_media(stream: &unduhin_core::wire::MediaStream) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(c) = stream.cookie_header.as_ref().filter(|s| !s.is_empty()) {
        out.push(("Cookie".to_string(), c.clone()));
    }
    if let Some(r) = stream.referrer.as_ref().filter(|s| !s.is_empty()) {
        out.push(("Referer".to_string(), r.clone()));
    }
    if let Some(ua) = stream.user_agent.as_ref().filter(|s| !s.is_empty()) {
        out.push(("User-Agent".to_string(), ua.clone()));
    }
    for h in &stream.request_headers {
        if unduhin_core::wire::is_prepended_header(&h.name) {
            continue;
        }
        out.push((h.name.clone(), h.value.clone()));
    }
    out
}

/// Re-export the pipe path so tests under `src-tauri/tests/` can
/// build a matching client. Kept module-public; the rest of the
/// app doesn't need it.
#[allow(dead_code)]
pub(crate) fn default_pipe_path() -> PathBuf {
    PathBuf::from(pipe_name())
}

// Platform-neutral: the dispatch and validation below are the same code on
// every OS, so these run wherever the crate's tests run (macOS CI included),
// not only on Windows.
#[cfg(test)]
mod tests {
    use super::*;
    use unduhin_core::wire::{headers_from_job, DownloadJob, MediaStream, RequestHeader};

    #[test]
    fn prepends_cookie_referer_ua_then_captured_headers() {
        let job = DownloadJob {
            final_url: "https://x/y.zip".into(),
            original_url: "https://x/y.zip".into(),
            referrer: Some("https://x/page".into()),
            filename: None,
            mime: None,
            size: None,
            cookie_header: Some("a=b".into()),
            user_agent: Some("ua/1.0".into()),
            request_headers: vec![RequestHeader {
                name: "Accept".into(),
                value: "*/*".into(),
            }],
            tab_id: None,
            page_url: None,
        };
        let h = headers_from_job(&job);
        assert_eq!(h[0], ("Cookie".into(), "a=b".into()));
        assert_eq!(h[1], ("Referer".into(), "https://x/page".into()));
        assert_eq!(h[2], ("User-Agent".into(), "ua/1.0".into()));
        assert_eq!(h[3], ("Accept".into(), "*/*".into()));
    }

    #[test]
    fn dedups_captured_cookie_referer_ua() {
        // The extension also captures Referer / User-Agent via webRequest;
        // they must not be appended a second time after the dedicated
        // fields. (Cookie is stripped before capture, but guard it too.)
        let job = DownloadJob {
            final_url: "https://x/y.zip".into(),
            original_url: "https://x/y.zip".into(),
            referrer: Some("https://x/page".into()),
            filename: None,
            mime: None,
            size: None,
            cookie_header: Some("a=b".into()),
            user_agent: Some("ua/1.0".into()),
            request_headers: vec![
                RequestHeader {
                    name: "referer".into(),
                    value: "https://x/page".into(),
                },
                RequestHeader {
                    name: "User-Agent".into(),
                    value: "ua/1.0".into(),
                },
                RequestHeader {
                    name: "Sec-Fetch-Dest".into(),
                    value: "document".into(),
                },
            ],
            tab_id: None,
            page_url: None,
        };
        let h = headers_from_job(&job);
        assert_eq!(h[0], ("Cookie".into(), "a=b".into()));
        assert_eq!(h[1], ("Referer".into(), "https://x/page".into()));
        assert_eq!(h[2], ("User-Agent".into(), "ua/1.0".into()));
        assert_eq!(h[3], ("Sec-Fetch-Dest".into(), "document".into()));
        assert_eq!(h.len(), 4, "duplicate Referer/User-Agent must be dropped");
    }

    #[test]
    fn skips_empty_auth_fields() {
        let job = DownloadJob {
            final_url: "https://x/y.zip".into(),
            original_url: "https://x/y.zip".into(),
            referrer: Some("".into()),
            filename: None,
            mime: None,
            size: None,
            cookie_header: None,
            user_agent: None,
            request_headers: vec![],
            tab_id: None,
            page_url: None,
        };
        let h = headers_from_job(&job);
        assert!(h.is_empty());
    }

    #[test]
    fn media_headers_same_shape() {
        let stream = MediaStream {
            kind: unduhin_core::wire::MediaKind::Hls,
            manifest_url: "https://x/master.m3u8".into(),
            page_url: None,
            tab_id: None,
            suggested_filename: Some("episode-1".into()),
            referrer: Some("https://x/watch".into()),
            user_agent: None,
            cookie_header: Some("s=1".into()),
            request_headers: vec![],
        };
        let h = headers_from_media(&stream);
        assert_eq!(h[0], ("Cookie".into(), "s=1".into()));
        assert_eq!(h[1], ("Referer".into(), "https://x/watch".into()));
    }

    #[test]
    fn validate_http_url_accepts_http_and_https() {
        assert!(validate_http_url("https://cdn.example.com/master.m3u8").is_ok());
        assert!(validate_http_url("http://cdn.example.com/master.m3u8").is_ok());
    }

    #[test]
    fn validate_http_url_rejects_non_http_schemes() {
        // `url` and `referrer` are untrusted browser input passed straight
        // to yt-dlp as CLI arguments — anything other than http/https must
        // be rejected before it gets there.
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com/x",
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        ] {
            assert!(
                validate_http_url(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn validate_http_url_rejects_unparseable_input() {
        // Includes the arg-injection shape a hostile extension build (or
        // a compromised native-messaging peer) could send: a bare string
        // starting with `-` isn't even a URL, and it must never reach
        // `Command::arg` unvalidated regardless of *why* it's rejected.
        for bad in ["", "not a url", "-–impersonate", "   "] {
            assert!(
                validate_http_url(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    /// Drives the real `dispatch -> handle_probe_media -> validate_http_url`
    /// path (not just the unit-level `validate_http_url` above) to confirm
    /// a hostile `url`/`referrer` never reaches `Core::probe_media_formats`
    /// — it comes back as an `Outbound::Error`, not a panic or a yt-dlp
    /// spawn attempt.
    #[tokio::test]
    async fn dispatch_probe_media_rejects_non_http_url() {
        use unduhin_core::wire::{Inbound, Outbound};

        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path().join("pipe-probe-media.db"))
            .await
            .unwrap();

        match dispatch(
            &core,
            Inbound::ProbeMedia {
                url: "file:///etc/passwd".into(),
                referrer: None,
            },
        )
        .await
        {
            Outbound::Error { message } => {
                assert!(message.contains("invalid url"), "got: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_probe_media_rejects_non_http_referrer() {
        use unduhin_core::wire::{Inbound, Outbound};

        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path().join("pipe-probe-media-referrer.db"))
            .await
            .unwrap();

        match dispatch(
            &core,
            Inbound::ProbeMedia {
                url: "https://cdn.example.com/master.m3u8".into(),
                referrer: Some("javascript:alert(1)".into()),
            },
        )
        .await
        {
            Outbound::Error { message } => {
                assert!(message.contains("invalid referrer"), "got: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// An `ask-first` capture's cookies stay in the backend: the prompt gets
    /// a redacted copy, and the real job is handed out once, by id.
    #[tokio::test]
    async fn ask_handoff_holds_the_job_and_shows_a_redacted_copy() {
        use unduhin_core::wire::{Inbound, Outbound};

        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path().join("pipe-ask.db")).await.unwrap();
        let job = DownloadJob {
            final_url: "https://x/y.zip".into(),
            original_url: "https://x/y.zip".into(),
            referrer: Some("https://x/page".into()),
            filename: Some("y.zip".into()),
            mime: None,
            size: Some(10),
            cookie_header: Some("session=secret".into()),
            user_agent: Some("ua/1.0".into()),
            request_headers: vec![RequestHeader {
                name: "X-Api-Key".into(),
                value: "k".into(),
            }],
            tab_id: None,
            page_url: None,
        };

        let shown = redacted_for_prompt(&job);
        assert_eq!(shown.cookie_header, None);
        assert!(shown.request_headers.is_empty());
        assert_eq!(shown.final_url, job.final_url);

        let id = "ask-test-1".to_string();
        match dispatch(
            &core,
            Inbound::AskHandoff {
                id: id.clone(),
                job: job.clone(),
            },
        )
        .await
        {
            Outbound::Ack { .. } => {}
            other => panic!("expected Ack, got {other:?}"),
        }
        let held = take_handoff(&id).await.expect("job held under its id");
        assert_eq!(held.cookie_header.as_deref(), Some("session=secret"));
        assert!(take_handoff(&id).await.is_none(), "handed out once");
    }

    /// The `DownloadTorrent` dispatch arm was once missing —
    /// the build broke at the Wave-3 merge and no test caught it (the core-side
    /// handoff test deliberately bypasses src-tauri). Drive the real
    /// `dispatch -> handle_download_torrent -> Core::add_download` path with a
    /// magnet and assert it Acks a torrent row, and that an identical second job
    /// de-dups to the same row (Q7) rather than minting a new one. No network:
    /// magnet `add_download` only inserts a row (the worker is never started).
    #[tokio::test]
    async fn dispatch_download_torrent_magnet_acks_and_dedups() {
        use unduhin_core::wire::{Inbound, Outbound, TorrentJob};

        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path().join("pipe-torrent.db"))
            .await
            .unwrap();

        let magnet = "magnet:?xt=urn:btih:6f84758b0ddd8dc05840bf932a77935d8b5b8b93&dn=debian.iso";
        let job = || TorrentJob {
            magnet: Some(magnet.to_string()),
            torrent_file_b64: None,
            page_url: None,
            tab_id: None,
            suggested_filename: None,
        };

        let id = match dispatch(&core, Inbound::DownloadTorrent { job: job() }).await {
            Outbound::Ack { id } => id,
            other => panic!("expected Ack, got {other:?}"),
        };
        let rec = core.get_download(id).await.unwrap();
        assert_eq!(rec.kind, unduhin_core::DownloadKind::Torrent);

        // Q7: an identical magnet must return the SAME row, not a new id.
        match dispatch(&core, Inbound::DownloadTorrent { job: job() }).await {
            Outbound::Ack { id: dup } => {
                assert_eq!(dup, id, "duplicate magnet must de-dup to the same row")
            }
            other => panic!("expected Ack on duplicate, got {other:?}"),
        }
    }
}
