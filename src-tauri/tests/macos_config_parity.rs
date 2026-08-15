//! Guards the one hazard created by having a macOS config overlay.
//!
//! Tauri merges `tauri.macos.conf.json` onto `tauri.conf.json` with JSON
//! Merge Patch (RFC 7396). Objects merge key by key, but **arrays are
//! replaced wholesale** — so `app.windows` cannot be a partial override and
//! has to restate every field. That means the window geometry now lives in
//! two places and can silently drift apart.
//!
//! Drift here is invisible on Windows and only shows up as a wrong-sized
//! window on a Mac, which nobody on this project can currently observe. So
//! assert it instead.

use serde_json::Value;

const BASE: &str = include_str!("../tauri.conf.json");
const MACOS: &str = include_str!("../tauri.macos.conf.json");

/// Fields that must be identical across both window definitions. Anything
/// deliberately different on macOS (`decorations`, `titleBarStyle`,
/// `hiddenTitle`) is excluded on purpose.
const SHARED_FIELDS: &[&str] = &[
    "label",
    "title",
    "width",
    "height",
    "minWidth",
    "minHeight",
    "resizable",
    "transparent",
    "visible",
];

fn main_window(config: &str, what: &str) -> Value {
    let parsed: Value = serde_json::from_str(config).expect("config is valid JSON");
    parsed["app"]["windows"]
        .as_array()
        .unwrap_or_else(|| panic!("{what} has no app.windows array"))
        .iter()
        .find(|w| w["label"] == "main")
        .unwrap_or_else(|| panic!("{what} has no window labelled \"main\""))
        .clone()
}

#[test]
fn macos_window_matches_base_geometry() {
    let base = main_window(BASE, "tauri.conf.json");
    let mac = main_window(MACOS, "tauri.macos.conf.json");

    for field in SHARED_FIELDS {
        assert_eq!(
            base.get(field),
            mac.get(field),
            "app.windows[main].{field} drifted between tauri.conf.json and \
             tauri.macos.conf.json. RFC 7396 replaces arrays wholesale, so \
             the macOS overlay must restate this field identically."
        );
    }
}

#[test]
fn macos_window_overrides_are_what_the_traffic_lights_need() {
    let mac = main_window(MACOS, "tauri.macos.conf.json");

    // `titleBarStyle: "Overlay"` is documented as requiring
    // `decorations: true`. With decorations off, macOS draws no traffic
    // lights at all and the window becomes unclosable by mouse.
    assert_eq!(
        mac["titleBarStyle"], "Overlay",
        "macOS expects the native traffic lights overlaid on the custom title bar"
    );
    assert_eq!(
        mac["decorations"], true,
        "titleBarStyle: Overlay requires decorations: true"
    );
    assert_eq!(
        mac["hiddenTitle"], true,
        "the custom title bar draws the title itself"
    );
}

#[test]
fn macos_bundle_drops_the_windows_native_host() {
    let mac: Value = serde_json::from_str(MACOS).expect("valid JSON");
    let resources = &mac["bundle"]["resources"];

    // `tauri_build::build()` validates every resource path and fails on a
    // missing file. The Windows `.exe` does not exist in a macOS build, so
    // it must be deleted — and under RFC 7396 an explicit null is what
    // deletes a key.
    assert!(
        resources["native-host/unduhin-native-host.exe"].is_null(),
        "the Windows .exe resource must be nulled out, or the macOS bundle fails to build"
    );
    assert_eq!(
        resources["native-host/unduhin-native-host"], "native-host/unduhin-native-host",
        "the macOS native host must be bundled in its place"
    );

    let targets = mac["bundle"]["targets"]
        .as_array()
        .expect("bundle.targets is an array");
    assert!(targets.iter().any(|t| t == "dmg"));
    assert!(targets.iter().any(|t| t == "app"));
}
