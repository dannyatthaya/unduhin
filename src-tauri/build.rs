fn main() {
    // Stage the `unduhin-native-host` binary alongside the
    // committed `native-host/com.unduhin.host.json` so Tauri's bundler
    // can pick both up via `bundle.resources`. Must run *before*
    // `tauri_build::build()` because that step validates every entry in
    // `bundle.resources` and fails if the file is missing.
    //
    // The NSIS hook that rewrites `PLACEHOLDER_ABS_PATH` to the real
    // install location lives in `nsis-hooks/hooks.nsi`; the
    // `manifest::reconcile_native_host_manifest` helper is the dev-build
    // / user-relocated fallback.
    #[cfg(target_os = "windows")]
    stage_native_host();

    tauri_build::build();
}

/// Build `unduhin-native-host` (release profile) and copy the resulting
/// `.exe` into `src-tauri/native-host/`. Uses a dedicated target dir so
/// the nested `cargo build` does not contend for the same lock the
/// outer build is holding.
#[cfg(target_os = "windows")]
fn stage_native_host() {
    use std::env;
    use std::path::PathBuf;
    use std::process::Command;

    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_dir
        .parent()
        .expect("workspace root one level above src-tauri")
        .to_path_buf();
    let native_host_src = workspace_root.join("crates").join("native-host");

    // Watch the host crate so a code change in `crates/native-host/`
    // re-runs this build.rs.
    println!(
        "cargo:rerun-if-changed={}",
        native_host_src.join("Cargo.toml").display()
    );
    let src_dir = native_host_src.join("src");
    walk_and_rerun(&src_dir);

    let staging_dir = crate_dir.join("native-host");
    std::fs::create_dir_all(&staging_dir).expect("create native-host staging dir");
    let dest = staging_dir.join("unduhin-native-host.exe");

    // Separate target dir so the nested cargo invocation has its own
    // lock and build cache. Shares CARGO_HOME so dependency downloads
    // are reused.
    let host_target = workspace_root.join("target").join("native-host-stage");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(&cargo)
        .args([
            "build",
            "-p",
            "unduhin-native-host",
            "--release",
            "--quiet",
            "--target-dir",
        ])
        .arg(&host_target)
        .current_dir(&workspace_root)
        .status()
        .expect("failed to spawn cargo for unduhin-native-host");
    assert!(
        status.success(),
        "nested `cargo build -p unduhin-native-host --release` failed"
    );

    let built = host_target.join("release").join("unduhin-native-host.exe");
    std::fs::copy(&built, &dest).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} → {}: {}",
            built.display(),
            dest.display(),
            e
        )
    });
}

#[cfg(target_os = "windows")]
fn walk_and_rerun(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_and_rerun(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
