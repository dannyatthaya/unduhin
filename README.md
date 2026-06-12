# Unduhin

A focused, segmented download manager for Windows. Fast on healthy
servers, polite on small ones.

> _unduhin_ (verb, Bahasa Indonesia, colloquial): to download.

[![Windows](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue)](https://github.com/dannyatthaya/unduhin/releases)
[![Status](https://img.shields.io/badge/status-early%20preview-orange)](https://github.com/dannyatthaya/unduhin/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](#license)

---

Unduhin splits each download across multiple connections, resumes
cleanly after a flaky network or a reboot, and keeps your queue
organised in a fast UI. Paste a direct link and it streams down in
parallel segments; paste a YouTube/Twitter/etc. URL and it hands off to
yt-dlp; paste a magnet or `.torrent` and it grabs that too.

Free, open source, single-developer, and Windows-only by design.

## Features

- **Multi-segment downloads** — splits a file into up to N segments
  (default 8) for a real 4–10× speed-up on servers that support HTTP
  Range, with a clean single-stream fallback when they don't.
- **Honest resume** — pause, crash, or reboot and downloads continue
  from the exact byte. Validates ETag / Last-Modified first and
  restarts cleanly if the file changed on the server.
- **Media URLs** — YouTube, Vimeo, Twitter/X, TikTok, Twitch, and the
  [~thousand other sites yt-dlp supports](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md).
  Pick best video+audio, audio-only, or a specific format. yt-dlp and
  ffmpeg install on demand — nothing ships bundled.
- **Torrents & magnets** — add a magnet link or a `.torrent` file and
  Unduhin manages the swarm alongside your HTTP downloads.
- **Queue & categories** — per-state filters, auto-sorting into
  categories with per-category folders, reordering, and per-download
  overrides.
- **Browser extension** — Chrome / Edge / Brave intercept in-progress
  downloads and hand them to Unduhin with cookies and headers intact.
- **Local-first** — no accounts, no licence checks, no telemetry by
  default, no ads. State lives in one SQLite file that survives
  reinstalls. See [PRIVACY.md](./PRIVACY.md).

## Download

Unduhin is in early preview — there's no signed installer on
[Releases](https://github.com/dannyatthaya/unduhin/releases) yet. When
the first build lands it'll be a per-user installer (no admin) that
auto-updates in place. For now, [build from source](#build-from-source).

## Build from source

You'll need [Rust](https://rustup.rs/) (stable, MSVC toolchain),
[Bun](https://bun.sh/), and the Tauri v2 CLI.

```powershell
cargo install tauri-cli --version "^2.0" --locked
bun install --cwd frontend
bun install --cwd extension
bun run --cwd extension build   # required: extension/dist ships as a bundled resource
cargo tauri dev
```

Install the browser extension (once):

```powershell
# chrome://extensions → enable Developer mode → Load unpacked →
#   installed builds: %LOCALAPPDATA%\unduhin\extension
#   working in this repo: extension/dist
```

The app maintains `%LOCALAPPDATA%\unduhin\extension` itself: every
launch syncs the bundled extension into it, and a running extension
reloads itself when the version changes — load it once and updates are
automatic from then on. (If you loaded the extension from an older
release's zip, re-load it from that folder to start getting updates.)

[`CONTRIBUTING.md`](./CONTRIBUTING.md) covers the architecture, repo
tour, and the release/packaging scripts.

## Where things live

- **Downloads** go to your configured folder (per-category and
  per-download overrides apply).
- **Queue, history, and settings** live in
  `%LOCALAPPDATA%\unduhin\unduhin.db`.
- **Logs** rotate in `%LOCALAPPDATA%\unduhin\logs\`; yt-dlp and ffmpeg
  install into `%LOCALAPPDATA%\unduhin\binaries\`.
- **The browser extension** you Load-unpacked lives in
  `%LOCALAPPDATA%\unduhin\extension\` — managed by the app, refreshed on
  every launch.

Uninstalling leaves `%LOCALAPPDATA%\unduhin\` in place so a reinstall
resumes where you left off — delete it by hand to start clean.

## License

Dual-licensed under MIT or Apache 2.0, at your option. See
[`LICENSE-MIT`](./LICENSE-MIT) and [`LICENSE-APACHE`](./LICENSE-APACHE).
Bundled runtime tools (yt-dlp, ffmpeg) and library licences are listed
in-app under Settings → About → Open-source licences.

## Acknowledgements

Built on [Tauri](https://tauri.app/), [Rust](https://www.rust-lang.org/),
[Vue](https://vuejs.org/), [Tailwind](https://tailwindcss.com/),
[yt-dlp](https://github.com/yt-dlp/yt-dlp), and
[FFmpeg](https://ffmpeg.org/). Made with care in Jakarta.
</content>
</invoke>
