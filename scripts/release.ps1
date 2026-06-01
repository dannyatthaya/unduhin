<#
.SYNOPSIS
    Local release pipeline. Builds artefacts under target\release\bundle\.
    With -Publish, also commits the version bump, tags, pushes, and
    uploads the artefacts to a GitHub Release via the gh CLI.

.DESCRIPTION
    Mirrors what .github\workflows\release.yml used to do on CI, but runs
    on your own machine — orders of magnitude faster than the free
    windows-latest runner. Steps:

      1. Bump versions across Cargo / tauri.conf.json / package.json.
      2. `bun install --frozen-lockfile` in frontend/.
      3. Regenerate licences.json.
      4. cargo test --workspace.
      5. cargo tauri build --bundles nsis,msi.
      6. Sign artefacts (no-op without -Thumbprint).
      7. Generate latest-<channel>.json next to the bundle.
      8. (with -Publish) git commit + tag + push, then gh release create.

    Prerequisites for -Publish: `gh auth status` must show you logged in,
    and the working tree must be clean before running (the script will
    add the files it bumped, but won't try to reason about other diffs).

.PARAMETER Version
    Semver to release, e.g. 0.2.0.

.PARAMETER Channel
    "stable" or "beta". Drives the updater-manifest filename only.

.PARAMETER ReleaseUrlBase
    Base URL for the eventual GitHub release assets. Used inside the
    manifest. Defaults to the GitHub Releases path for this repo at this
    version.

.PARAMETER Thumbprint
    Optional code-signing thumbprint. Empty = unsigned build.

.PARAMETER Notes
    Markdown release notes embedded into the manifest.

.PARAMETER Publish
    Commit, tag, push, and upload to GitHub. Without this flag the script
    is a dry run that leaves artefacts under target\release\bundle\.

.PARAMETER Remote
    Git remote to push the tag to. Defaults to "origin".

.PARAMETER WebStoreExtensionId
    Optional Chrome Web Store extension ID. When set, the host manifest
    template at src-tauri/native-host/com.unduhin.host.json is rewritten
    so its `allowed_origins` references this ID instead of the dev-time
    unpacked extension ID. The original file is restored after the
    bundle step regardless of outcome, so the working tree stays clean.
    Until first Web Store submission, leave this empty — the dev ID
    will ship.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)] [string]$Version,
    [ValidateSet("stable", "beta")] [string]$Channel = "stable",
    [string]$ReleaseUrlBase = "",
    [string]$Thumbprint = "",
    [string]$Notes = "",
    [switch]$Publish,
    [string]$Remote = "origin",
    [string]$WebStoreExtensionId = ""
)

$ErrorActionPreference = "Stop"

if (-not $ReleaseUrlBase) {
    $ReleaseUrlBase = "https://github.com/dannyatthaya/unduhin/releases/download/v$Version"
}

if ($Publish) {
    # Fail fast before we burn 10 minutes on a build we can't ship.
    $ghCmd = Get-Command gh -ErrorAction SilentlyContinue
    if (-not $ghCmd) { throw "gh CLI not found on PATH. Install it from https://cli.github.com." }
    # Auth state is checked implicitly at upload time — gh release create
    # fails loudly if you're not logged in, and explicit pre-checks have
    # proven flaky across gh install variants on Windows.

    $dirty = git status --porcelain
    if ($dirty) {
        throw "Working tree is not clean. Commit or stash before running with -Publish:`n$dirty"
    }

    $existingTag = git tag --list "v$Version"
    if ($existingTag) { throw "Tag v$Version already exists locally. Delete it or pick another version." }
}

$mode = if ($Publish) { "publish" } else { "dry-run" }
Write-Host ("=== Unduhin release ({0}) ===" -f $mode) -ForegroundColor Cyan
Write-Host ("Version: {0}  Channel: {1}" -f $Version, $Channel)
Write-Host ""

# 1. Version bump
& "$PSScriptRoot\bump-version.ps1" -Version $Version

# 2. Install frontend deps strictly from the lockfile. Bun has no `ci`
#    subcommand; `bun install --frozen-lockfile` is the equivalent — it
#    installs from bun.lock and fails rather than writing the lockfile if
#    package.json and bun.lock have drifted.
Write-Host "[2/7] bun install --frozen-lockfile (frontend)" -ForegroundColor DarkGray
bun install --cwd frontend --frozen-lockfile
if ($LASTEXITCODE -ne 0) { throw "bun install --frozen-lockfile failed" }

# 3. Licence manifest
Write-Host "[3/7] regenerating licence manifest" -ForegroundColor DarkGray
& "$PSScriptRoot\generate-licences.ps1"

