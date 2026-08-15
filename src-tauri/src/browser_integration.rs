//! Read-only detection surface for the Settings → Browser panel.
//!
//! Two responsibilities:
//!
//! 1. [`detect_installed_browsers`] checks, for each Chromium-family
//!    browser, whether it is present and whether `com.unduhin.host` is
//!    registered with it. On Windows that means probing
//!    `HKCU\Software\…\NativeMessagingHosts\com.unduhin.host`, which the
//!    NSIS hook writes. On macOS there is no installer and no registry, so
//!    it means checking for the browser's profile directory and the
//!    manifest file the app writes itself (see [`crate::manifest`]).
//!    Either way the result powers the "Browser extensions" card: green
//!    when the registration is live, amber when the browser appears
//!    installed but the registration is missing, grey when the browser is
//!    not there at all.
//!
//! 2. [`pipe_status`] reads the [`crate::pipe::listening_snapshot`]
//!    pair so the "Listening for handoffs" card can show the real
//!    endpoint and whether the listener is bound, without polling.
//!
//! Both functions are infallible at the API level — probe errors degrade
//! to "not detected" so a transient permission failure never takes the
//! whole settings page down.

use serde::Serialize;

pub const NM_HOST_NAME: &str = "com.unduhin.host";

/// Stable identifier the frontend uses to key its per-browser card
/// state. Matches the kebab-case slugs the mockup uses internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserId {
    Chrome,
    Edge,
    Brave,
    Firefox,
    /// Placeholder. Safari uses App Extensions rather than native
    /// messaging, which is what Unduhin's extension is built on, so this
    /// card never reports `installed: true` on any platform. The row is
    /// kept so the panel's grid renders a consistent slot.
    Safari,
}

impl BrowserId {
    pub const fn label(self) -> &'static str {
        match self {
            BrowserId::Chrome => "Chrome",
            BrowserId::Edge => "Edge",
            BrowserId::Brave => "Brave",
            BrowserId::Firefox => "Firefox",
            BrowserId::Safari => "Safari",
        }
    }

    /// Browser family. The Chromium family shares the
    /// NativeMessagingHosts protocol; Firefox uses a Mozilla-specific
    /// variant (out of scope); Safari is its own family
    /// and macOS-only.
    pub const fn family(self) -> BrowserFamily {
        match self {
            BrowserId::Chrome | BrowserId::Edge | BrowserId::Brave => BrowserFamily::Chromium,
            BrowserId::Firefox => BrowserFamily::Firefox,
            BrowserId::Safari => BrowserFamily::Safari,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFamily {
    Chromium,
    Firefox,
    Safari,
}

/// One row in the "Browser extensions" card.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserStatus {
    pub id: BrowserId,
    pub label: &'static str,
    pub family: BrowserFamily,
    /// The browser itself appears installed for the current user
    /// (presence of its top-level `HKCU\Software\…` key).
    pub installed: bool,
    /// `com.unduhin.host` is registered under this browser's
    /// `NativeMessagingHosts` key.
    pub host_registered: bool,
}

/// Live state of the handoff bridge.
#[derive(Debug, Clone, Serialize)]
pub struct PipeStatus {
    /// The bound endpoint — `\\.\pipe\unduhin` on Windows, a socket path
    /// such as `~/Library/Application Support/unduhin/unduhin.sock` on
    /// macOS. `null` until the
    /// listener has bound for the first time this process lifetime.
    pub name: Option<String>,
    /// `true` once the listener is bound and accepting connections.
    pub listening: bool,
}

/// All known browser slots. The Chromium trio follow the registry
/// matrix the NSIS hook writes; Firefox is a placeholder card
/// (out of scope).
pub const ALL_BROWSERS: &[BrowserId] = &[
    BrowserId::Chrome,
    BrowserId::Edge,
    BrowserId::Brave,
    BrowserId::Firefox,
    BrowserId::Safari,
];

/// Key identifying an installed browser, probed for the `installed` flag
/// on `BrowserStatus`.
///
/// A registry subkey on Windows. On macOS the browser's profile directory
/// under `Application Support`, which is a better signal than looking for
/// the `.app`: users install to `/Applications`, `~/Applications`, or
/// wherever Homebrew Cask puts things, so there is no single bundle path
/// to test. The profile directory is one location, and it is the same
/// directory the native-host manifest has to go into — so "is it
/// installed" and "can we register with it" become one question.
///
/// The trade-off is that a browser installed but never launched reads as
/// absent. For this card that is arguably the right answer.
#[cfg(windows)]
fn install_key(id: BrowserId) -> Option<String> {
    Some(
        match id {
            BrowserId::Chrome => r"Software\Google\Chrome",
            BrowserId::Edge => r"Software\Microsoft\Edge",
            BrowserId::Brave => r"Software\BraveSoftware\Brave-Browser",
            // Firefox: the NM protocol layout differs (Mozilla writes
            // under `Software\Mozilla\NativeMessagingHosts`). Detect the
            // browser via its top-level key so the card can show
            // "installed, extension not shipped yet".
            BrowserId::Firefox => r"Software\Mozilla\Mozilla Firefox",
            // Safari never appears on Windows.
            BrowserId::Safari => return None,
        }
        .to_string(),
    )
}

/// Registry subkey the installer wrote `com.unduhin.host` into, or on
/// macOS the manifest path the app writes itself.
#[cfg(windows)]
fn host_key(id: BrowserId) -> Option<String> {
    Some(
        match id {
            BrowserId::Chrome => r"Software\Google\Chrome\NativeMessagingHosts\com.unduhin.host",
            BrowserId::Edge => r"Software\Microsoft\Edge\NativeMessagingHosts\com.unduhin.host",
            BrowserId::Brave => {
                r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.unduhin.host"
            }
            // Firefox isn't covered by the NSIS matrix.
            BrowserId::Firefox => return None,
            BrowserId::Safari => return None,
        }
        .to_string(),
    )
}

#[cfg(not(windows))]
fn app_support() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|home| {
        std::path::Path::new(&home)
            .join("Library")
            .join("Application Support")
    })
}

