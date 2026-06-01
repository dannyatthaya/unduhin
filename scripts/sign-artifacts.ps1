<#
.SYNOPSIS
    Code-signs the NSIS .exe / MSI .msi installer artefacts in-place.

.DESCRIPTION
    Stub for the EV code-signing step. The actual `signtool` invocation
    is intentionally commented out — when a real certificate is wired up
    (cert in the runner's certstore, or signed via Azure Trusted Signing),
    uncomment the marked block and supply -Thumbprint.

    The script is idempotent: re-signing an already-signed binary is a
    no-op when the thumbprint matches.

    By design, this script produces identical artefacts whether signing
    is configured or not — when no thumbprint is given, it logs and exits
    cleanly. The artefacts themselves are not rewritten in that case.

.PARAMETER Thumbprint
    The SHA-1 thumbprint of the signing certificate in the runner's
    certificate store. Required for real signing; if absent or empty,
    the script no-ops.

.PARAMETER BundleDir
    Folder containing the NSIS .exe and MSI .msi to sign. Defaults to
    target/release/bundle/.
#>

[CmdletBinding()]
param(
    [string]$Thumbprint = "",
    [string]$BundleDir  = "target\release\bundle"
)

$ErrorActionPreference = "Stop"

if (-not $Thumbprint) {
    Write-Host "No SIGN_CERT_THUMBPRINT configured — leaving artefacts unsigned." -ForegroundColor Yellow
    Write-Host "Plug in by setting `$env:SIGN_CERT_THUMBPRINT` and re-running."
    exit 0
}

if (-not (Test-Path $BundleDir)) {
    throw "Bundle directory not found: $BundleDir. Run `cargo tauri build` first."
}

# Locate signtool. Windows 10/11 SDKs install it under Program Files (x86).
$signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
if (-not $signtool) {
    $sdk = "C:\Program Files (x86)\Windows Kits\10\bin"
    $signtool = Get-ChildItem -Path $sdk -Recurse -Filter "signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $signtool) {
    throw "signtool.exe not on PATH and not found under Windows Kits — install the Windows 10 SDK."
}

$artefacts = @()
$artefacts += Get-ChildItem -Path $BundleDir -Recurse -Filter "*.exe" -ErrorAction SilentlyContinue
$artefacts += Get-ChildItem -Path $BundleDir -Recurse -Filter "*.msi" -ErrorAction SilentlyContinue

if ($artefacts.Count -eq 0) {
    Write-Warning "No .exe or .msi found under $BundleDir — nothing to sign."
    exit 0
}

foreach ($file in $artefacts) {
    Write-Host ("Signing {0}" -f $file.FullName) -ForegroundColor DarkGray
    # Real signing — uncomment when EV cert is plumbed:
    # & $signtool sign /sha1 $Thumbprint `
    #     /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
    #     "$($file.FullName)"
    # if ($LASTEXITCODE -ne 0) { throw "signtool failed for $($file.Name)" }
    Write-Host "  (stub: signtool block is commented out; no bytes changed)" -ForegroundColor Yellow
}

Write-Host "Done." -ForegroundColor Green