# 4. cargo test
Write-Host "[4/8] cargo test --workspace" -ForegroundColor DarkGray
cargo test --workspace
if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }

# 4.5. Extension build (Chromium native-messaging extension)
# Produces extension/dist/, which we zip into a release asset further
# down. Bun is the project's chosen JS package manager — pnpm-lock.yaml
# in the tree is historical and may diverge from bun.lockb; that's a
# separate cleanup item, not blocking the release.
Write-Host "[5/8] bun install + bun run build (extension)" -ForegroundColor DarkGray
$bunCmd = Get-Command bun -ErrorAction SilentlyContinue
if (-not $bunCmd) { throw "bun not on PATH — install from https://bun.sh" }
Push-Location extension
try {
    bun install
    if ($LASTEXITCODE -ne 0) { throw "bun install failed (extension)" }
    bun run typecheck
    if ($LASTEXITCODE -ne 0) { throw "bun run typecheck failed (extension)" }
    bun run build
    if ($LASTEXITCODE -ne 0) { throw "bun run build failed (extension)" }
}
finally {
    Pop-Location
}

# 5. tauri build
Write-Host "[6/8] cargo tauri build (NSIS + MSI)" -ForegroundColor DarkGray

# Stamp the Web Store extension ID into the host manifest template if
# one was supplied; otherwise the committed dev ID ships.
# The original file is restored in a `finally` block so the working
# tree stays clean even if the build fails.
$hostManifestPath = "src-tauri\native-host\com.unduhin.host.json"
$hostManifestBackup = $null
if ($WebStoreExtensionId) {
    if ($WebStoreExtensionId -notmatch '^[a-p]{32}$') {
        throw "Web Store extension IDs are 32 chars from a-p. Got: '$WebStoreExtensionId'."
    }
    if (-not (Test-Path $hostManifestPath)) {
        throw "Host manifest not found at $hostManifestPath. Did the build stage the template?"
    }
    Write-Host "      Stamping Web Store extension ID into host manifest" -ForegroundColor DarkGray
    $hostManifestBackup = Get-Content -Raw -Path $hostManifestPath
    # Replace any prior chrome-extension://<id>/ entries with the Web Store ID.
    $rewritten = [regex]::Replace(
        $hostManifestBackup,
        'chrome-extension://[a-p]{32}/',
        "chrome-extension://$WebStoreExtensionId/"
    )
    Set-Content -Path $hostManifestPath -Value $rewritten -Encoding utf8 -NoNewline
}

try {
    cargo tauri build --bundles nsis,msi
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
}
finally {
    if ($null -ne $hostManifestBackup) {
        Set-Content -Path $hostManifestPath -Value $hostManifestBackup -Encoding utf8 -NoNewline
        Write-Host "      Restored host manifest template" -ForegroundColor DarkGray
    }
}

# 6. Sign artefacts
Write-Host "[7/8] sign artefacts" -ForegroundColor DarkGray
& "$PSScriptRoot\sign-artifacts.ps1" -Thumbprint $Thumbprint

# 6.5. Zip the extension build as a release asset. Filename mirrors the
# Tauri NSIS/MSI artefacts so users can spot it on the Releases page.
$extZipDir = "target\release\bundle\extension"
New-Item -ItemType Directory -Force $extZipDir | Out-Null
$extZipPath = "$extZipDir\Unduhin_${Version}_extension.zip"
if (Test-Path $extZipPath) { Remove-Item $extZipPath -Force }
Compress-Archive -Path "extension\dist\*" -DestinationPath $extZipPath -Force
Write-Host "      extension zipped → $extZipPath" -ForegroundColor DarkGray

# 7. Updater manifest
Write-Host "[8/8] updater manifest" -ForegroundColor DarkGray
$bundleRoot = "target\release\bundle\nsis"
# Tauri v2 with `createUpdaterArtifacts: true` (the modern, non-v1Compatible
# format) signs the NSIS installer directly: the updater artifact is the
# `-setup.exe` itself and its signature is `<setup>.exe.sig`. (The legacy
# `.nsis.zip` is only emitted under `createUpdaterArtifacts: "v1Compatible"`.)
# Filter by the version being released so stale `-setup.exe` files from
# prior builds in the same target dir don't win Select-Object -First 1 and
# point the manifest at the wrong asset.
$archive = Get-ChildItem -Path $bundleRoot -Filter "*_${Version}_*-setup.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
$sig     = if ($archive) {
    Get-ChildItem -Path $bundleRoot -Filter "$($archive.Name).sig" -ErrorAction SilentlyContinue | Select-Object -First 1
} else { $null }