#[cfg(not(windows))]
fn profile_dir(id: BrowserId) -> Option<std::path::PathBuf> {
    let leaf = match id {
        BrowserId::Chrome => "Google/Chrome",
        BrowserId::Edge => "Microsoft Edge",
        BrowserId::Brave => "BraveSoftware/Brave-Browser",
        BrowserId::Firefox => "Firefox",
        // Safari stores nothing here and uses App Extensions rather than
        // native messaging, so it is out of scope on every platform.
        BrowserId::Safari => return None,
    };
    Some(app_support()?.join(leaf))
}

#[cfg(not(windows))]
fn install_key(id: BrowserId) -> Option<String> {
    profile_dir(id).map(|p| p.display().to_string())
}

#[cfg(not(windows))]
fn host_key(id: BrowserId) -> Option<String> {
    match id {
        // Mozilla's native-messaging layout differs and the extension is
        // Chromium-only, so Firefox is detected but never registered.
        BrowserId::Firefox | BrowserId::Safari => None,
        id => profile_dir(id).map(|p| {
            p.join("NativeMessagingHosts")
                .join("com.unduhin.host.json")
                .display()
                .to_string()
        }),
    }
}

/// Minimal read-only existence probe. Implemented against the registry on
/// Windows and the filesystem elsewhere, and by `MockProbe` in tests so
/// the detection matrix is unit-testable without touching either.
///
/// The `key` is an opaque platform-specific identifier: a registry subkey
/// on Windows, an absolute path on macOS.
pub trait IntegrationProbe {
    /// `true` when the identified key or path exists.
    fn key_exists(&self, key: &str) -> bool;
}

/// Production probe — calls `RegOpenKeyExW` against the live HKCU.
#[derive(Debug, Default, Clone, Copy)]
pub struct Win32Probe;

#[cfg(windows)]
impl IntegrationProbe for Win32Probe {
    fn key_exists(&self, subkey: &str) -> bool {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        };

        let wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut handle = HKEY::default();
        let pcwstr = PCWSTR(wide.as_ptr());
        // SAFETY: `wide` is a null-terminated UTF-16 buffer owned for
        // the duration of the call; the out-param is a stack HKEY we
        // close immediately on success.
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, pcwstr, Some(0), KEY_READ, &mut handle) };
        if status == ERROR_SUCCESS {
            // SAFETY: handle came from a successful Open and is
            // owned exclusively by this stack frame.
            let _ = unsafe { RegCloseKey(handle) };
            true
        } else {
            false
        }
    }
}

/// Filesystem probe used everywhere except Windows. The keys are absolute
/// paths, so existence is the whole test.
#[cfg(not(windows))]
impl IntegrationProbe for Win32Probe {
    fn key_exists(&self, key: &str) -> bool {
        std::path::Path::new(key).exists()
    }
}

/// Walk the [`ALL_BROWSERS`] list and resolve each to its current
/// installed / host-registered status. Uses `Win32Probe` directly so
/// callers don't have to pick a probe in production; tests should
/// use [`detect_with`] with a `MockProbe`.
pub fn detect_installed_browsers() -> Vec<BrowserStatus> {
    detect_with(&Win32Probe)
}

/// Same as [`detect_installed_browsers`] but with an explicit probe.
/// Kept `pub(crate)` so the unit test in this module can drive it.
pub(crate) fn detect_with<P: IntegrationProbe>(probe: &P) -> Vec<BrowserStatus> {
    ALL_BROWSERS
        .iter()
        .copied()
        .map(|id| BrowserStatus {
            id,
            label: id.label(),
            family: id.family(),
            installed: install_key(id)
                .map(|k| probe.key_exists(&k))
                .unwrap_or(false),
            host_registered: host_key(id).map(|k| probe.key_exists(&k)).unwrap_or(false),
        })
        .collect()
}

