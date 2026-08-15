<#
.SYNOPSIS
    Launch an isolated Unduhin "dev channel" that runs alongside the installed
    production app without disturbing it.

.DESCRIPTION
    A plain `cargo tauri dev` cannot coexist with an installed release: they
    share the single-instance mutex, the named pipe, the native-messaging host
    registration, and the whole %LOCALAPPDATA%\unduhin data root. This script
    gives the dev build its own copy of every one of those:

        single-instance mutex   com.unduhin.app.dev-sim   (tauri.dev.conf.json)
        named pipe              \\.\pipe\unduhin-dev      (UNDUHIN_PIPE_NAME)
        data root               %LOCALAPPDATA%\unduhin-dev (UNDUHIN_DATA_ROOT)
        native-messaging host   com.unduhin.host.dev      (own HKCU keys)
        host binary             target\dev-host\
        autostart Run key       "Unduhin Dev"
        WebView2 profile        %LOCALAPPDATA%\com.unduhin.app.dev
        browser profile         .dev-browser\[browser]

    The production registration `com.unduhin.host` is never read or written.

    The environment variables are set in this process, so every child inherits
    them -- including the native host that the dev Chrome spawns. That
    inheritance is what keeps browser handoffs on the dev pipe.

.PARAMETER Reseed
    Delete the dev data root and re-copy it from production before launching.
    Without this, seeding happens only on the first run (when the dev root does
    not exist yet).

.PARAMETER NoSeed
    Start from an empty dev data root instead of copying production state.

.PARAMETER Force
    Allow seeding while the production app is running. Not recommended: the
    SQLite snapshot may be torn, and any download that was in flight will be
    resumed by the dev app into the same output path production is writing.

.PARAMETER SkipExtensionBuild
    Skip `bun run --cwd extension build`. Only safe if extension/dist is current.

.PARAMETER NoBrowser
    Do not launch the dev browser profile; just run the app.

.PARAMETER Browser
    Which Chromium browser to launch the dev profile in: auto (default; tries
    Chrome, then Brave, then Edge), chrome, brave, or edge. The dev host is
    registered for all three, so any of them works.

.PARAMETER Unregister
    Remove the three dev native-messaging registry keys and exit. Does not
    touch the production keys or any data.

.EXAMPLE
    .\scripts\dev.ps1
    First run: seeds from production, registers the dev host, starts the app
    and the dev browser profile.

.EXAMPLE
    .\scripts\dev.ps1 -Reseed
    Refresh the dev database from production to reproduce a live bug. Quit the
    production app first so the snapshot is clean.

.NOTES
    One-time manual step, in the dev browser profile only:
    open the Unduhin extension's Options page and set
    "Native host name" to com.unduhin.host.dev.

    Never sign the dev browser profile into Google. Both profiles run the same
    extension ID, so chrome.storage.sync would propagate that host name back to
    your real browser and point it at the dev build. --disable-sync is passed
    as a guard; leave it in place.
#>

[CmdletBinding()]
param(
    [switch]$Reseed,
    [switch]$NoSeed,
    [switch]$Force,
    [switch]$SkipExtensionBuild,
    [switch]$NoBrowser,
    [ValidateSet("auto", "chrome", "brave", "edge")] [string]$Browser = "auto",
    [switch]$Unregister
)

$ErrorActionPreference = "Stop"

$RepoRoot         = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$DevHostName      = "com.unduhin.host.dev"
$ProdRoot         = Join-Path $env:LOCALAPPDATA "unduhin"
$DevRoot          = Join-Path $env:LOCALAPPDATA "unduhin-dev"
$DevHostDir       = Join-Path $RepoRoot "target\dev-host"
$DevHostExe       = Join-Path $DevHostDir "unduhin-native-host.exe"
$DevHostManifest  = Join-Path $DevHostDir "$DevHostName.json"
$DevProfileRoot   = Join-Path $RepoRoot ".dev-browser"
$DevConfig        = Join-Path $RepoRoot "src-tauri\tauri.dev.conf.json"
$ProdHostManifest = Join-Path $RepoRoot "src-tauri\native-host\com.unduhin.host.json"

# Mirrors the browser matrix in src-tauri/nsis-hooks/hooks.nsi, but scoped to
# the dev host name so the production entries are untouched.
$DevHostKeys = @(
    "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$DevHostName",
    "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\$DevHostName",
    "HKCU:\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\$DevHostName"
)

