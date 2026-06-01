# Privacy Policy

_Last updated: 2026-05-28._

Unduhin is a Windows desktop application. It does as little networking
as possible, and what it does is listed here.

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
- Reads system information (Windows version, architecture, free disk
  space) for display in the UI. None of it is transmitted.

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

- **Send anonymous crash reports.** Sends a stack trace + Windows
  version when the app crashes. It does **not** send URLs, filenames,
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
  payload travels over stdio and a local Windows named pipe — it never
  leaves the machine.
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

## What Unduhin never does

- It does not phone home for licence checks.
- It does not embed analytics SDKs.
- It does not include third-party advertising.
- It does not read your clipboard unless you paste into it, or unless
  you explicitly turn on the optional clipboard watcher described above.
- It does not access your browser history, cookies, or other
  applications' state.

## Where the data lives

- Database: `%LOCALAPPDATA%\unduhin\unduhin.db`
- Logs: `%LOCALAPPDATA%\unduhin\logs\unduhin.log.YYYY-MM-DD`
- Managed tool binaries: `%LOCALAPPDATA%\unduhin\binaries\`
- Downloaded files: wherever you point them.

Uninstalling Unduhin removes the application from `%LOCALAPPDATA%\Programs\`
but **does not delete `%LOCALAPPDATA%\unduhin\`** so your queue history
and settings survive reinstalls. Delete the folder manually if you want
to start clean.

## Questions

File an issue at
[github.com/dannyatthaya/unduhin/issues](https://github.com/dannyatthaya/unduhin/issues).