/// Snapshot the pipe-listener state. Reads the atomics set in
/// [`crate::pipe::install`] without any locking.
pub fn pipe_status() -> PipeStatus {
    let (name, listening) = crate::pipe::listening_snapshot();
    PipeStatus { name, listening }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// In-memory probe — every subkey passed to the constructor is
    /// reported as "present"; everything else is absent.
    struct MockProbe {
        present: HashSet<String>,
    }

    impl MockProbe {
        fn new<I: IntoIterator<Item = String>>(keys: I) -> Self {
            Self {
                present: keys.into_iter().collect(),
            }
        }

        /// Build the probe from the real key tables rather than from
        /// hardcoded strings. The tables are registry subkeys on Windows
        /// and filesystem paths on macOS, so literals would only ever
        /// match one platform — and the point of these tests is that the
        /// detection matrix behaves identically on both.
        fn with_installed<I: IntoIterator<Item = BrowserId>>(ids: I) -> Self {
            Self::new(ids.into_iter().filter_map(install_key))
        }

        fn with_installed_and_registered<I: IntoIterator<Item = BrowserId> + Clone>(
            ids: I,
        ) -> Self {
            let installed = ids.clone().into_iter().filter_map(install_key);
            let registered = ids.into_iter().filter_map(host_key);
            Self::new(installed.chain(registered))
        }
    }

    impl IntegrationProbe for MockProbe {
        fn key_exists(&self, key: &str) -> bool {
            self.present.contains(key)
        }
    }

    #[test]
    fn detects_chrome_and_edge_with_host_registered() {
        let probe = MockProbe::with_installed_and_registered([BrowserId::Chrome, BrowserId::Edge]);
        let rows = detect_with(&probe);
        let chrome = rows.iter().find(|r| r.id == BrowserId::Chrome).unwrap();
        assert!(chrome.installed);
        assert!(chrome.host_registered);
        let edge = rows.iter().find(|r| r.id == BrowserId::Edge).unwrap();
        assert!(edge.installed);
        assert!(edge.host_registered);
        let brave = rows.iter().find(|r| r.id == BrowserId::Brave).unwrap();
        assert!(!brave.installed);
        assert!(!brave.host_registered);
    }

    #[test]
    fn browser_installed_but_host_missing() {
        let probe = MockProbe::with_installed([BrowserId::Brave]);
        let rows = detect_with(&probe);
        let brave = rows.iter().find(|r| r.id == BrowserId::Brave).unwrap();
        assert!(brave.installed);
        assert!(
            !brave.host_registered,
            "host should be flagged missing — drives the amber card state"
        );
    }

    #[test]
    fn returns_every_known_browser_row_in_stable_order() {
        let probe = MockProbe::new(std::iter::empty::<String>());
        let rows = detect_with(&probe);
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, ALL_BROWSERS.to_vec());
    }

    #[test]
    fn safari_is_never_reported_as_integrated() {
        // Safari uses App Extensions rather than native messaging, so it
        // has no key on any platform and both tables return `None`. Even
        // an adversarial key must not flip the card to "Installed".
        let probe = MockProbe::new([
            r"Software\Apple Computer, Inc.\Safari".to_string(),
            "/Applications/Safari.app".to_string(),
        ]);
        let rows = detect_with(&probe);
        let safari = rows.iter().find(|r| r.id == BrowserId::Safari).unwrap();
        assert!(!safari.installed);
        assert!(!safari.host_registered);
        assert_eq!(safari.family, BrowserFamily::Safari);
    }

    #[test]
    fn firefox_is_detected_but_never_registered() {
        // The extension is Chromium-only and Mozilla's native-messaging
        // layout differs, so Firefox may show as installed but must never
        // claim a registered host.
        let probe = MockProbe::with_installed_and_registered([BrowserId::Firefox]);
        let rows = detect_with(&probe);
        let firefox = rows.iter().find(|r| r.id == BrowserId::Firefox).unwrap();
        assert!(!firefox.host_registered);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_keys_are_absolute_paths_under_application_support() {
        // Guards the shape the filesystem probe depends on: if these ever
        // became relative, `Path::exists` would resolve them against the
        // process CWD and quietly report nonsense.
        if std::env::var("HOME").is_err() {
            return; // No home directory in this environment; nothing to assert.
        }
        let key = install_key(BrowserId::Chrome).expect("chrome has an install key");
        assert!(key.starts_with('/'), "{key}");
        assert!(key.contains("Application Support"), "{key}");

        let host = host_key(BrowserId::Chrome).expect("chrome has a host key");
        assert!(
            host.ends_with("NativeMessagingHosts/com.unduhin.host.json"),
            "{host}"
        );
    }
}
