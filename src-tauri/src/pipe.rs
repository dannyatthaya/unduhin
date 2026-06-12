//! In-app named-pipe server. Accepts framed `Inbound` JSON messages
//! from the native-messaging host (`unduhin-native-host.exe`)
//! and dispatches them onto the live [`Core`].
//!
//! The pipe runs on the same tokio runtime as the rest of the Tauri
//! app. Multiple host sessions can be in flight at once — each
//! accepted connection is handled on its own task. Single-instance
//! enforcement (already wired in `lib.rs`) guarantees exactly one
//! pipe server is alive, so the well-known `\\.\pipe\unduhin` name
//! never collides.
//!
//! The pipe is Windows-only by design — a Linux / macOS host is not
//! yet implemented. The
//! [`install`] function compiles into a no-op on other targets so
//! the rest of the app continues to build cross-platform.

#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::{Arc, OnceLock};

use tauri::AppHandle;
#[cfg(windows)]
use tokio::io::WriteHalf;
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(windows)]
use tokio::sync::Mutex as AsyncMutex;
#[cfg(windows)]
use unduhin_core::wire::ExtensionSettings;
#[cfg(windows)]
use unduhin_core::wire::HandoffDecision;
#[cfg(windows)]
use unduhin_core::wire::RuleMetric;
use unduhin_core::Core;

#[cfg(windows)]
use tauri::Emitter;

/// `true` once the in-app named-pipe server has bound the well-known
/// path and is accepting client connections. Set exactly once per
/// process; the Settings → Browser status card reads this through
/// [`crate::browser_integration::pipe_status`].
#[cfg(windows)]
static PIPE_LISTENING: AtomicBool = AtomicBool::new(false);

/// Live write halves of every connected pipe client, used by the
/// settings-changed broadcast. Each entry is wrapped in its own
/// async mutex so [`broadcast_settings_changed`] can fan out
/// concurrently without two writers stepping on each other.
///
/// Per-process singleton: the accept loop pushes on each new
/// connection; the per-connection task removes its slot on hang-up.
#[cfg(windows)]
type ClientWriter = AsyncMutex<WriteHalf<NamedPipeServer>>;
#[cfg(windows)]
static CONNECTED_CLIENTS: OnceLock<AsyncMutex<Vec<Arc<ClientWriter>>>> = OnceLock::new();

#[cfg(windows)]
fn connected_clients() -> &'static AsyncMutex<Vec<Arc<ClientWriter>>> {
    CONNECTED_CLIENTS.get_or_init(|| AsyncMutex::new(Vec::new()))
}

/// Cached "last-known" extension settings. Populated whenever the
/// extension pushes via [`Inbound::SetSettings`] (or on the extension
/// bridge's connect-time full push). [`Inbound::GetSettings`] returns
/// this; if nothing has been pushed yet the default shape is returned
/// — it matches the extension's own `DEFAULT_SETTINGS` byte-for-byte
/// so the panel doesn't lie about the user's choices.
#[cfg(windows)]
static SETTINGS_CACHE: OnceLock<AsyncMutex<Option<ExtensionSettings>>> = OnceLock::new();

#[cfg(windows)]
fn settings_cache() -> &'static AsyncMutex<Option<ExtensionSettings>> {
    SETTINGS_CACHE.get_or_init(|| AsyncMutex::new(None))
}

/// Cached per-rule metrics snapshot pushed by the extension's alarm
/// tick (`Inbound::RuleMetrics`). The Tauri panel reads via
/// `get_rule_metrics`; the cache is replaced (not merged) on every
/// push so the extension's local store is the source of truth.
#[cfg(windows)]
static RULE_METRICS_CACHE: OnceLock<AsyncMutex<Vec<RuleMetric>>> = OnceLock::new();

#[cfg(windows)]
fn rule_metrics_cache() -> &'static AsyncMutex<Vec<RuleMetric>> {
    RULE_METRICS_CACHE.get_or_init(|| AsyncMutex::new(Vec::new()))
}

/// Read-only accessor for the cached rule-metrics snapshot. Returns
/// an empty vec until the first push arrives.
#[cfg(windows)]
pub async fn cached_rule_metrics() -> Vec<RuleMetric> {
    rule_metrics_cache().lock().await.clone()
}

#[cfg(not(windows))]
pub async fn cached_rule_metrics() -> Vec<unduhin_core::wire::RuleMetric> {
    Vec::new()
}

/// Read the cached settings if any. Used by future Tauri commands
/// (9e's `apply_extension_settings_patch`) and tests.
#[cfg(windows)]
pub async fn cached_extension_settings() -> Option<ExtensionSettings> {
    settings_cache().lock().await.clone()
}

/// Replace the cached settings. Called by the panel-driven
/// `apply_extension_settings_patch` Tauri command so a subsequent
/// `GetSettings` returns the panel's new shape without waiting for the
/// extension's `chrome.storage.onChanged` echo.
#[cfg(windows)]
pub async fn store_extension_settings(full: ExtensionSettings) {
    *settings_cache().lock().await = Some(full);
}