function Write-Step([string]$Message) {
    Write-Host $Message -ForegroundColor DarkGray
}

# serde_json (and Chrome's manifest parser) reject a UTF-8 BOM with
# "expected value at line 1 column 1". PowerShell 5.1's Out-File -Encoding utf8
# writes one, so go through .NET with the BOM suppressed -- same reason
# bump-version.ps1 avoids Set-Content for tauri.conf.json.
function Write-Utf8NoBom([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText($Path, $Content, (New-Object System.Text.UTF8Encoding($false)))
}

function Test-ProductionRunning {
    # The installed bundle renames the binary to Unduhin.exe (productName);
    # the dev build stays unduhin-app.exe, so this only matches production.
    return $null -ne (Get-Process -Name "Unduhin" -ErrorAction SilentlyContinue)
}

function Remove-DevHostKeys {
    foreach ($key in $DevHostKeys) {
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
            Write-Step "  removed $key"
        }
    }
}

# --- Unregister short-circuit ------------------------------------------------

if ($Unregister) {
    Write-Host "=== Unduhin dev channel: unregister ===" -ForegroundColor Cyan
    Remove-DevHostKeys
    Write-Host "Dev native-messaging host deregistered." -ForegroundColor Green
    Write-Host "Data root $DevRoot and profiles under $DevProfileRoot were left in place."
    return
}

# --- Dev environment ---------------------------------------------------------
# Set before anything is launched: cargo, the app, and Chrome all inherit it,
# and the native host Chrome spawns inherits it from Chrome.

$env:UNDUHIN_DATA_ROOT = $DevRoot
$env:UNDUHIN_PIPE_NAME = '\\.\pipe\unduhin-dev'

Write-Host "=== Unduhin dev channel ===" -ForegroundColor Cyan
Write-Host ("Data root: {0}" -f $env:UNDUHIN_DATA_ROOT)
Write-Host ("Pipe:      {0}" -f $env:UNDUHIN_PIPE_NAME)
Write-Host ("Host name: {0}" -f $DevHostName)
Write-Host ""

# --- 1. Seed the dev data root from production -------------------------------

Write-Step "[1/7] Seed dev data root"

$needsSeed = (-not $NoSeed) -and ($Reseed -or -not (Test-Path -LiteralPath $DevRoot))

