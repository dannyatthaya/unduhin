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
              "url":       "https://.../Unduhin_0.2.0_x64-setup.exe"
            }
          }
        }

    Under Tauri v2's modern updater format (`createUpdaterArtifacts: true`)
    the NSIS installer is signed directly, so the update artefact is the
    "-setup.exe" itself and its signature is "<setup>.exe.sig". (The legacy
    ".nsis.zip" is only produced under `createUpdaterArtifacts:
    "v1Compatible"`.) The signature is embedded into the manifest below; the
    installer is uploaded as a release asset and referenced by `url`.

.PARAMETER Version
    Bare semver, no "v" prefix.

.PARAMETER Channel
    "stable" or "beta". Drives the output filename.

.PARAMETER ArtifactPath
    Path to the signed "-setup.exe" update artefact produced by `tauri build`.

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

if (-not $OutFile) {
    $OutFile = "target\release\bundle\latest-$Channel.json"
}

$dir = Split-Path $OutFile -Parent
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }

# Emit the manifest by hand instead of via ConvertTo-Json. Windows
# PowerShell 5.1's serializer pretty-prints with 4-space indents, two
# spaces after every colon, and closing braces aligned under the key —
# valid JSON, but a non-standard layout that doesn't match what the Tauri
# docs show or what the rest of the project uses. We still lean on
# ConvertTo-Json for per-value escaping so any quotes / backslashes /
# newlines in $Notes are encoded correctly; it just escapes one scalar
# string at a time and we assemble the structure ourselves.
function ConvertTo-JsonString([string]$value) {
    # A lone string through ConvertTo-Json comes back as a fully quoted,
    # escaped JSON string literal (e.g. `"a\"b"`), on a single line.
    return ([string]$value | ConvertTo-Json -Compress)
}

$json = @"
{
  "version": $(ConvertTo-JsonString $Version),
  "notes": $(ConvertTo-JsonString $Notes),
  "pub_date": $(ConvertTo-JsonString $pubDate),
  "platforms": {
    "windows-x86_64": {
      "signature": $(ConvertTo-JsonString $sig),
      "url": $(ConvertTo-JsonString $assetUrl)
    }
  }
}
"@ + "`n"
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
$abs = if ([System.IO.Path]::IsPathRooted($OutFile)) { $OutFile } else { Join-Path (Get-Location) $OutFile }
[System.IO.File]::WriteAllText($abs, $json, $utf8NoBom)

Write-Host ("Wrote {0} (artifact={1}, channel={2})" -f $OutFile, $archiveName, $Channel) -ForegroundColor Green
