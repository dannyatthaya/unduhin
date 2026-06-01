//! Read-only Win32 surface for the Settings → Browser panel.
//!
//! Two responsibilities:
//!
//! 1. [`detect_installed_browsers`] probes `HKCU\Software\…\NativeMessagingHosts\com.unduhin.host`
//!    for each Chromium-family browser the NSIS hook registers.
//!    The result powers the "Browser extensions" card: a green dot when
//!    the registry key is live, amber when the browser appears installed
//!    but the key is missing (re-install needed), grey when the browser
//!    isn't installed at all.
//!
//! 2. [`pipe_status`] reads the [`crate::pipe::listening_snapshot`]
//!    pair so the "Listening for handoffs" card can show the real pipe
//!    path and whether the listener is bound, without polling.
//!
//! Both functions are infallible at the API level — registry I/O errors
//! degrade to "not detected" so a transient permission failure never
//! takes the whole settings page down.

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
    /// Placeholder. The Unduhin shell is Windows-only this
    /// release, so the Safari card never reports `installed: true`. The
    /// row is kept so the panel's grid renders the macOS-Q3 stub in a
    /// consistent slot.
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

/// Live state of the named-pipe handoff bridge.
#[derive(Debug, Clone, Serialize)]
pub struct PipeStatus {
    /// The bound pipe path, e.g. `\\.\pipe\unduhin`. `null` until the
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

/// HKCU subkey path under which a Chromium-family browser registers
/// its installation. Probed for the `installed` flag on `BrowserStatus`.
const fn install_key(id: BrowserId) -> Option<&'static str> {
    match id {
        BrowserId::Chrome => Some(r"Software\Google\Chrome"),
        BrowserId::Edge => Some(r"Software\Microsoft\Edge"),
        BrowserId::Brave => Some(r"Software\BraveSoftware\Brave-Browser"),
        // Firefox: the NM protocol layout differs (Mozilla writes
        // under `Software\Mozilla\NativeMessagingHosts`). Detect the
        // browser via its top-level key so the card can show
        // "installed, extension not shipped yet" in 9g.
        BrowserId::Firefox => Some(r"Software\Mozilla\Mozilla Firefox"),
        // Safari is macOS-only — no HKCU presence is ever expected on
        // Windows; the card surfaces a static "macOS in Q3" message.
        BrowserId::Safari => None,
    }
}

/// HKCU subkey path under which the installer wrote
/// `com.unduhin.host`. Only the Chromium-family browsers
/// the NSIS hook registers are wired.
const fn host_key(id: BrowserId) -> Option<&'static str> {
    Some(match id {
        BrowserId::Chrome => r"Software\Google\Chrome\NativeMessagingHosts\com.unduhin.host",
        BrowserId::Edge => r"Software\Microsoft\Edge\NativeMessagingHosts\com.unduhin.host",
        BrowserId::Brave => {
            r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.unduhin.host"
        }
        // Firefox isn't covered by the NSIS matrix.
        BrowserId::Firefox => return None,
        // Safari is macOS-only — no host registration on Windows.
        BrowserId::Safari => return None,
    })
}

/// Minimal read-only registry probe. Implemented by [`Win32Probe`] in
/// production and by `MockProbe` (in tests) to keep the detection
/// matrix unit-testable without touching the real HKCU.
pub trait RegistryProbe {
    /// `true` when the requested subkey exists under `HKEY_CURRENT_USER`.
    fn hkcu_key_exists(&self, subkey: &str) -> bool;
}

/// Production probe — calls `RegOpenKeyExW` against the live HKCU.
#[derive(Debug, Default, Clone, Copy)]
pub struct Win32Probe;

#[cfg(windows)]
impl RegistryProbe for Win32Probe {
    fn hkcu_key_exists(&self, subkey: &str) -> bool {
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

/// No-op probe so the non-Windows build still compiles. Always returns
/// `false`, which renders every Chromium card as "not installed" — the
/// shell is Windows-only this release.
#[cfg(not(windows))]
impl RegistryProbe for Win32Probe {
    fn hkcu_key_exists(&self, _subkey: &str) -> bool {
        false
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
pub(crate) fn detect_with<P: RegistryProbe>(probe: &P) -> Vec<BrowserStatus> {
    ALL_BROWSERS
        .iter()
        .copied()
        .map(|id| BrowserStatus {
            id,
            label: id.label(),
            family: id.family(),
            installed: install_key(id)
                .map(|k| probe.hkcu_key_exists(k))
                .unwrap_or(false),
            host_registered: host_key(id)
                .map(|k| probe.hkcu_key_exists(k))
                .unwrap_or(false),
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
        fn new<I: IntoIterator<Item = &'static str>>(keys: I) -> Self {
            Self {
                present: keys.into_iter().map(str::to_owned).collect(),
            }
        }
    }

    impl RegistryProbe for MockProbe {
        fn hkcu_key_exists(&self, subkey: &str) -> bool {
            self.present.contains(subkey)
        }
    }

    #[test]
    fn detects_chrome_and_edge_with_host_registered() {
        let probe = MockProbe::new([
            r"Software\Google\Chrome",
            r"Software\Google\Chrome\NativeMessagingHosts\com.unduhin.host",
            r"Software\Microsoft\Edge",
            r"Software\Microsoft\Edge\NativeMessagingHosts\com.unduhin.host",
        ]);
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
        let probe = MockProbe::new([r"Software\BraveSoftware\Brave-Browser"]);
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
        let probe = MockProbe::new(std::iter::empty::<&'static str>());
        let rows = detect_with(&probe);
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, ALL_BROWSERS.to_vec());
    }

    #[test]
    fn safari_card_is_never_installed_on_windows() {
        // Even if some adversarial key somehow exists in HKCU, the Safari
        // row should stay flagged as not-installed because the install
        // probe table returns `None` for it. This guards the macOS-Q3
        // stub card against accidental "Installed" UI states.
        let probe = MockProbe::new([r"Software\Apple Computer, Inc.\Safari"]);
        let rows = detect_with(&probe);
        let safari = rows.iter().find(|r| r.id == BrowserId::Safari).unwrap();
        assert!(!safari.installed);
        assert!(!safari.host_registered);
        assert_eq!(safari.family, BrowserFamily::Safari);
    }
}
