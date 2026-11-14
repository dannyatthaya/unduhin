//! What to do when a download the browser handed over fails as "expired".
//!
//! A captured download that answers 401 / 403 / 410 — or serves a page in
//! place of the file — is usually not expired at all. The browser could fetch
//! it; the app's own client could not, for one of three reasons, each with
//! its own remedy. They are tried in order, each at most once per capture:
//!
//! 1. **The link was single-use.** The browser already spent the post-redirect
//!    URL before the extension cancelled its copy. Start again from the URL
//!    the user actually clicked, so the host issues a fresh one.
//! 2. **The session moved on.** Ask the extension for fresh cookies (the
//!    silent credential refresh that lives in the parent module).
//! 3. **The host only serves a real browser** (TLS / HTTP fingerprinting, a
//!    challenge page). Nothing the app sends will pass, so hand the download
//!    back: the extension starts it in the browser and lets it through.
//!
//! Only rows captured in this session, and only for a while after capture,
//! are eligible. A file that expires hours into a transfer is the "Refresh
//! link" flow's job — starting a browser download the user did not just ask
//! for would be a surprise, not a rescue.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;
use unduhin_core::wire::{DownloadJob, Outbound};
use unduhin_core::{Core, CoreEvent, DownloadId, ErrorKind};

/// How long after capture a failure still counts as "the hand-off did not
/// work" rather than "this link went stale later".
const RESCUE_WINDOW: Duration = Duration::from_secs(30 * 60);

/// Most captures remembered at once; the oldest goes first past this.
const MAX_CAPTURES: usize = 512;

/// One download the extension handed over, and which remedies it has used.
struct Capture {
    /// The URL the user clicked, when a redirect led somewhere else.
    original_url: Option<String>,
    captured_at: Instant,
    handed_back: bool,
    /// The browser connection the capture arrived on. A hand-back goes only
    /// there: with two browsers connected, the other one has neither the
    /// session nor the user's attention.
    client: Option<Weak<super::ClientWriter>>,
}

fn captures() -> &'static AsyncMutex<HashMap<DownloadId, Capture>> {
    static CAPTURES: OnceLock<AsyncMutex<HashMap<DownloadId, Capture>>> = OnceLock::new();
    CAPTURES.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

/// The pre-redirect URL of `job`, when it is a usable alternative to the URL
/// the row was created with.
fn alternative_url(job: &DownloadJob) -> Option<String> {
    let original = job.original_url.trim();
    if original.is_empty() || original == job.final_url {
        return None;
    }
    let parsed = url::Url::parse(original).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
}

/// Remember that `id` was just created from a browser capture.
pub async fn note_capture(id: DownloadId, job: &DownloadJob) {
    let mut guard = captures().lock().await;
    let now = Instant::now();
    guard.retain(|_, c| now.duration_since(c.captured_at) < RESCUE_WINDOW);
    while guard.len() >= MAX_CAPTURES {
        let Some(oldest) = guard
            .iter()
            .min_by_key(|(_, c)| c.captured_at)
            .map(|(id, _)| *id)
        else {
            break;
        };
        guard.remove(&oldest);
    }
    guard.insert(
        id,
        Capture {
            original_url: alternative_url(job),
            captured_at: now,
            handed_back: false,
            client: None,
        },
    );
}

/// Record the connection capture `id` arrived on. No-op for a row
/// [`note_capture`] has not seen.
pub(super) async fn set_client(id: DownloadId, client: &Arc<super::ClientWriter>) {
    if let Some(capture) = captures().lock().await.get_mut(&id) {
        capture.client = Some(Arc::downgrade(client));
    }
}

/// Forget `id`: it completed, was removed, or the user took it over by hand.
pub async fn forget(id: DownloadId) {
    captures().lock().await.remove(&id);
}

/// Take the pre-redirect URL for `id`, if it has one that was not tried yet.
async fn take_original_url(id: DownloadId) -> Option<String> {
    let mut guard = captures().lock().await;
    let capture = guard.get_mut(&id)?;
    if capture.captured_at.elapsed() >= RESCUE_WINDOW {
        return None;
    }
    capture.original_url.take()
}

/// Claim the one hand-back for `id`. `None` when the row is not a recent
/// capture or was already handed back; otherwise the connection it arrived
/// on, if that is known and still open.
async fn claim_hand_back(id: DownloadId) -> Option<Option<Arc<super::ClientWriter>>> {
    let mut guard = captures().lock().await;
    match guard.get_mut(&id) {
        Some(c) if !c.handed_back && c.captured_at.elapsed() < RESCUE_WINDOW => {
            c.handed_back = true;
            Some(c.client.as_ref().and_then(Weak::upgrade))
        }
        _ => None,
    }
}

/// Is `event` one [`on_core_event`] acts on? Lets the caller skip spawning a
/// task for the progress ticks that make up nearly all of the stream.
pub fn wants(event: &CoreEvent) -> bool {
    matches!(
        event,
        CoreEvent::Failed {
            error_kind: ErrorKind::ExpiredAuth,
            ..
        } | CoreEvent::Completed { .. }
            | CoreEvent::Removed { .. }
    )
}

