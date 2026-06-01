# Generate tinted tray icons from src-tauri/icons/icon.png.
#
# Output: src-tauri/icons/tray/{idle,downloading,paused,error}.png at 32x32.
# Each variant tints the source by multiplying its RGB by a colour and
# keeping the original alpha. The tray is small (32x32 on standard DPI,
# 64x64 on 200%) so a soft tint reads better than a heavy overlay.
#
# Re-run after changing icon.png; commit the generated PNGs.

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $root
$src = Join-Path $repo "src-tauri/icons/icon.png"
$out = Join-Path $repo "src-tauri/icons/tray"

if (-not (Test-Path $out)) {
    New-Item -ItemType Directory -Path $out | Out-Null
}

$variants = @{
    "idle"        = @{ R = 0.78; G = 0.82; B = 0.88 }  # neutral, slightly cool
    "downloading" = @{ R = 0.30; G = 0.60; B = 1.00 }  # accent blue
    "paused"      = @{ R = 1.00; G = 0.72; B = 0.18 }  # warning amber
    "error"       = @{ R = 0.95; G = 0.32; B = 0.28 }  # danger red
}

$sourceBitmap = [System.Drawing.Bitmap]::FromFile($src)
try {
    foreach ($name in $variants.Keys) {
        $tint = $variants[$name]
        # Render at 64x64 so the icon is crisp on 200% DPI; Windows picks
        # the right size from the PNG itself.
        $size = 64
        $bmp = New-Object System.Drawing.Bitmap $size, $size
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $matrix = New-Object System.Drawing.Imaging.ColorMatrix
            $matrix.Matrix00 = [float]$tint.R
            $matrix.Matrix11 = [float]$tint.G
            $matrix.Matrix22 = [float]$tint.B
            $matrix.Matrix33 = 1.0
            $matrix.Matrix44 = 1.0
            $attrs = New-Object System.Drawing.Imaging.ImageAttributes
            $attrs.SetColorMatrix($matrix)
            $rect = New-Object System.Drawing.Rectangle 0, 0, $size, $size
            $g.DrawImage(
                $sourceBitmap,
                $rect,
                0, 0, $sourceBitmap.Width, $sourceBitmap.Height,
                [System.Drawing.GraphicsUnit]::Pixel,
                $attrs
            )
        }
        finally {
            $g.Dispose()
        }
        $target = Join-Path $out "$name.png"
        $bmp.Save($target, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        Write-Host "Wrote $target"
    }
}
finally {
    $sourceBitmap.Dispose()
}
