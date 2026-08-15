# Privacy Policy

_Last updated: 2026-05-28._

Unduhin is a desktop application for Windows and macOS. It does as little
networking as possible, and what it does is listed here.

Paths below are written in their Windows form. The macOS equivalent of
`%LOCALAPPDATA%\unduhin\` is `~/Library/Application Support/unduhin/`,
and every file named under it keeps the same name.

## What Unduhin always does

- Sends HTTP(S) requests to the URLs **you** supply (the file you're
  downloading and any redirects it resolves through).
- Stores its operational state (download history, queue, settings,
  categories, segment progress) in a local SQLite database at
  `%LOCALAPPDATA%\unduhin\unduhin.db`. Nothing in that database is sent
  anywhere — it's purely local.
- Writes rotating log files to `%LOCALAPPDATA%\unduhin\logs\`. URLs and
  filenames appear in those logs the same way they appear in the UI.
  You can delete the directory at any time.
- Reads system information (operating system version, architecture, free
  disk space) for display in the UI. None of it is transmitted.

## What Unduhin does only when you ask

- **Check for updates.** When you press "Check for updates" in
  Settings → About, or on startup if "Check for updates on startup"
  is on, the app downloads a small JSON manifest from the configured
  update endpoint (default: `github.com/dannyatthaya/unduhin/releases/...`).
  No identifying information is sent — just a standard HTTPS GET.
- **Install yt-dlp / FFmpeg.** When you press "Install" or "Update" in
  Settings → Media, the app downloads the pinned binary from its
  upstream release URL (yt-dlp's GitHub Releases / gyan.dev FFmpeg
  builds). The download is verified by running `--version` on the
  result. No identifying information is sent.
- **Download a media URL via yt-dlp.** When you paste a URL that yt-dlp
  recognizes, the app runs `yt-dlp --dump-single-json <url>` and then
  `yt-dlp ... -o <path> <url>`. Networking is done by yt-dlp itself —
  see [yt-dlp's privacy notes](https://github.com/yt-dlp/yt-dlp).

## What Unduhin can do, but only with your consent

These two switches in Settings → About are **off by default**. You must
turn them on for the corresponding network traffic to happen.

- **Send anonymous crash reports.** Sends a stack trace and the
  operating system version when the app crashes. It does **not** send
  URLs, filenames,
  proxy credentials, or anything from the downloads table.
- **Send anonymous usage statistics.** Sends feature counts and timing
  (e.g. "the user clicked Add URL 12 times this session") to help
  prioritize what to build next. It does **not** send the URLs you
  download, what you searched for, or anything from the downloads
  table.

Neither switch is wired to a real backend in this release. They exist
so the UX is honest about the toggles being opt-in only; when a backend
is added later, this document will be updated alongside it.

- **Watch the OS clipboard for download URLs.** When you turn on
  Settings → Browser → "Watch the clipboard for download URLs", Unduhin
  polls the Windows clipboard every ~1.5 seconds. Only HTTP(S) URLs
  whose final path extension matches your file-type allowlist surface
  a one-click capture toast; everything else is ignored. The clipboard
  text is read locally and is **not** sent over the network. Captured
  URLs are queued the same way an Add URL paste would be. Turn the
  toggle off to stop all clipboard reads.

## What the browser extension does

The optional Unduhin browser extension (Chrome / Edge / Brave)
intercepts in-progress browser downloads and hands them to
Unduhin via a Native Messaging host. It runs entirely on your machine
and **does not make any network calls of its own** — every fetch goes
through the main Unduhin app.

- Reads request headers from outgoing browser requests (via
  `chrome.webRequest`) so the engine can replay the same request the
  browser would have sent. The cache lives only in the service-worker
  session and is dropped when the worker sleeps.
- Reads cookies for a URL (via `chrome.cookies.getAll`) so auth-gated
  downloads can be resumed. Cookies are attached as the `Cookie` header
  on the captured job; they are not persisted anywhere by the extension
  itself.
- Sends the captured job (URL, filename, size, cookies, referer,
  user-agent, observed request headers, tab id, page URL) to the local
  Unduhin app over the `com.unduhin.host` Native Messaging host. The
  payload travels over stdio and then a local IPC channel — a named pipe
  on Windows, a Unix domain socket in the app data folder on macOS. It
  never leaves the machine. The socket is created private to your user
  account, and the app checks the connecting process's user id before
  accepting a connection.
- Stores a 5-entry ring buffer of recent jobs in
  `chrome.storage.session` so the popup can show "Recent downloads".
  This is in-memory only and clears when the browser is closed.
- Stores user-set interception filters (min file size, host rules,
  HLS/DASH toggles, native host name) in `chrome.storage.sync`. This
  syncs to your browser profile per Chrome's normal settings sync; it
  does not pass through any Unduhin server.

The extension has **no analytics, telemetry, or external network
calls.** Cookies and captured headers are exposed only to the local
Unduhin app, which uses them to perform the download you requested.

Once a job is queued by the extension, its captured headers are stored
in Unduhin's local SQLite database (`%LOCALAPPDATA%\unduhin\unduhin.db`)
under the `headers` column of the corresponding `downloads` row so
resume / segment requests can replay the same auth context. Delete the
row to forget the headers.

### How captured headers are protected at rest

Cookies and `Authorization` headers are encrypted before they reach the
database, so a backup, a synced profile, or another process reading the
SQLite file does not get them in the clear.

- **Windows** uses DPAPI, scoped to your Windows account on that machine.
- **macOS** keeps one AES-256-GCM key in your login Keychain and wraps
  the values with it.

Both degrade rather than fail: if encryption is unavailable the value is
stored as plain text and the download still works. The macOS build is
unsigned, and macOS ties a Keychain item to the creating binary's code
signature, so the system may ask for Keychain access again after an
update. Granting it restores encryption; declining falls back to plain
text.

### One difference on macOS: ffmpeg integrity

Downloaded tool binaries are verified against a SHA-256 before they are
installed or run, which stops a tampered mirror or an intercepting proxy
from substituting an executable. yt-dlp publishes checksums on both
platforms, so that check is identical everywhere.

The macOS ffmpeg build server publishes no checksums. Rather than trust
the transport alone, Unduhin pins one specific dated build per
architecture and carries its SHA-256 in the source, so the same
fail-closed check applies. The cost is that macOS ffmpeg updates arrive
when the pin is bumped rather than automatically.

## What Unduhin never does

- It does not phone home for licence checks.
- It does not embed analytics SDKs.
- It does not include third-party advertising.
- It does not read your clipboard unless you paste into it, or unless
  you explicitly turn on the optional clipboard watcher described above.
- It does not access your browser history, cookies, or other
  applications' state.

## Where the data lives

The app data root is `%LOCALAPPDATA%\unduhin\` on Windows and
`~/Library/Application Support/unduhin/` on macOS. Inside it:

- Database: `unduhin.db`
- Logs: `logs/unduhin.log.YYYY-MM-DD`
- Managed tool binaries: `binaries/`
- Downloaded files: wherever you point them, never here.

Removing the app does **not** delete that folder, so your queue history
and settings survive a reinstall. On Windows the uninstaller removes the
program from `%LOCALAPPDATA%\Programs\`; on macOS you drag
`Unduhin.app` to the Trash. Either way, delete the data folder manually
if you want to start clean.

## Questions

File an issue at
[github.com/dannyatthaya/unduhin/issues](https://github.com/dannyatthaya/unduhin/issues).
