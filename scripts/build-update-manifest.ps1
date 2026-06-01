<#
.SYNOPSIS
    Builds the Tauri updater manifest (latest-<channel>.json) from a
    bundled NSIS update artefact + its signature.

.DESCRIPTION
    Tauri v2 expects the manifest to look like:
        {
          "version": "0.2.0",
          "notes":   "release notes (markdown)",
          "pub_date": "2026-05-25T10:30:00Z",
          "platforms": {
            "windows-x86_64": {
              "signature": "<minisign-style base64 sig>",
              "url":       "https://.../Unduhin_0.2.0_x64-setup.nsis.zip"
            }
          }
        }

    The update artefact is the ".nsis.zip" produced by the Tauri bundler
    when the updater plugin is enabled. The corresponding signature file
    is "<archive>.sig". Both are uploaded as release assets.

.PARAMETER Version
    Bare semver, no "v" prefix.

.PARAMETER Channel
    "stable" or "beta". Drives the output filename.

.PARAMETER ArtifactPath
    Path to the .nsis.zip update artefact produced by `tauri build`.

.PARAMETER SigPath
    Path to the corresponding .sig file. Read into the manifest verbatim.

.PARAMETER ReleaseUrlBase
    Base URL where assets will be hosted (e.g.
    "https://github.com/dannyatthaya/unduhin/releases/download/v0.2.0").

.PARAMETER Notes
    Release notes (markdown). Embedded as a single string.

.PARAMETER OutFile
    Path to write the manifest. Defaults to
    "target/release/bundle/latest-<channel>.json".
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)] [string]$Version,
    [Parameter(Mandatory=$true)] [ValidateSet("stable", "beta")] [string]$Channel,
    [Parameter(Mandatory=$true)] [string]$ArtifactPath,
    [Parameter(Mandatory=$true)] [string]$SigPath,
    [Parameter(Mandatory=$true)] [string]$ReleaseUrlBase,
    [string]$Notes = "",
    [string]$OutFile = ""
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $ArtifactPath)) { throw "Artifact not found: $ArtifactPath" }
if (-not (Test-Path $SigPath))      { throw "Signature not found: $SigPath" }

$archiveName = Split-Path $ArtifactPath -Leaf
$sig         = (Get-Content $SigPath -Raw).Trim()
$pubDate     = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$assetUrl    = "$($ReleaseUrlBase.TrimEnd('/'))/$archiveName"

$manifest = [ordered]@{
    version   = $Version
    notes     = $Notes
    pub_date  = $pubDate
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $sig
            url       = $assetUrl
        }
    }
}

if (-not $OutFile) {
    $OutFile = "target\release\bundle\latest-$Channel.json"
}

$dir = Split-Path $OutFile -Parent
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }

$json = ($manifest | ConvertTo-Json -Depth 6) + "`n"
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
$abs = if ([System.IO.Path]::IsPathRooted($OutFile)) { $OutFile } else { Join-Path (Get-Location) $OutFile }
[System.IO.File]::WriteAllText($abs, $json, $utf8NoBom)

Write-Host ("Wrote {0} (artifact={1}, channel={2})" -f $OutFile, $archiveName, $Channel) -ForegroundColor Green