/// Broadcast a `SettingsChanged { full }` frame to every connected
/// pipe client. Best-effort: per-client write errors are logged and
/// the broken connection's writer eventually gets reaped by its
/// owning task. Public so Tauri commands can push panel-driven edits
/// back out to the extension.
#[cfg(windows)]
pub async fn broadcast_settings_changed(full: ExtensionSettings) {
    use unduhin_core::wire::framing::write_frame;
    use unduhin_core::wire::Outbound;

    let frame = match serde_json::to_vec(&Outbound::SettingsChanged { full }) {
        Ok(buf) => buf,
        Err(e) => {
            tracing::warn!(error = %e, "serialize SettingsChanged failed");
            return;
        }
    };
    // Snapshot the client list so we don't hold the outer lock across
    // any per-client write. The Arc<Mutex<WriteHalf>>s stay valid even
    // after the snapshot vec is dropped — broken connections are
    // pruned by their own per-connection task.
    let snapshot: Vec<Arc<ClientWriter>> = connected_clients().lock().await.clone();
    for client in snapshot {
        let mut writer = client.lock().await;
        if let Err(e) = write_frame(&mut *writer, &frame).await {
            tracing::debug!(error = %e, "broadcast write failed (client likely gone)");
        }
    }
}

#[cfg(not(windows))]
pub async fn broadcast_settings_changed(_full: unduhin_core::wire::ExtensionSettings) {}

/// Broadcast a `HandoffDecision { id, decision }` frame. Mirrors
/// [`broadcast_settings_changed`]: snapshot-then-fan-out so a slow
/// client only blocks its own per-writer mutex. The extension routes
/// the unsolicited frame back to the matching `ask-first` waiter by
/// `id`.
#[cfg(windows)]
pub async fn broadcast_handoff_decision(id: String, decision: HandoffDecision) {
    use unduhin_core::wire::framing::write_frame;
    use unduhin_core::wire::Outbound;

    let frame = match serde_json::to_vec(&Outbound::HandoffDecision { id, decision }) {
        Ok(buf) => buf,
        Err(e) => {
            tracing::warn!(error = %e, "serialize HandoffDecision failed");
            return;
        }
    };
    let snapshot: Vec<Arc<ClientWriter>> = connected_clients().lock().await.clone();
    for client in snapshot {
        let mut writer = client.lock().await;
        if let Err(e) = write_frame(&mut *writer, &frame).await {
            tracing::debug!(error = %e, "handoff broadcast write failed (client likely gone)");
        }
    }
}

#[cfg(not(windows))]
pub async fn broadcast_handoff_decision(
    _id: String,
    _decision: unduhin_core::wire::HandoffDecision,
) {
}

#[cfg(not(windows))]
pub async fn cached_extension_settings() -> Option<unduhin_core::wire::ExtensionSettings> {
    None
}

/// Test-only: wipe the settings cache so integration tests don't
/// leak state across `#[tokio::test]`s running in the same binary.
/// The cache is otherwise a per-process singleton (production only
/// runs one server per process).
#[cfg(windows)]
#[doc(hidden)]
pub async fn reset_settings_cache_for_tests() {
    *settings_cache().lock().await = None;
}

/// The bound pipe name, captured at first-accept time. Used by
/// [`crate::browser_integration::pipe_status`] so the UI can surface
/// the real path (handy when `UNDUHIN_PIPE_NAME` is set for a dev
/// override).
#[cfg(windows)]
static BOUND_PIPE_NAME: OnceLock<String> = OnceLock::new();

/// Snapshot of the pipe listener state read by the Settings → Browser
/// card. Returns `(name, listening)` — the name may still be set even
/// when `listening` is false on platforms that build with the no-op
/// stub.
#[cfg(windows)]
pub fn listening_snapshot() -> (Option<String>, bool) {
    (
        BOUND_PIPE_NAME.get().cloned(),
        PIPE_LISTENING.load(Ordering::Acquire),
    )
}

/// Cross-platform stub so the non-Windows build keeps linking. The
/// browser integration commands only exist on Windows but compile on
/// other targets too.
#[cfg(not(windows))]
pub fn listening_snapshot() -> (Option<String>, bool) {
    (None, false)
}

/// Resolved pipe path. `UNDUHIN_PIPE_NAME` is honoured so the
/// integration test can use a per-process random name and avoid
/// colliding with a real running app.
#[cfg(windows)]
pub fn pipe_name() -> String {
    std::env::var("UNDUHIN_PIPE_NAME").unwrap_or_else(|_| r"\\.\pipe\unduhin".to_string())
}

/// AppHandle stash so the `AskHandoff` dispatch can emit a frontend
/// event without threading the handle through every helper. Set once
/// in `install`; ignored in tests that drive `run_server` directly.
#[cfg(windows)]
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

#[cfg(windows)]
pub(crate) fn app_handle() -> Option<&'static AppHandle> {
    APP_HANDLE.get()
}

