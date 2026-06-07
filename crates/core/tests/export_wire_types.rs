//! Generate `wire.d.ts` for the browser extension (and a mirror for the
//! frontend) from the Rust source of truth in `unduhin_core::wire`.
//!
//! Only compiled when the `ts-rs-export` feature is on:
//!
//! ```text
//! cargo test -p unduhin-core --features ts-rs-export export_wire_types
//! ```
//!
//! CI re-runs this test and `git diff --exit-code`s the produced files,
//! so an unintentional shape change in the Rust types breaks the build.

#![cfg(feature = "ts-rs-export")]

use std::path::{Path, PathBuf};

use ts_rs::TS;
use unduhin_core::wire::{
    DownloadJob, ExtensionSettings, HandoffDecision, HandoffMode, HostRule, Inbound, MediaKind,
    MediaStream, Outbound, RequestHeader, RuleMetric, SettingsPatch, StatusEntry, TorrentJob,
    ALLOWED_DEV_EXTENSION_ID, HOST_NAME,
};

/// Build the concatenated `.d.ts` body. Order matters — referenced types
/// must precede their referers, otherwise downstream TS compilers complain
/// about undefined names.
fn render() -> String {
    let header = "\
// AUTO-GENERATED FROM crates/core/src/wire.rs — DO NOT EDIT BY HAND.
// Run `cargo test -p unduhin-core --features ts-rs-export export_wire_types`
// to regenerate. Shape changes that aren't matched by a Rust-side change
// will fail CI via `git diff --exit-code`.

";

    let decls = [
        RequestHeader::decl(),
        MediaKind::decl(),
        MediaStream::decl(),
        DownloadJob::decl(),
        TorrentJob::decl(),
        StatusEntry::decl(),
        HandoffMode::decl(),
        HandoffDecision::decl(),
        HostRule::decl(),
        RuleMetric::decl(),
        ExtensionSettings::decl(),
        SettingsPatch::decl(),
        Inbound::decl(),
        Outbound::decl(),
    ];

    let mut out = String::with_capacity(header.len() + 4096);
    out.push_str(header);
    for decl in decls {
        // ts-rs v9 emits `type Foo = …;` — prepend `export ` so the
        // extension's strict TS imports actually find them.
        out.push_str("export ");
        out.push_str(&decl);
        out.push_str("\n\n");
    }

    out.push_str(&format!(
        "export const HOST_NAME = {:?} as const;\n\
         export const ALLOWED_DEV_EXTENSION_ID = {:?} as const;\n",
        HOST_NAME, ALLOWED_DEV_EXTENSION_ID,
    ));

    out
}

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` resolves to `<repo>/crates/core` for this
    // crate; two `..` jumps land at the workspace root.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // <repo>/
    p
}

fn write_if_changed(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return;
        }
    }
    std::fs::write(path, content).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
}

#[test]
fn export_wire_types() {
    let body = render();
    let root = workspace_root();

    // Source of truth for the extension. Stable path because the
    // extension imports from `../shared/wire`.
    let ext_path = root.join("extension/src/shared/wire.d.ts");
    write_if_changed(&ext_path, &body);

    // Mirrored copy for the Tauri frontend — read-only reference today,
    // useful if a future caller wants to share the StatusEntry shape with
    // the downloads view.
    let fe_path = root.join("frontend/src/types/wire.d.ts");
    write_if_changed(&fe_path, &body);
}
