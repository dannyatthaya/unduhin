# Contributing to Unduhin

Thanks for being curious enough to open this file. The
[README](./README.md) covers what Unduhin is and how to install it; the
**Architecture** section below is the fastest way to orient yourself in
the code.

## Workspace layout

```
unduhin/
├── Cargo.toml                       # workspace root
├── crates/
│   ├── engine/                      # download library (no Tauri, no UI)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── download.rs          # orchestrator + segment workers
│   │   │   ├── error.rs             # EngineError, Result
│   │   │   ├── http.rs              # probe(), RemoteInfo
│   │   │   ├── meta.rs              # `.unduhin-meta` sidecar
│   │   │   ├── progress.rs          # ProgressEvent + SpeedMeter
│   │   │   ├── retry.rs             # transient/terminal + backoff
│   │   │   └── segment.rs
│   │   └── tests/integration.rs
│   ├── core/                        # queue + persistence (package: unduhin-core)
│   │   ├── migrations/              # sqlx migrations
│   │   ├── src/
│   │   │   ├── lib.rs               # `Core` facade, public surface
│   │   │   ├── build_info.rs        # build-time git sha + timestamp
│   │   │   ├── logging.rs           # rotating file logger
│   │   │   ├── error.rs             # CoreError, Result
│   │   │   ├── event.rs             # CoreEvent
│   │   │   ├── download.rs          # DownloadRecord, statuses, repo
│   │   │   ├── category.rs          # categories + auto-categorize
│   │   │   ├── settings.rs          # k/v JSON settings + validation
│   │   │   ├── queue.rs             # QueueManager + per-download worker
│   │   │   ├── speed.rs             # TokenBucket
│   │   │   ├── tooling.rs           # yt-dlp / ffmpeg install + resolve
│   │   │   ├── ytdlp/               # yt-dlp probe + child-process driver
│   │   │   │   ├── mod.rs
│   │   │   │   ├── progress.rs
│   │   │   │   └── wire.rs
│   │   │   └── db.rs                # SqlitePool + migrations
│   │   └── tests/integration.rs
│   └── cli/                         # binary, output name `unduhin`
│       └── src/main.rs
├── src-tauri/                       # Tauri v2 shell (package: unduhin-app)
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── nsis-hooks/                  # installer registry hooks
│   ├── capabilities/default.json
│   └── src/
│       ├── main.rs                  # entry → unduhin_app_lib::run()
│       ├── lib.rs                   # tauri::Builder, command registry
│       ├── commands.rs              # one #[tauri::command] per Core method
│       └── error.rs
├── frontend/                        # Vue 3 + TS + Tailwind frontend
│   ├── package.json
│   ├── vite.config.ts
│   └── src/
│       ├── App.vue
│       ├── style.css                # CSS-variable theme
│       ├── types/tauri-bindings.ts  # typed contract: invoke + listen + types
│       ├── stores/                  # Pinia stores
│       ├── composables/             # reusable hooks
│       ├── generated/licences.json  # auto-generated; do not hand-edit
│       ├── views/
│       │   ├── DownloadsView.vue
│       │   └── settings/
│       └── components/
├── scripts/                         # PowerShell helpers (release, bumps)
├── .github/workflows/               # CI
└── PRIVACY.md
```

## Prerequisites

You'll need:

- **Windows 10 or 11.** Unduhin is Windows-only by design.
- **Rust** 1.75+ via [rustup](https://rustup.rs/). The MSVC toolchain
  is what Tauri builds against.
- **Node.js** 20+.
- **The Tauri CLI**: `cargo install tauri-cli --version "^2.0" --locked`.
- **WebView2 Runtime** — bundled with Windows 11; install separately on
  fresh Windows 10 images.

Optional, but useful:

- **`cargo-edit`** for `cargo set-version` (used by the version bump
  script): `cargo install cargo-edit`.
- **`cargo-watch`** for incremental dev runs:
  `cargo install cargo-watch`.

## Running the dev app

```powershell
# one-time
npm --prefix frontend install

# dev — launches the desktop window with HMR'd Vite frontend
cargo tauri dev
```

This spins up the Vite server on `127.0.0.1:5173` and opens the Tauri
window. Both halves hot-reload on save (frontend immediately,
Rust on `cargo tauri dev` re-running).

The dev shell opens the same SQLite database the CLI uses
(`%LOCALAPPDATA%\unduhin\unduhin.db` by default; override with
`UNDUHIN_DB`).

## CLI

The `unduhin` binary is the engine + queue exercised from the shell.
Useful for testing the backend without firing up the GUI.

```powershell
# build
cargo build --release

# one-shot transfer, bypassing the queue
cargo run --bin unduhin -- info     https://example.com/file.bin
cargo run --bin unduhin -- download https://example.com/file.bin -o .\file.bin -n 4
cargo run --bin unduhin -- resume   .\file.bin.unduhin-meta

# queue-backed verbs (persistent SQLite at %LOCALAPPDATA%\unduhin\unduhin.db
# unless overridden with --db <path>)
cargo run --bin unduhin -- add      https://example.com/file.bin
cargo run --bin unduhin -- list
cargo run --bin unduhin -- pause    1
cargo run --bin unduhin -- continue 1
cargo run --bin unduhin -- cancel   1
cargo run --bin unduhin -- retry    1
cargo run --bin unduhin -- remove   1               # forget the row, keep file
cargo run --bin unduhin -- remove   1 --with-data   # also delete the file
cargo run --bin unduhin -- category list
cargo run --bin unduhin -- settings list
cargo run --bin unduhin -- settings set max_concurrent_downloads 2
cargo run --bin unduhin -- daemon                   # runs the queue
```

`add` enqueues a URL; the actual transfer happens inside the daemon
process. Run `daemon` in one shell and `add` / `pause` / etc. in another;
both processes share the SQLite file and the daemon reconciles state
every ~500 ms.

### Example: pause + resume across daemon restart

```powershell
$db = "$env:TEMP\unduhin-demo.db"

# Terminal A
cargo run --bin unduhin -- --db $db daemon

# Terminal B
cargo run --bin unduhin -- --db $db add https://speed.hetzner.de/100MB.bin
cargo run --bin unduhin -- --db $db pause 1

# Terminal A: Ctrl-C the daemon. The sidecar (`100MB.bin.unduhin-meta`)
# stays on disk and the row stays `paused` in the database.

# Later
cargo run --bin unduhin -- --db $db daemon
cargo run --bin unduhin -- --db $db continue 1
```

## Architecture

```
       ┌──────────────────────────────┐
       │   Vue 3 + Pinia + Tailwind   │
       │   useUnduhinEvents()         │ <- listen("unduhin:event")
       │   api.* helpers              │ -> invoke("...")
       └────────────┬─────────┬───────┘
                    │         │
                    v         ^
       ┌──────────────────────┴──────┐
       │   src-tauri (unduhin-app)   │
       │   thin command wrappers     │
       └────────────┬─────────┬──────┘
                    │ Core::* │ Core::subscribe()
                    v         ^
              ┌────────────────────────┐
              │   unduhin-core         │
              │   queue + persistence  │
              │   + tooling + yt-dlp   │
              └────────────────────────┘
```

The Tauri shell wraps `unduhin_core::Core`. Its public methods are the
entire contract:

```rust
let core = Core::open(path).await?;
core.start().await?;                       // begins the queue manager loop
let mut events = core.subscribe();         // CoreEvent broadcast

// Downloads
core.add_download(AddDownload { … }).await?;
core.list_downloads(DownloadFilter { … }).await?;
core.get_download(id).await?;
core.pause(id).await?;
core.resume(id).await?;
core.cancel(id).await?;
core.retry(id).await?;
core.remove(id, delete_data).await?;
core.set_priority(id, priority).await?;

// Categories
core.list_categories().await?;
core.add_category(NewCategory { … }).await?;
core.update_category(id, NewCategory { … }).await?;
core.remove_category(id).await?;
core.set_category_order(ids).await?;

// Settings
core.get_setting(key).await?;
core.set_setting(key, SettingValue).await?;
core.all_settings().await?;

// Media (yt-dlp / ffmpeg)
core.probe_media_url(url).await?;
core.tool_status(tool).await;
core.install_tool(tool).await?;

core.shutdown().await?;
```

`CoreEvent` variants: `DownloadAdded`, `StatusChanged`, `ProgressUpdate`,
`Completed`, `Failed`, `Removed`, `CategoryChanged`, `PathsChanged`,
`SettingChanged`, `ToolInstallProgress`, `ToolInstallCompleted`,
`ToolInstallFailed`.

The `Status` enum: `queued | active | muxing | paused | completed | failed | cancelled`.

### Settings keys

Keys live in `unduhin_core::settings::settings_keys`:

- **Queue / files**: `max_concurrent_downloads`, `default_segments`,
  `global_speed_limit_bps`, `default_output_path`,
  `connect_timeout_secs`, `read_timeout_secs`.
- **Appearance / shell**: `theme_mode` (`light` / `dark` / `system`),
  `autostart`, `start_minimized`, `close_behavior`
  (`minimize` / `exit` / `ask`), `confirm_on_quit`,
  `delete_default_action` (`ask` / `row_only` / `row_and_data`).
- **Notifications**: `notify_complete`, `notify_fail`,
  `notify_queue_empty`.
- **Retries / HTTP**: `max_retries`, `retry_backoff_base_ms`,
  `user_agent`.
- **Media (yt-dlp)**: `ytdlp_binary_path`, `ffmpeg_binary_path`,
  `ytdlp_default_format`, `ytdlp_probe_timeout_ms`,
  `ytdlp_consent_accepted_at`.
- **Updates / telemetry**: `update_channel` (`stable` / `beta`),
  `update_check_on_startup`, `send_crash_reports`, `send_usage_stats`,
  `last_update_check_at`, `last_update_check_result`.

The settings table is open-ended — new keys don't require a migration,
but adding a `settings_keys` constant + a validator branch is
encouraged so the CLI and UI agree on the shape.

### Frontend conventions

- Composition API with `<script setup>` everywhere. No Options API.
- Components in PascalCase under `frontend/src/components/`.
- Composables prefixed `use` under `frontend/src/composables/`.
- Pinia stores prefixed `use`, suffixed `Store`
  (e.g. `useDownloadsStore`) under `frontend/src/stores/`.
- The typed contract with Rust lives in
  `frontend/src/types/tauri-bindings.ts`. Keep it in sync by hand with
  Serde-derived structs in `crates/core` and `src-tauri`.

### yt-dlp integration

When the user pastes a URL into the Add URL dialog, the frontend first
calls `probe_media_url`. If yt-dlp recognises it, a media dialog
appears with title / uploader / duration / thumbnail and a format
picker (Best video + audio / Audio only / Custom). On submit, the row
is saved with a `media_info` blob; the queue worker spawns yt-dlp (and
ffmpeg for muxed selectors), parses `--progress-template` lines as
`ProgressEvent`s, and updates the DB once yt-dlp prints the
`after_move:` line.

A few non-obvious choices documented in the code:

- The "Best video + audio" recommendation scores formats explicitly by
  `(height, tbr, filesize)` — yt-dlp's `formats` ordering isn't stable
  across extractors. Separate streams (`bv+ba`) win only when their
  resolution beats the best combined format.
  (`crates/core/src/ytdlp/wire.rs`)
- The worker passes `--merge-output-format mp4`; yt-dlp transparently
  falls back to `.mkv` for combinations that can't live in mp4
  (VP9/opus, AV1, ...). The final-path sync below keeps the DB honest
  either way.
- After a successful download the worker prefers
  `fs::metadata(final_path).len()` for the row's byte count — the
  progress template can't see the muxed output and would otherwise
  leave the row showing `0 B`. (`crates/core/src/ytdlp/mod.rs`)
- yt-dlp / ffmpeg binaries are discovered via
  `unduhin_core::tooling::resolve_path`: explicit setting → managed dir
  under `%LOCALAPPDATA%\unduhin\binaries\` → system PATH. Settings →
  Media drives `install_tool` to fetch the latest builds into the
  managed dir.

## Tests

```powershell
# Rust workspace
cargo test

# engine only
cargo test -p engine

# core only (unit + integration; spins up a hyper test server)
cargo test -p unduhin-core

# frontend
npm --prefix frontend test
npm --prefix frontend run typecheck
npm --prefix frontend run build
```

## Design notes

### Why the daemon polls the database

`Core::add_download` and friends modify rows directly; the queue
manager's ~500 ms reconciliation tick observes those changes and acts
on them. That gives us a single source of truth (the DB) and lets a
different process (the CLI) drive the queue without a separate IPC
layer. The Tauri shell runs the daemon in-process and pokes the queue
manager directly through `Core` whenever it mutates state, so it
doesn't pay the 500 ms latency.

### Why we never open multi-statement transactions for state changes

SQLite's default `BEGIN DEFERRED` deadlocks with any concurrent writer
the moment the transaction tries to upgrade from read to write — the
other connection's write lock blocks us, and SQLite refuses to wait
because giving up the read snapshot would violate isolation. We
sidestep this by issuing a single conditional UPDATE for every status
transition (`UPDATE … WHERE id = ? AND status = ?`). If the row's
status has moved since we read it, the UPDATE affects zero rows and we
re-fetch to produce a precise error.

### Why the global speed limit is wired but not enforced

The engine builds its own `reqwest::Client` and doesn't accept an
external byte gate. `unduhin_core::speed` exposes a working
`TokenBucket` and the setting persists, but there's no consumer yet.
Plumbing it through requires `engine::DownloadOptions` to grow a
`throttle: Option<Arc<dyn ByteGate>>` field — a small follow-up to the
engine.

### Auto-categorization

`AddDownload` may name a category or leave it unset. When unset, the
filename's extension is matched against each category's
`extension_rules` (lower-cased, no leading dot); first match wins.
If nothing matches the row is filed under "Other".

### Removing downloads: row only vs. row + file

The trash icon in any list / detail / batch surface routes through the
shared `useDeleteConfirm()` composable. It reads the
`delete_default_action` setting (`ask` / `row_only` / `row_and_data`)
and either runs the delete silently or pops a three-option modal
(Cancel / Remove entry / Remove entry & file). On the Rust side
`Core::remove(id, delete_data)` cancels any active worker, deletes the
row, and — when `delete_data` is true — best-effort removes the file
and the engine sidecar.

## Releasing

### Local dry run

```powershell
# Build NSIS + MSI installers, regenerate licences.json, and produce a
# Tauri-updater manifest under target\release\bundle\. Does
# not upload anything; safe to re-run.
scripts\release.ps1 -Version 0.2.0 -Channel stable -Notes "First public preview"

# Just rebuild the licences manifest:
scripts\generate-licences.ps1
```

### Tagged GitHub release

```powershell
scripts\bump-version.ps1 0.2.0
git add -A
git commit -m "chore: release v0.2.0"
git tag v0.2.0
git push --follow-tags
```

The push triggers `.github/workflows/release.yml`, which runs the
build, signs the artefacts if `vars.SIGN_CERT_THUMBPRINT` is set,
generates the updater manifest, and uploads everything to the matching
GitHub Release.

### First public release checklist

- [ ] `scripts\release.ps1` succeeds locally on a clean checkout.
- [ ] Tauri signing keypair generated (`tauri signer generate`) and
  pubkey written into `src-tauri/tauri.conf.json::plugins.updater.pubkey`.
- [ ] `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  set as GitHub secrets.
- [ ] (Optional) `SIGN_CERT_THUMBPRINT` configured as a GitHub variable
  for code-signing.
- [ ] `PRIVACY.md` reviewed and accurate.
- [ ] Release notes drafted in the GitHub Release editor.
- [ ] Tested install + auto-update on a clean Windows VM (10 + 11).
- [ ] Verified the Native Messaging hooks register/unregister
  correctly.

### Updater manifest

```jsonc
{
  "version": "0.2.0",
  "notes": "Markdown release notes.",
  "pub_date": "2026-05-25T10:30:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<minisign-style base64 signature>",
      "url": "https://github.com/.../Unduhin_0.2.0_x64-setup.nsis.zip"
    }
  }
}
```

The endpoint baked into the app
(`src-tauri/tauri.conf.json::plugins.updater.endpoints`) is
`latest-stable.json` at the GitHub Releases "latest" alias.

## Pull-request expectations

- Branch from `main`. Keep PRs focused — one logical change at a time.
- Cover Rust changes with unit and/or integration tests. Frontend
  changes that touch the event reducer or settings should have a
  Vitest case.
- Run the full test suite locally before opening the PR.
- If you change a `CoreEvent` variant, a Tauri command shape, or a
  settings key, update `frontend/src/types/tauri-bindings.ts` in the
  same PR.
- Discuss anything that touches the queue manager, the schema, or the
  native-messaging hooks before writing code — the wrong call is
  expensive to back out of.

## Filing a bug

Open an issue at
[github.com/dannyatthaya/unduhin/issues](https://github.com/dannyatthaya/unduhin/issues)
and include the "Copy diagnostic" payload from Settings → About — it
captures the version, build, commit, channel, and OS in one block.