/// Windows pipe security: build a restrictive DACL so only the current
/// user (and LocalSystem) can connect to the pipe. Without this the pipe
/// inherits a default descriptor that lets *any* same-user process open
/// it and inject download jobs — historically combinable with the
/// filename path-traversal bug into an arbitrary-file-write primitive.
#[cfg(windows)]
mod pipe_security {
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
#[cfg(windows)]
pub fn install(app: AppHandle, core: Core) {
    let _ = APP_HANDLE.set(app);
    let name = pipe_name();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_server(name.clone(), core).await {
            tracing::warn!(error = %e, pipe = %name, "pipe server exited");
        }
    });
}

/// No-op stub so cross-platform builds (CI / Linux dev) still link.
#[cfg(not(windows))]
pub fn install(_app: AppHandle, _core: Core) {}

/// The actual accept loop. Exposed `pub` so the integration test in
/// `src-tauri/tests/pipe_smoke.rs` can drive it with a per-test pipe
/// name without needing a Tauri `AppHandle`. Production callers go
/// through [`install`].
#[cfg(windows)]
pub async fn run_server(name: String, core: Core) -> std::io::Result<()> {
    use std::time::Duration;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    // Build a restrictive security descriptor once; every pipe instance is
    // created with it so no other process — even one running as the same
    // user — can connect and inject download jobs. Fail closed: if we
    // cannot build the DACL we do not fall back to a permissive pipe.
    let security = pipe_security::PipeSecurity::current_user_only()?;

    // Create one pipe instance. The first instance owns the well-known
    // name; subsequent instances are created without `first_pipe_instance`
    // so they can co-exist. Each carries the restrictive DACL.
    let make = |first: bool| -> std::io::Result<NamedPipeServer> {
        let mut opts = ServerOptions::new();
        if first {
            opts.first_pipe_instance(true);
        }
        // SAFETY: `security` lives for the whole of `run_server`, so the
        // attributes pointer is valid for every create call below.
        unsafe { opts.create_with_security_attributes_raw(&name, security.as_attrs_ptr()) }
    };

    let mut server = make(true)?;

    tracing::info!(pipe = %name, "pipe server listening");

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

    // Recreate a pipe instance, retrying transient failures with capped
    // exponential backoff. A single listener error must not permanently
    // disable the bridge (the previous code propagated the error out of
    // the loop and never respawned), so this never gives up.
    async fn recreate(
        make: &impl Fn(bool) -> std::io::Result<NamedPipeServer>,
    ) -> NamedPipeServer {
        let mut backoff = Duration::from_millis(100);
        loop {
            match make(false) {
                Ok(server) => return server,
                Err(e) => {
                    tracing::warn!(error = %e, backoff_ms = backoff.as_millis(),
                        "failed to (re)create pipe instance; retrying");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            }
        }
    }

    loop {
        // Block until a client connects on the current server handle. On a
        // connect error the handle is unusable, so log and rebuild it
        // rather than tearing down the whole server.
        if let Err(e) = server.connect().await {
            tracing::warn!(error = %e, "pipe connect failed; recreating listener");
            server = recreate(&make).await;
            continue;
        }

        // Move the connected handle into the per-connection task and
        // immediately create a fresh server handle for the next client —
        // canonical tokio NamedPipeServer accept-loop shape.
        let connected = server;
        server = recreate(&make).await;

        let core = core.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(connected, core).await {
                tracing::debug!(error = %e, "pipe connection closed with error");
            }
        });
    }
}

#[cfg(windows)]
async fn handle_connection(stream: NamedPipeServer, core: Core) -> std::io::Result<()> {
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

#[cfg(windows)]
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
            if let Some(app) = app_handle() {
                if let Err(e) = app.emit(
                    "unduhin:ask-handoff",
                    AskHandoffPayload { id: &id, job: &job },
                ) {
                    tracing::warn!(error = %e, "failed to emit ask-handoff event");
                }
            } else {
                tracing::warn!("ask-handoff fired with no AppHandle — frontend will not prompt");
            }
            let _ = (id, job);
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

#[cfg(windows)]
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

#[cfg(windows)]
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

/// Extension torrent hand-off. The untrusted [`wire::TorrentJob`] (magnet URI
/// or base64 `.torrent` bytes) is validated and turned into an
/// `AddDownload { kind: Torrent, source: ExtensionPipe }` by
/// `core::torrent_handoff` — which size-limits / sanity-checks the payload and
/// writes any `.torrent` bytes into the managed dir under a content-hash name
/// (no caller-supplied path ever reaches the filesystem). We then hand it to
/// `Core::add_download`, which de-dups by info-hash and assigns the row id.
///
/// [`wire::TorrentJob`]: unduhin_core::wire::TorrentJob
#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(all(windows, test))]
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

    /// The `DownloadTorrent` dispatch arm is Windows-only and was once missing —
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

/// Re-export the pipe path so tests under `src-tauri/tests/` can
/// build a matching client. Kept module-public; the rest of the
/// app doesn't need it.
#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn default_pipe_path() -> PathBuf {
    PathBuf::from(pipe_name())
}
