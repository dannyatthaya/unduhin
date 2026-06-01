# Unduhin

A focused, segmented download manager for Windows. Fast on healthy
servers, polite on small ones.

> _unduhin_ (verb, Bahasa Indonesia, colloquial): to download.

[![Windows](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue)](https://github.com/dannyatthaya/unduhin/releases)
[![Status](https://img.shields.io/badge/status-early%20preview-orange)](https://github.com/dannyatthaya/unduhin/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](#license)

---

Unduhin is a modern alternative to the download managers from the
2000s. It splits each file across multiple connections, resumes
cleanly after a flaky network or a forced reboot, and keeps your queue
organised in a clean, fast UI. Paste a YouTube or Twitter URL and it
hands the job to yt-dlp behind the scenes; paste a direct download
link and it streams it down in parallel segments.

It's free, single-developer, open source, and Windows-only by design.

## Features

**Fast multi-segment downloads.** Unduhin probes the server, splits
the file into as many segments as you allow (default 8), and pulls them
in parallel. On healthy CDNs and mirrors that's a real, measurable
speed-up — sometimes 4–10x over a single-connection download.

**Honest resume.** Pull the network cable, kill the process, reboot —
when you come back, paused and interrupted downloads continue from the
exact byte they stopped at. Resume validates the remote file's ETag /
Last-Modified before continuing; if the file changed on the server,
Unduhin restarts cleanly instead of producing a corrupt mix.

**Polished queue and categories.** Queued, active, paused, completed,
failed, cancelled — each state has its own filter. Downloads
auto-sort into categories (Documents, Music, Video, Compressed,
Programs, Other) by extension, and each category can have its own
default folder. Edit, reorder, and re-rule any of it.

**Media URLs, not just files.** Paste a YouTube, Vimeo, Twitter,
TikTok, or any other URL yt-dlp supports. Unduhin probes it, shows you
the available formats with size estimates, and lets you pick:
"Best video + audio", "Audio only", or a specific format. yt-dlp and
ffmpeg are installed on demand from Settings → Media — nothing ships
bundled, so your install stays small.

**Settings that actually do something.** Default download folder,
default segment count, max concurrent downloads, retry policy,
connect / read timeouts, custom user-agent, dark mode, autostart,
close-to-tray (coming), and per-download overrides — all backed by a
single SQLite file that survives reinstalls.

**Built to leave you alone.** No accounts, no licence checks, no
telemetry by default, no ads. Crash reports and usage stats exist as
opt-in toggles in Settings → About; they're off until you turn them
on, and even on they never include URLs or filenames. See
[PRIVACY.md](./PRIVACY.md) for the full breakdown.

## Download

> **Note:** Unduhin is in early preview. There's no signed installer
> on the public Releases page yet. The sections below describe the
> shipping plan, and how to run a development build today.

When the first public release lands, you'll be able to grab a Windows
installer from
[github.com/dannyatthaya/unduhin/releases](https://github.com/dannyatthaya/unduhin/releases).
The installer is per-user (no admin needed), and Unduhin updates
itself in place — turn the toggle on in Settings → About and you'll
get fresh stable builds as they ship.

In the meantime, see [Build from source](#build-from-source).

## Quickstart

1. **Launch Unduhin.** The main window opens to the downloads list.
2. **Click "Add URL"** in the top bar, or press the keyboard shortcut.
3. **Paste a URL.** For a direct file (`.iso`, `.zip`, `.mp4`, etc.)
   Unduhin queues it immediately. For a media URL (YouTube, Twitter,
   etc.) you'll see a format picker first.
4. **Watch it go.** Progress, speed, and ETA update live. Click any
   row to see per-segment progress and a speed graph.
5. **Pause, resume, retry, cancel** from the row context menu, the
   batch action bar, or the detail panel.

## What you can paste

- **Direct files.** Anything served as a regular HTTP(S) download:
  `.iso`, `.zip`, `.exe`, `.mp4`, `.pdf`, `.7z`, `.tar.gz`, you name
  it. Servers that support HTTP Range requests get the full
  multi-segment treatment; servers that don't fall back to a clean
  single-stream download.
- **Media URLs** — whatever yt-dlp recognises, including:
  - YouTube (single videos; playlists are on the roadmap)
  - Vimeo
  - Twitter / X
  - TikTok
  - Twitch VODs
  - …and the [thousand-odd other sites yt-dlp
    supports](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md).
- **What it won't touch:**
  - DRM-protected services (Netflix, Disney+, etc.) — yt-dlp refuses
    these by design and Unduhin surfaces the refusal cleanly.
  - FTP, BitTorrent, magnet links.

## Where things live

- **Downloads** go to the folder you set as `Default download folder`
  in Settings → General, or whichever folder the category overrides
  to. Per-download overrides are remembered too.
- **Queue, history, and settings** sit in a single SQLite file at
  `%LOCALAPPDATA%\unduhin\unduhin.db`. Back it up and you'll have your
  full state on the next machine.
- **Logs** rotate daily in `%LOCALAPPDATA%\unduhin\logs\`. Drag this
  folder onto a bug report if you ever need to.
- **yt-dlp and ffmpeg** install into
  `%LOCALAPPDATA%\unduhin\binaries\` so they don't clutter your `PATH`.

Uninstalling Unduhin removes the program but leaves
`%LOCALAPPDATA%\unduhin\` in place so a reinstall picks up where you
left off. Delete that folder by hand if you want to start clean.

## Keyboard shortcuts

| Action                 | Shortcut         |
|------------------------|------------------|
| Add URL                | `Ctrl+N`         |
| Pause selected         | `Space`          |
| Open settings filter   | `Ctrl+F` (in Settings) |
| Show details panel     | Click the row    |

(Full shortcut sheet ships in a later release.)

## Build from source

Want to run today's `main`? You'll need
[Rust](https://rustup.rs/) (stable, MSVC toolchain), Node.js 20+, and
the Tauri v2 CLI.

```powershell
# one-time
cargo install tauri-cli --version "^2.0" --locked
npm --prefix frontend install

# run the dev app
cargo tauri dev
```

The first build downloads a lot of crates; subsequent runs are quick.

If you want to **package an installer** locally — for example to try
the auto-update flow end-to-end —
[`CONTRIBUTING.md`](./CONTRIBUTING.md) has the release scripts.

## Roadmap

Unduhin is built up feature by feature; the headlines so far:

- [x] **Multi-segment HTTP engine** with resume, ETag validation,
  exponential backoff, cancellation.
- [x] **Persistent queue, categories, settings** backed by SQLite.
- [x] **Tauri v2 desktop shell** with a Vue 3 + TypeScript frontend.
- [x] **Full Settings page** — general, behaviour, network,
  categories, appearance.
- [x] **yt-dlp integration** — probe, format picker, on-demand
  install.
- [x] **Installer, auto-updater, About page.**
- [x] **System tray, taskbar progress, scheduled downloads.**
- [x] **Indonesian and English UI** with a live language switcher.
- [x] **Browser extension** (Chrome / Edge / Brave) via native
  messaging.

## Browser extension

Unduhin ships a Chromium browser extension that intercepts in-progress
downloads, gathers their cookies / referer / user-agent / request
headers, cancels the browser's native download, and hands the job to
Unduhin via a Native Messaging host. It targets Chrome, Edge, and Brave
(Firefox is on the roadmap as its own iteration).

The installer registers the host under
`HKCU\Software\<browser>\NativeMessagingHosts\com.unduhin.host` for all
three browsers and drops the host binary plus its manifest at
`$INSTDIR\native-host\`. Once installed, load the unpacked extension
from `extension/dist/` (or install from the Chrome Web Store when it
ships):

```powershell
# Build the extension from source
cd extension
pnpm install
pnpm typecheck
pnpm build
# In chrome://extensions → enable Developer mode → Load unpacked →
# select extension/dist
```

The extension talks to the main app over a Windows named pipe
(`\\.\pipe\unduhin`). If Unduhin isn't running, the host launches it
detached and retries; if it can't, the browser's built-in download
proceeds and the extension shows a one-shot "Unduhin is not running"
notification.

**Permissions justification** (for Chrome Web Store review):

| Permission         | Why it's needed |
|--------------------|-----------------|
| `downloads`        | Detect and cancel the browser's built-in download so Unduhin can take over. |
| `webRequest`       | Capture request headers (referer, range hints, custom auth) so the engine can replay the same request the browser would have sent. |
| `nativeMessaging`  | Forward the captured job to the local Unduhin app over stdio. |
| `cookies`          | Build a `Cookie` header for auth-gated downloads so resumes and segment requests succeed. |
| `contextMenus`     | Add "Download with Unduhin" entries to link, image, and media right-click menus. |
| `storage`          | Persist user-configurable interception filters and the recent-jobs ring buffer. |
| `tabs`             | Resolve the active tab id for the popup snapshot and per-tab media stream maps. |
| `notifications`    | Surface the one-shot "Unduhin is not running" toast when the bridge is unreachable. |
| `webNavigation`    | Clear the per-tab media stream map on top-frame navigation. |
| `alarms`           | Health-check the native bridge every 30s while the service worker may be asleep. |
| `host_permissions: <all_urls>` | Media sniffing (HLS / DASH) requires `webRequest.onResponseStarted` on every host the user visits. |

Cookies and headers stay on the local machine — see
[PRIVACY.md](./PRIVACY.md). The extension makes no network calls of its
own; everything goes through the native app.

The Tauri shell also surfaces a dedicated **Settings → Browser** panel
that round-trips every extension setting (mode, file-type allowlist,
domain rules, behaviour toggles) without the user leaving the app.
Live handoff status, per-rule match counts, and an opt-in clipboard
watcher (see Privacy Policy) live there too.

## Privacy

Unduhin is local-first. It downloads what you ask it to and otherwise
keeps to itself. Two opt-in toggles in Settings → About control the
only outbound traffic that isn't a download:

- _Check for updates_ — fetches a small JSON manifest from GitHub.
- _Crash reports / usage stats_ — off by default. Neither is wired to
  a backend in this release.

The full policy is in [PRIVACY.md](./PRIVACY.md).

## Contributing

Bug reports and pull requests are welcome.
[`CONTRIBUTING.md`](./CONTRIBUTING.md) has the architecture overview,
prerequisites, repository tour, CLI reference, design notes, and
release process. If you're filing a bug, the "Copy diagnostic" button
in Settings → About produces a paste-ready block with the version,
build, commit, and OS.

## License

Dual-licensed under MIT or Apache 2.0, at your option. See
[`LICENSE-MIT`](./LICENSE-MIT) and [`LICENSE-APACHE`](./LICENSE-APACHE),
or refer to the SPDX headers in each crate's `Cargo.toml`.

Unduhin bundles open-source software at runtime (yt-dlp, ffmpeg) and
links against many MIT / Apache / ISC libraries. The full list is
viewable in-app under Settings → About → Open-source licences, and
can be exported as `NOTICE.txt`.

## Acknowledgements

Built on [Tauri](https://tauri.app/), [Rust](https://www.rust-lang.org/),
[Vue](https://vuejs.org/), [Tailwind](https://tailwindcss.com/),
[yt-dlp](https://github.com/yt-dlp/yt-dlp), and
[FFmpeg](https://ffmpeg.org/). Made with care in Jakarta.
