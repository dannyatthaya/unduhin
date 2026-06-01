<#
.SYNOPSIS
    Generates frontend/src/generated/licences.json from cargo metadata
    and node package manifests.

.DESCRIPTION
    The About page lists open-source dependencies with their licence.
    Rather than ship a hand-maintained list (rot-prone), this script
    crawls:
      - `cargo metadata` for direct Rust workspace dependencies.
      - frontend/package.json + frontend/node_modules/<name>/package.json
        for direct node dependencies (NOT devDependencies -- those are
        build tooling, not shipped artefacts).

    The output is one JSON file the Vue layer imports as a module.

    Idempotent. Re-run any time, before `bun run build` or by hand.

.PARAMETER OutFile
    Where to write the manifest. Defaults to
    frontend/src/generated/licences.json.

.NOTES
    Run from the repo root. Requires cargo + node on PATH.
#>

[CmdletBinding()]
param(
    [string]$OutFile = "frontend\src\generated\licences.json"
)

$ErrorActionPreference = "Stop"

function Resolve-CargoCrates {
    $metaJson = & cargo metadata --format-version 1 --quiet 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed"
    }
    $meta = $metaJson | ConvertFrom-Json

    # Collect direct deps of every workspace member. We deduplicate by
    # (name, version) so a crate shared across members isn't listed twice.
    $wsIds = $meta.workspace_members
    $directDepIds = @{}
    foreach ($pkg in $meta.packages) {
        if ($wsIds -contains $pkg.id) {
            foreach ($dep in $pkg.dependencies) {
                if ($dep.kind -eq "dev" -or $dep.kind -eq "build") { continue }
                $directDepIds[$dep.name] = $true
            }
        }
    }

    $rows = @()
    foreach ($pkg in $meta.packages) {
        if (-not $directDepIds.ContainsKey($pkg.name)) { continue }
        if ($wsIds -contains $pkg.id) { continue }  # don't list our own crates
        $rows += [pscustomobject]@{
            name       = $pkg.name
            version    = $pkg.version
            license    = if ($pkg.license) { $pkg.license } else { "Unknown" }
            repository = if ($pkg.repository) { $pkg.repository } else { $null }
        }
    }
    return $rows | Sort-Object -Property name -Unique
}

function Resolve-NodeDeps {
    $pkgJsonPath = "frontend\package.json"
    if (-not (Test-Path $pkgJsonPath)) {
        Write-Warning "frontend/package.json missing -- skipping node crawl"
        return @()
    }
    $pkg = Get-Content $pkgJsonPath -Raw | ConvertFrom-Json
    $deps = $pkg.dependencies
    if (-not $deps) { return @() }

    $rows = @()
    foreach ($name in $deps.PSObject.Properties.Name) {
        $manifestPath = "frontend\node_modules\$name\package.json"
        if (-not (Test-Path $manifestPath)) {
            Write-Warning ("  missing {0} -- run 'bun install' first" -f $manifestPath)
            continue
        }
        $m = Get-Content $manifestPath -Raw | ConvertFrom-Json
        $licence = $m.license
        if (-not $licence -and $m.licenses) {
            $licence = $m.licenses[0].type
        }
        if (-not $licence) { $licence = "Unknown" }

        $repo = $null
        if ($m.repository) {
            if ($m.repository -is [string]) {
                $repo = $m.repository
            } elseif ($m.repository.url) {
                $repo = $m.repository.url
            }
        }
        # Normalize "git+https://github.com/..." to a clickable URL.
        if ($repo -and $repo -match '^git\+(.*)$') { $repo = $matches[1] }
        if ($repo -and $repo -match '^(.*)\.git$') { $repo = $matches[1] }

        $rows += [pscustomobject]@{
            name       = $name
            version    = $m.version
            license    = $licence
            repository = $repo
        }
    }
    return $rows | Sort-Object -Property name -Unique
}

Write-Host "Crawling Rust dependencies..." -ForegroundColor DarkGray
$rustRows = Resolve-CargoCrates
Write-Host ("  Found {0} direct Rust deps" -f $rustRows.Count)

Write-Host "Crawling node dependencies..." -ForegroundColor DarkGray
$nodeRows = Resolve-NodeDeps
Write-Host ("  Found {0} direct node deps" -f $nodeRows.Count)

$payload = [ordered]@{
    generated_at = (Get-Date).ToUniversalTime().ToString("o")
    rust         = $rustRows
    node         = $nodeRows
}

$outDir = Split-Path $OutFile -Parent
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }

$utf8NoBom = New-Object System.Text.UTF8Encoding $false
$outPath = Join-Path (Resolve-Path -LiteralPath $outDir).Path (Split-Path $OutFile -Leaf)
# ConvertTo-Json is valid JSON but Windows PowerShell 5.1 formats it with
# ugly ":  " spacing and inconsistent indentation. Write it, then reformat
# in place via node (already a prerequisite of this script) to conventional
# 2-space JSON so the committed file stays clean and diffs stay small.
[System.IO.File]::WriteAllText($outPath, ($payload | ConvertTo-Json -Depth 6), $utf8NoBom)
node -e "const fs=require('fs');const f=process.argv[process.argv.length-1];fs.writeFileSync(f,JSON.stringify(JSON.parse(fs.readFileSync(f,'utf8')),null,2)+'\n')" $outPath
if ($LASTEXITCODE -ne 0) { throw "node failed to reformat $OutFile" }

Write-Host ("Wrote {0}" -f $OutFile) -ForegroundColor Green
