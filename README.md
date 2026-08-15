# Unduhin

A focused, segmented download manager for Windows and macOS. Fast on
healthy servers, polite on small ones.

> _unduhin_ (verb, Bahasa Indonesia, colloquial): to download.

[![Windows](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue)](https://github.com/dannyatthaya/unduhin/releases)
[![macOS](https://img.shields.io/badge/platform-macOS%2011%2B-lightgrey)](https://github.com/dannyatthaya/unduhin/releases)
[![Status](https://img.shields.io/badge/status-early%20preview-orange)](https://github.com/dannyatthaya/unduhin/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](#license)

---

Unduhin splits each download across multiple connections, resumes
cleanly after a flaky network or a reboot, and keeps your queue
organised in a fast UI. Paste a direct link and it streams down in
parallel segments; paste a YouTube/Twitter/etc. URL and it hands off to
yt-dlp; paste a magnet or `.torrent` and it grabs that too.

Free, open source, and single-developer.

Windows is the primary platform and gets the most testing. macOS support
is new and is built entirely in CI, because the maintainer does not own a
Mac. Treat the macOS build as a preview and please report what breaks.

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

### macOS: getting past Gatekeeper

The macOS build is **not signed or notarized**. Signing needs a paid Apple
Developer account, which this project does not have. macOS therefore
refuses to open the app after a browser download, and the message it shows
("Unduhin is damaged and can't be opened") is misleading — the app is
fine, it just carries a quarantine flag.

To install:

1. Open the `.dmg` and drag Unduhin to Applications.
2. Remove the quarantine flag:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Unduhin.app
   ```

3. Open the app normally.

Right-click then Open does not work for an unsigned app on recent macOS.
Verify the `.dmg` against the SHA-256 published in the release notes if
you want an integrity check in place of a signature.

## Build from source

You'll need [Rust](https://rustup.rs/) (stable), [Bun](https://bun.sh/),
and the Tauri v2 CLI. Windows needs the MSVC toolchain. macOS needs the
Xcode command line tools, which supply `cc`, `cmake`, and `lipo`.

```powershell
cargo install tauri-cli --version "^2.0" --locked
bun install --cwd frontend
bun install --cwd extension
bun run --cwd extension build   # required: extension/dist ships as a bundled resource
cargo tauri dev
```

On macOS, build a universal `.dmg` with:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo tauri build --target universal-apple-darwin --bundles app,dmg
```

Both targets are required. The build compiles each architecture and joins
them with `lipo`, and the native messaging host is built the same way.

Install the browser extension (once). Go to `chrome://extensions`, enable
Developer mode, then Load unpacked and pick the extension folder:

- installed builds, Windows: `%LOCALAPPDATA%\unduhin\extension`
- installed builds, macOS: `~/Library/Application Support/unduhin/extension`
- working in this repo: `extension/dist`

The app maintains that folder itself: every launch syncs the bundled
extension into it, and a running extension reloads itself when the
version changes — load it once and updates are automatic from then on.
(If you loaded the extension from an older release's zip, re-load it from
that folder to start getting updates.)

On macOS, use Settings → Browser → Open folder and drag the revealed
folder onto `chrome://extensions`. The Load-unpacked dialog hides
`~/Library`, so browsing to it by hand does not work.

[`CONTRIBUTING.md`](./CONTRIBUTING.md) covers the architecture, repo
tour, and the release/packaging scripts.

## Where things live

Everything sits under one app-data root:

- Windows: `%LOCALAPPDATA%\unduhin\`
- macOS: `~/Library/Application Support/unduhin/`

Inside it:

- **Downloads** go to your configured folder (per-category and
  per-download overrides apply), not here.
- **Queue, history, and settings** live in `unduhin.db`.
- **Logs** rotate in `logs/`; yt-dlp and ffmpeg install into `binaries/`.
- **The browser extension** you Load-unpacked lives in `extension/` —
  managed by the app, refreshed on every launch.

Removing the app leaves that folder in place so a reinstall resumes where
you left off — delete it by hand to start clean.

On macOS the folder is inside `~/Library`, which Finder hides by default.
Use Settings → Browser → Open folder to reveal it rather than browsing
there manually. Chrome's Load-unpacked dialog also hides it, so drag the
revealed folder onto `chrome://extensions` instead.

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