/// React to one core event: rescue a capture that failed as "expired", and
/// forget a row that finished or was removed — its next failure is a new
/// situation and deserves its own automatic tries.
pub async fn on_core_event(core: &Core, event: &CoreEvent) {
    match event {
        CoreEvent::Failed {
            id,
            error_kind: ErrorKind::ExpiredAuth,
            ..
        } => on_expired(core, *id).await,
        CoreEvent::Completed { id, .. } | CoreEvent::Removed { id } => {
            super::forget_credential_refresh(*id).await;
            forget(*id).await;
        }
        _ => {}
    }
}

/// A download just failed with [`ErrorKind::ExpiredAuth`]. Run the next
/// remedy that has not been used on it yet.
async fn on_expired(core: &Core, id: DownloadId) {
    if retry_from_original(core, id).await {
        return;
    }
    if let Some(token) = super::begin_credential_refresh(id).await {
        match core.get_download(id).await {
            Ok(rec) => super::broadcast_refresh_credentials(token, id, rec.url).await,
            Err(e) => {
                tracing::debug!(id, error = %e, "credential refresh: row vanished");
                super::forget_credential_refresh(id).await;
            }
        }
        return;
    }
    hand_back(core, id).await;
}

/// Remedy 1: restart the row from the URL the user clicked. `true` when the
/// row was re-queued.
async fn retry_from_original(core: &Core, id: DownloadId) -> bool {
    let Some(original) = take_original_url(id).await else {
        return false;
    };
    // Full: the capture's headers ride along to the new URL.
    let headers = match core.get_download_full(id).await {
        Ok(rec) => rec.headers,
        Err(e) => {
            tracing::debug!(id, error = %e, "rescue: row vanished before the original-url retry");
            return false;
        }
    };
    // `force_restart`: whatever the dead link left on disk is not worth
    // keeping, and a size mismatch must not stop to ask the user.
    match core.refresh_source(id, &original, headers, true).await {
        Ok(_) => {
            tracing::info!(id, "rescue: restarted from the pre-redirect URL");
            true
        }
        Err(e) => {
            tracing::debug!(id, error = %e, "rescue: the pre-redirect URL did not work either");
            false
        }
    }
}

/// Remedy 3: ask the extension to download the file in the browser.
///
/// Best-effort and at most once per capture, and to ONE browser: the one the
/// capture came from, or — for an `ask-first` capture, whose row is created
/// by the app's own dialog — the most recently connected one. With no
/// extension connected the row simply stays failed, as it would have.
pub async fn hand_back(core: &Core, id: DownloadId) {
    let Some(origin) = claim_hand_back(id).await else {
        return;
    };
    let client = match origin {
        Some(client) => client,
        None => match super::latest_client().await {
            Some(client) => client,
            None => return,
        },
    };
    let url = match core.get_download(id).await {
        Ok(rec) => rec.url,
        Err(e) => {
            tracing::debug!(id, error = %e, "rescue: row vanished before the hand-back");
            return;
        }
    };
    tracing::info!(id, "rescue: handing the download back to the browser");
    super::send_to(
        &client,
        Outbound::DownloadInBrowser {
            download_id: id,
            url,
        },
        "DownloadInBrowser",
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(final_url: &str, original_url: &str) -> DownloadJob {
        DownloadJob {
            final_url: final_url.into(),
            original_url: original_url.into(),
            referrer: None,
            filename: None,
            mime: None,
            size: None,
            cookie_header: None,
            user_agent: None,
            request_headers: Vec::new(),
            tab_id: None,
            page_url: None,
        }
    }

    #[test]
    fn alternative_url_needs_a_different_http_url() {
        let same = job("https://a.example/f", "https://a.example/f");
        assert_eq!(alternative_url(&same), None);

        let redirected = job("https://cdn.example/t/123", "https://a.example/get/9");
        assert_eq!(
            alternative_url(&redirected).as_deref(),
            Some("https://a.example/get/9")
        );

        assert_eq!(alternative_url(&job("https://a.example/f", "")), None);
        assert_eq!(
            alternative_url(&job("https://a.example/f", "blob:https://a.example/x")),
            None
        );
    }

    #[tokio::test]
    async fn each_remedy_is_claimed_once() {
        // Ids well away from anything another test in this binary creates.
        let id: DownloadId = 9_000_001;
        note_capture(
            id,
            &job("https://cdn.example/t/1", "https://a.example/get/1"),
        )
        .await;

        assert_eq!(
            take_original_url(id).await.as_deref(),
            Some("https://a.example/get/1")
        );
        assert_eq!(take_original_url(id).await, None);

        assert!(claim_hand_back(id).await.is_some());
        assert!(claim_hand_back(id).await.is_none());

        forget(id).await;
        assert!(claim_hand_back(id).await.is_none());
    }

    #[tokio::test]
    async fn a_row_that_was_never_captured_is_not_rescued() {
        let id: DownloadId = 9_000_002;
        assert_eq!(take_original_url(id).await, None);
        assert!(claim_hand_back(id).await.is_none());
    }
}