if (-not $needsSeed) {
    if ($NoSeed) {
        Write-Step "  -NoSeed: starting from whatever is already in the dev root"
    } else {
        Write-Step "  dev root already exists; pass -Reseed to refresh it from production"
    }
} elseif (-not (Test-Path -LiteralPath $ProdRoot)) {
    Write-Step "  no production data at $ProdRoot; starting fresh"
} else {
    if ((Test-ProductionRunning) -and (-not $Force)) {
        throw @"
The production app is running, so it is not safe to seed from it:
  * its SQLite database is open, so the copy could be torn, and
  * any download in flight would be resumed by the dev app into the same
    output file production is currently writing.

Quit Unduhin from the tray (right-click -> Exit), re-run this script, then
start production again. It only needs to be closed for the copy itself.

Pass -Force to seed anyway.
"@
    }

    if ($Reseed -and (Test-Path -LiteralPath $DevRoot)) {
        # A running dev app holds unduhin.db open, so the delete would fail
        # halfway and leave a half-seeded root behind.
        if (Get-Process -Name "unduhin-app" -ErrorAction SilentlyContinue) {
            throw "A dev app (unduhin-app.exe) is already running. Close it before -Reseed."
        }
        Write-Step "  -Reseed: removing $DevRoot"
        Remove-Item -LiteralPath $DevRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $DevRoot -Force | Out-Null

    # Copy the database with its WAL sidecars -- the -wal file holds committed
    # pages that have not been checkpointed back into the .db yet, so copying
    # the .db alone can silently lose recent rows.
    foreach ($name in @("unduhin.db", "unduhin.db-wal", "unduhin.db-shm")) {
        $src = Join-Path $ProdRoot $name
        if (Test-Path -LiteralPath $src) {
            Copy-Item -LiteralPath $src -Destination (Join-Path $DevRoot $name) -Force
            Write-Step "  copied $name"
        }
    }

    # torrents/ holds fastresume state keyed to the rows we just copied, so it
    # has to travel with the database. binaries/ (yt-dlp, ffmpeg) is copied to
    # save re-downloading them.
    #
    # Deliberately NOT copied:
    #   logs/       -- noise, and the dev app writes its own
    #   extension/  -- extension_sync rebuilds it from the dev bundle on startup
    foreach ($name in @("torrents", "binaries")) {
        $src = Join-Path $ProdRoot $name
        if (Test-Path -LiteralPath $src) {
            # Name the destination explicitly: `-Destination $DevRoot` merges
            # into an existing same-named folder instead of replacing it.
            Copy-Item -LiteralPath $src -Destination (Join-Path $DevRoot $name) -Recurse -Force
            Write-Step "  copied $name\"
        }
    }

    Write-Host "  Seeded from production." -ForegroundColor Green
    Write-Host "  Note: rows that were queued or active in the snapshot will resume in" -ForegroundColor Yellow
    Write-Host "        the dev app, writing to the paths recorded on those rows." -ForegroundColor Yellow
}

# --- 2. Build the extension --------------------------------------------------

Write-Step "[2/7] Build extension"
if ($SkipExtensionBuild) {
    Write-Step "  skipped (-SkipExtensionBuild)"
} else {
    bun run --cwd extension build
    if ($LASTEXITCODE -ne 0) { throw "extension build failed" }
}

# --- 3. Stage the dev native host --------------------------------------------

# Built into its own directory rather than reusing
# src-tauri\native-host\unduhin-native-host.exe: src-tauri/build.rs rewrites
# that file on every app build, and Windows file-locks a running executable, so
# a live dev host process there would break the next `cargo tauri dev`.

Write-Step "[3/7] Build + stage dev native host"
cargo build -p unduhin-native-host --release
if ($LASTEXITCODE -ne 0) { throw "cargo build -p unduhin-native-host failed" }

$builtHost = Join-Path $RepoRoot "target\release\unduhin-native-host.exe"
if (-not (Test-Path -LiteralPath $builtHost)) { throw "native host not found at $builtHost" }

New-Item -ItemType Directory -Path $DevHostDir -Force | Out-Null
Copy-Item -LiteralPath $builtHost -Destination $DevHostExe -Force
Write-Step "  staged $DevHostExe"

# --- 4. Write the dev manifest and register it -------------------------------

Write-Step "[4/7] Register $DevHostName"

# Take allowed_origins from the committed production manifest so the extension
# ID cannot drift from crates/core/src/wire.rs::ALLOWED_DEV_EXTENSION_ID.
if (-not (Test-Path -LiteralPath $ProdHostManifest)) {
    throw "Host manifest template not found at $ProdHostManifest"
}
$template = Get-Content -Raw -LiteralPath $ProdHostManifest | ConvertFrom-Json
$allowedOrigins = @($template.allowed_origins)
if ($allowedOrigins.Count -eq 0) { throw "No allowed_origins in $ProdHostManifest" }

$manifest = [ordered]@{
    name            = $DevHostName
    description     = "Unduhin native messaging host (dev channel)"
    path            = $DevHostExe
    type            = "stdio"
    allowed_origins = $allowedOrigins
}
Write-Utf8NoBom $DevHostManifest ($manifest | ConvertTo-Json -Depth 5)
Write-Step ("  manifest -> {0}" -f $DevHostManifest)
Write-Step ("  origins  -> {0}" -f ($allowedOrigins -join ", "))

foreach ($key in $DevHostKeys) {
    New-Item -Path $key -Force | Out-Null
    New-ItemProperty -LiteralPath $key -Name "(default)" -Value $DevHostManifest `
        -PropertyType String -Force | Out-Null
}
Write-Step "  registered for Chrome, Edge, Brave (production key untouched)"

# --- 5. Start the dev app ----------------------------------------------------

Write-Step "[5/7] Start app (cargo tauri dev)"
if (-not (Test-Path -LiteralPath $DevConfig)) { throw "Dev config not found at $DevConfig" }

$app = Start-Process -FilePath "cargo" `
    -ArgumentList @("tauri", "dev", "--config", $DevConfig) `
    -WorkingDirectory $RepoRoot -NoNewWindow -PassThru

# --- 6. Wait for the canonical dev extension folder --------------------------

# extension_sync::sync (src-tauri/src/lib.rs) copies the bundled extension into
# <data root>\extension on startup. Loading Chrome from there rather than from
# extension/dist mirrors production and keeps the app's reload-on-update
# broadcast working.

Write-Step "[6/7] Wait for dev extension folder"
$canonical = Join-Path $DevRoot "extension"
$deadline = (Get-Date).AddSeconds(120)
while (-not (Test-Path -LiteralPath (Join-Path $canonical "manifest.json"))) {
    if ($app.HasExited) { throw "The dev app exited before it staged its extension folder." }
    if ((Get-Date) -gt $deadline) { break }
    Start-Sleep -Milliseconds 500
}

if (Test-Path -LiteralPath (Join-Path $canonical "manifest.json")) {
    Write-Step "  ready: $canonical"
} else {
    $canonical = Join-Path $RepoRoot "extension\dist"
    Write-Host "  timed out; falling back to $canonical" -ForegroundColor Yellow
}

# --- 7. Launch the dev browser profile ---------------------------------------

Write-Step "[7/7] Launch dev browser profile"

# Chromium-family browsers all accept --user-data-dir / --load-extension, and
# the dev host is registered for all three, so any of them works. Probed in
# preference order unless -Browser pins one.
$BrowserCandidates = [ordered]@{
    chrome = @{
        Exe   = "chrome.exe"
        Paths = @("Google\Chrome\Application\chrome.exe")
    }
    brave  = @{
        Exe   = "brave.exe"
        Paths = @("BraveSoftware\Brave-Browser\Application\brave.exe")
    }
    edge   = @{
        Exe   = "msedge.exe"
        Paths = @("Microsoft\Edge\Application\msedge.exe")
    }
}

function Resolve-BrowserPath([hashtable]$Spec) {
    $roots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:LOCALAPPDATA)
    foreach ($root in $roots) {
        if (-not $root) { continue }
        foreach ($rel in $Spec.Paths) {
            $full = Join-Path $root $rel
            if (Test-Path -LiteralPath $full) { return $full }
        }
    }
    # Fall back to the App Paths registry -- covers non-default install
    # locations and per-machine installs the folder probe misses.
    foreach ($hive in @("HKLM:", "HKCU:")) {
        $key = "$hive\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\$($Spec.Exe)"
        if (-not (Test-Path -LiteralPath $key)) { continue }
        $resolved = (Get-ItemProperty -LiteralPath $key)."(default)"
        if ($resolved -and (Test-Path -LiteralPath $resolved)) { return $resolved }
    }
    return $null
}

function Find-DevBrowser([string]$Preference) {
    $names = if ($Preference -eq "auto") { @($BrowserCandidates.Keys) } else { @($Preference) }
    foreach ($name in $names) {
        $path = Resolve-BrowserPath $BrowserCandidates[$name]
        if ($path) { return [pscustomobject]@{ Name = $name; Path = $path } }
    }
    return $null
}

# Per-browser profile dir: a user-data-dir is not portable between browsers.
function Get-DevProfileDir([string]$Name) {
    return (Join-Path $DevProfileRoot $Name)
}

if ($NoBrowser) {
    Write-Step "  skipped (-NoBrowser)"
} else {
    $found = Find-DevBrowser $Browser
    if (-not $found) {
        $which = if ($Browser -eq "auto") { "No Chromium browser" } else { $Browser }
        Write-Host "  $which found. Launch one yourself with:" -ForegroundColor Yellow
        Write-Host ("    --user-data-dir=`"{0}`" --load-extension=`"{1}`" --disable-sync" -f (Get-DevProfileDir "manual"), $canonical)
    } else {
        $profileDir = Get-DevProfileDir $found.Name
        Start-Process -FilePath $found.Path -ArgumentList @(
            "--user-data-dir=$profileDir",
            "--load-extension=$canonical",
            "--no-first-run",
            "--no-default-browser-check",
            # Hard guard: both profiles run the same extension ID, so a synced
            # dev profile would push nativeHostName back to your real browser.
            "--disable-sync"
        ) | Out-Null
        Write-Step ("  launched {0} with profile {1}" -f $found.Name, $profileDir)
    }
}

Write-Host ""
Write-Host "Dev channel is up. Production is untouched." -ForegroundColor Green
Write-Host "First run only: in the dev browser profile, open the Unduhin extension's" -ForegroundColor Cyan
Write-Host ("Options page and set 'Native host name' to {0}." -f $DevHostName) -ForegroundColor Cyan
Write-Host ""
Write-Host "Ctrl+C stops the app. Run '.\scripts\dev.ps1 -Unregister' to deregister the dev host."

$app.WaitForExit()