if (-not $archive -or -not $sig) {
    Write-Warning "No -setup.exe + .sig pair found under $bundleRoot — manifest step skipped."
    Write-Warning "Is createUpdaterArtifacts enabled and a signing key (TAURI_SIGNING_PRIVATE_KEY) configured?"
} else {
    & "$PSScriptRoot\build-update-manifest.ps1" `
        -Version $Version `
        -Channel $Channel `
        -ArtifactPath $archive.FullName `
        -SigPath $sig.FullName `
        -ReleaseUrlBase $ReleaseUrlBase `
        -Notes $Notes
}

Write-Host ""
Write-Host "Build complete." -ForegroundColor Green
$artefacts = @(Get-ChildItem -Path "target\release\bundle" -Recurse -Include "*.exe","*.msi","*.nsis.zip","*.sig","latest-*.json")
foreach ($a in $artefacts) { Write-Host ("  " + $a.FullName) }

if (-not $Publish) {
    Write-Host ""
    Write-Host "Dry run — nothing published. Re-run with -Publish to ship." -ForegroundColor Yellow
    return
}

# --- 8. Publish ------------------------------------------------------------
Write-Host ""
Write-Host "[8/8] publishing v$Version" -ForegroundColor DarkGray

# Stage only the files bump-version.ps1 touches, so we don't accidentally
# sweep in unrelated edits even though the pre-flight required a clean tree.
$bumpedPaths = @(
    "Cargo.toml",
    "Cargo.lock",
    "src-tauri/tauri.conf.json",
    "frontend/package.json",
    "frontend/package-lock.json",
    "extension/package.json",
    "extension/manifest.json"
) | Where-Object { Test-Path $_ }

git add -- $bumpedPaths
if ($LASTEXITCODE -ne 0) { throw "git add failed" }

# `bump-version.ps1` is idempotent — re-running it on a tree that
# already carries the target version yields no diff. In that case
# `git commit` would fail with "nothing to commit". Detect that and
# skip straight to tagging existing HEAD.
$staged = git diff --cached --name-only
if (-not $staged) {
    Write-Host "  no version-bump diff to commit — tagging existing HEAD" -ForegroundColor DarkGray
} else {
    git commit -m "chore: release v$Version"
    if ($LASTEXITCODE -ne 0) { throw "git commit failed" }
}

# Tag may already exist if a prior -Publish made it this far before
# failing further down — tolerate that.
$existing = git tag --list "v$Version"
if ($existing) {
    Write-Host "  tag v$Version already exists — reusing it" -ForegroundColor DarkGray
} else {
    git tag "v$Version"
    if ($LASTEXITCODE -ne 0) { throw "git tag failed" }
}

git push $Remote HEAD
if ($LASTEXITCODE -ne 0) { throw "git push (branch) failed" }

git push $Remote "v$Version"
if ($LASTEXITCODE -ne 0) { throw "git push (tag) failed" }

# Collect assets in the same shape the old CI workflow uploaded.
$assets = @()
$nsisDir = "target\release\bundle\nsis"
$msiDir  = "target\release\bundle\msi"
$bundleRoot = "target\release\bundle"

if (Test-Path $nsisDir) {
    # Filter by the version being released so previous builds left in
    # target\release\bundle\ don't get uploaded alongside the current one.
    # The signed `-setup.exe` IS the updater artefact; its `.exe.sig` is
    # uploaded too so users can verify the download out of band (the
    # updater itself reads the signature embedded in latest-<channel>.json).
    $assets += Get-ChildItem -Path $nsisDir -Filter "*_${Version}_*-setup.exe"     -File -ErrorAction SilentlyContinue
    $assets += Get-ChildItem -Path $nsisDir -Filter "*_${Version}_*-setup.exe.sig" -File -ErrorAction SilentlyContinue
}
if (Test-Path $msiDir) {
    $assets += Get-ChildItem -Path $msiDir -Filter "*_${Version}_*.msi" -File -ErrorAction SilentlyContinue
}
# Extension zip (Chromium browsers load this unpacked until the Web
# Store listing is live).
if (Test-Path $extZipDir) {
    $assets += Get-ChildItem -Path $extZipDir -Filter "*_extension.zip" -File -ErrorAction SilentlyContinue
}
$assets += Get-ChildItem -Path $bundleRoot -Filter "latest-*.json" -File -ErrorAction SilentlyContinue

if (-not $assets) { throw "No release assets found under $bundleRoot." }

$assetPaths = $assets | ForEach-Object { $_.FullName }

Write-Host "  uploading $($assetPaths.Count) asset(s) to GitHub Release v$Version"
& gh release create "v$Version" `
    --title "Unduhin v$Version" `
    --generate-notes `
    $assetPaths
if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }

Write-Host ""
Write-Host "Released v$Version." -ForegroundColor Green
