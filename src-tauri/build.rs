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
    // / user-relocated fallback. macOS has no installer, so there the
    // reconcile helper is the only thing that writes the manifests.
    //
    // NOTE: `cfg(target_os)` in a build script describes the *host*, not
    // the target being compiled for. Dispatch on `CARGO_CFG_TARGET_OS`,
    // which cargo sets to the real target.
    match std::env::var("CARGO_CFG_TARGET_OS")
        .unwrap_or_default()
        .as_str()
    {
        "windows" => stage_native_host_windows(),
        "macos" => stage_native_host_macos(),
        _ => {}
    }

    tauri_build::build();
}

/// Build `unduhin-native-host` (release profile) for the Windows target
/// and copy the resulting `.exe` into `src-tauri/native-host/`.
fn stage_native_host_windows() {
    let ctx = StageContext::new();
    let target = std::env::var("TARGET").expect("TARGET is set for build scripts");
    build_host(&ctx, &target);
    let built = ctx
        .host_target
        .join(&target)
        .join("release")
        .join("unduhin-native-host.exe");
    let dest = ctx.staging_dir.join("unduhin-native-host.exe");
    std::fs::copy(&built, &dest).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} -> {}: {}",
            built.display(),
            dest.display(),
            e
        )
    });
}

/// Build `unduhin-native-host` for **both** Apple triples and `lipo` them
/// into one universal binary.
///
/// This cannot follow the Windows shape of "build for `TARGET`".
/// `universal-apple-darwin` is not a rustc target: the Tauri CLI compiles
/// the app once per arch and `lipo`s the two results. So this build script
/// runs twice, with `TARGET` set to a *thin* triple each time. Staging the
/// host for `TARGET` alone would ship a thin host inside a universal app,
/// and the extension bridge would be dead on the other architecture —
/// Rosetta translates x86_64 to arm64, never the reverse.
///
/// Building both arches on each pass is mildly wasteful and obviously
/// correct: the second pass finds both thin builds cached and rewrites
/// identical bytes.
fn stage_native_host_macos() {
    const TRIPLES: [&str; 2] = ["x86_64-apple-darwin", "aarch64-apple-darwin"];

    let ctx = StageContext::new();
    let mut slices = Vec::new();
    for triple in TRIPLES {
        build_host(&ctx, triple);
        slices.push(
            ctx.host_target
                .join(triple)
                .join("release")
                .join("unduhin-native-host"),
        );
    }

    let dest = ctx.staging_dir.join("unduhin-native-host");
    let status = std::process::Command::new("lipo")
        .arg("-create")
        .args(&slices)
        .arg("-output")
        .arg(&dest)
        .status()
        .expect("failed to spawn `lipo` (install the Xcode command line tools)");
    assert!(
        status.success(),
        "`lipo -create` failed while building a universal unduhin-native-host"
    );
}

/// Paths and watch registrations shared by both staging paths.
struct StageContext {
    staging_dir: std::path::PathBuf,
    host_target: std::path::PathBuf,
    workspace_root: std::path::PathBuf,
}

impl StageContext {
    fn new() -> Self {
        use std::path::PathBuf;

        let crate_dir =
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
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
        walk_and_rerun(&native_host_src.join("src"));

        let staging_dir = crate_dir.join("native-host");
        std::fs::create_dir_all(&staging_dir).expect("create native-host staging dir");

        // Separate target dir so the nested cargo invocation has its own
        // lock and build cache. Shares CARGO_HOME so dependency downloads
        // are reused.
        let host_target = workspace_root.join("target").join("native-host-stage");

        Self {
            staging_dir,
            host_target,
            workspace_root,
        }
    }
}

/// Run the nested `cargo build` for one target triple.
///
/// `unduhin-native-host` depends on `unduhin-core` only. It must never gain
/// a path dependency on `unduhin-app`, or this nested build becomes an
/// infinite recursion.
fn build_host(ctx: &StageContext, triple: &str) {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = std::process::Command::new(&cargo)
        .args([
            "build",
            "-p",
            "unduhin-native-host",
            "--release",
            "--quiet",
            "--target",
            triple,
            "--target-dir",
        ])
        .arg(&ctx.host_target)
        .current_dir(&ctx.workspace_root)
        .status()
        .expect("failed to spawn cargo for unduhin-native-host");
    assert!(
        status.success(),
        "nested `cargo build -p unduhin-native-host --release --target {triple}` failed"
    );
}

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
