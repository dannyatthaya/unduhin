<#
.SYNOPSIS
    Bumps the project version across all three sources of truth.

.DESCRIPTION
    Updates:
      - Cargo workspace (engine, unduhin-core, cli, unduhin-app) via
        cargo-edit's `cargo set-version --workspace`.
      - src-tauri/tauri.conf.json, frontend/package.json, and
        extension/{package,manifest}.json via a surgical, formatting-
        preserving edit of just the "version" field (Set-JsonVersion).
        No package manager is invoked. Manifest.json is the one browsers
        actually read, so it must be in lockstep.

    Cargo.lock is refreshed with `cargo check` so the new workspace
    versions are pinned. No git operations are performed.

.PARAMETER Version
    The new semver string, e.g. 0.2.0 or 1.0.0-rc.1.

.EXAMPLE
    .\scripts\bump-version.ps1 0.2.0

.NOTES
    Prerequisite: cargo-edit installed (cargo install cargo-edit).
    Run from the repo root.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory=$true, Position=0)]
    [string]$Version
)

$ErrorActionPreference = "Stop"

# Sanity-check the semver shape. We don't enforce the full spec -- just
# enough to catch obvious typos like "v0.2" or "0,2,0".
if ($Version -notmatch '^\d+\.\d+\.\d+(-[\w\.-]+)?(\+[\w\.-]+)?$') {
    Write-Error "Version '$Version' does not look like semver (e.g. 0.2.0 or 1.0.0-rc.1)."
    exit 1
}

# Verify we're at the repo root by looking for known anchors.
$repoRoot = Get-Location
if (-not (Test-Path "$repoRoot\Cargo.toml") -or -not (Test-Path "$repoRoot\src-tauri\tauri.conf.json")) {
    Write-Error "Run this script from the repo root (the directory containing Cargo.toml and src-tauri/)."
    exit 1
}

# Verify cargo-edit is installed.
$cargoSetVersion = & cargo set-version --help 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Error "cargo-edit is not installed. Install it with: cargo install cargo-edit"
    exit 1
}

# Surgically bump the top-level "version" string in a JSON file, preserving
# the file's existing formatting. We deliberately avoid
# `ConvertFrom-Json | ConvertTo-Json`: Windows PowerShell 5.1's
# ConvertTo-Json reformats the whole document with ugly ":  " spacing and
# inconsistent indentation, producing huge, noisy diffs on every release.
function Set-JsonVersion {
    param([string]$Path, [string]$NewVersion)
    $abs = (Resolve-Path -LiteralPath $Path).Path
    $raw = [System.IO.File]::ReadAllText($abs)
    $rx  = [regex]'("version"\s*:\s*")[^"]*(")'
    if (-not $rx.IsMatch($raw)) {
        throw "No top-level `"version`" field found in $Path"
    }
    # Replace only the first match (the top-level version) so any nested
    # "version" key added by future schema growth is left untouched.
    $updated = $rx.Replace($raw, "`${1}$NewVersion`${2}", 1)
    # UTF-8 *without* a BOM -- Tauri's serde_json rejects a BOM with
    # "expected value at line 1 column 1" when it reads the config.
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($abs, $updated, $utf8NoBom)
}

Write-Host "Bumping to v$Version" -ForegroundColor Cyan
Write-Host ""

# --- 1. Cargo workspace ----------------------------------------------------
Write-Host "  [1/4] cargo set-version --workspace $Version" -ForegroundColor DarkGray
cargo set-version --workspace $Version
if ($LASTEXITCODE -ne 0) {
    Write-Error "cargo set-version failed"
    exit 1
}

# Refresh Cargo.lock so the new workspace versions are pinned. We use
# `cargo check` rather than `cargo build` to keep this fast -- the lock
# file is what we care about, not artefacts.
Write-Host "  [.../] cargo check (refreshing Cargo.lock)" -ForegroundColor DarkGray
cargo check --workspace --quiet
if ($LASTEXITCODE -ne 0) {
    Write-Error "cargo check failed after version bump -- Cargo.lock may be stale"
    exit 1
}

# --- 2. Tauri config -------------------------------------------------------
Write-Host "  [2/4] src-tauri/tauri.conf.json" -ForegroundColor DarkGray
Set-JsonVersion "src-tauri\tauri.conf.json" $Version

# --- 3. Frontend package.json ----------------------------------------------
# Bun has no `npm version` equivalent (no `bun pm version`), so bump the
# version field directly with the same surgical, formatting-preserving
# helper used for the other JSON files. The real lockfile (frontend/bun.lock)
# carries only dependency versions, not the project version, so it needs no
# update.
Write-Host "  [3/4] frontend/package.json" -ForegroundColor DarkGray
Set-JsonVersion "frontend\package.json" $Version

# --- 4. Extension package.json + manifest.json ----------------------------
# Both need bumping: package.json tracks the source-repo version,
# manifest.json is what browsers display. Surgical edit (see
# Set-JsonVersion) keeps their formatting intact.
Write-Host "  [4/4] extension/{package,manifest}.json" -ForegroundColor DarkGray
Set-JsonVersion "extension\package.json" $Version
Set-JsonVersion "extension\manifest.json" $Version

Write-Host ""
Write-Host "Bumped to v$Version. Files changed:" -ForegroundColor Green
Write-Host "  - Cargo.toml (workspace)"
Write-Host "  - Cargo.lock"
Write-Host "  - src-tauri/tauri.conf.json"
Write-Host "  - frontend/package.json"
Write-Host "  - extension/package.json"
Write-Host "  - extension/manifest.json"
Write-Host ""
Write-Host "Suggested next steps:" -ForegroundColor Cyan
Write-Host "  git add -A"
Write-Host "  git commit -m 'chore: release v$Version'"
Write-Host "  git tag v$Version"
Write-Host "  git push --follow-tags"
