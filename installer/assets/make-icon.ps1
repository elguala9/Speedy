# make-icon.ps1
# Generates installer\assets\speedy.ico using System.Drawing (.NET Framework).
# Output: speedy.ico in the same folder as this script.
#
# Requirements: PowerShell 5.1+ with .NET Framework 4.x (included in Windows 10+).
# Run once before building the installer, or let
# build-installer.ps1 call it automatically if speedy.ico does not exist.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$outPath = Join-Path $PSScriptRoot 'speedy.ico'
$sizes   = @(16, 32, 48, 256)
$bgColor = [System.Drawing.Color]::FromArgb(255, 22, 163, 74)   # green (Tailwind green-600)
$fgColor = [System.Drawing.Color]::White

function New-SpeedyBitmap([int]$sz) {
    $bmp = New-Object System.Drawing.Bitmap($sz, $sz, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g   = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode  = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.Clear([System.Drawing.Color]::Transparent)

    # Green background circle
    $margin = [Math]::Max(1, [int]($sz * 0.04))
    $brush  = New-Object System.Drawing.SolidBrush($bgColor)
    $g.FillEllipse($brush, $margin, $margin, $sz - 2*$margin - 1, $sz - 2*$margin - 1)
    $brush.Dispose()

    # White "S" letter in the center (only for sizes >= 24 px)
    if ($sz -ge 24) {
        $fontSize = [float]($sz * 0.48)
        $font = New-Object System.Drawing.Font(
            'Segoe UI', $fontSize,
            [System.Drawing.FontStyle]::Bold,
            [System.Drawing.GraphicsUnit]::Pixel
        )
        $tb = New-Object System.Drawing.SolidBrush($fgColor)
        $sf = New-Object System.Drawing.StringFormat
        $sf.Alignment     = [System.Drawing.StringAlignment]::Center
        $sf.LineAlignment = [System.Drawing.StringAlignment]::Center
        $rect = [System.Drawing.RectangleF]::new(0, 0, $sz, $sz)
        $g.DrawString('S', $font, $tb, $rect, $sf)
        $font.Dispose()
        $tb.Dispose()
        $sf.Dispose()
    }

    $g.Dispose()
    return $bmp
}

# Convert each bitmap into a PNG block (modern ICO uses PNG even for small frames;
# on Windows 10+ all PNG-in-ICO formats are supported)
$pngBlocks = foreach ($sz in $sizes) {
    $bmp = New-SpeedyBitmap $sz
    $ms  = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    ,$ms.ToArray()    # comma = wrap in array (avoids unrolling in the pipeline)
    $ms.Dispose()
}

# ---- Manual construction of the ICO file ----
# ICO format: [Header 6 bytes] + [Directory count*16 bytes] + [Image data]
$count      = $sizes.Count
$dataOffset = 6 + $count * 16     # offset of the first image block

$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($ms)

# Header
$bw.Write([uint16]0)       # reserved (must be 0)
$bw.Write([uint16]1)       # image type: 1 = ICO
$bw.Write([uint16]$count)  # number of images

# Directory entries
for ($i = 0; $i -lt $count; $i++) {
    $sz = $sizes[$i]
    $bw.Write([byte]$(if ($sz -ge 256) { 0 } else { $sz }))   # width  (0 = 256)
    $bw.Write([byte]$(if ($sz -ge 256) { 0 } else { $sz }))   # height
    $bw.Write([byte]0)       # num colors in palette (0 = no palette)
    $bw.Write([byte]0)       # reserved
    $bw.Write([uint16]1)     # color planes
    $bw.Write([uint16]32)    # bit depth
    $bw.Write([uint32]$pngBlocks[$i].Length)  # image data size
    $bw.Write([uint32]$dataOffset)            # offset from start of file
    $dataOffset += $pngBlocks[$i].Length
}

# Image data
foreach ($block in $pngBlocks) {
    $bw.Write($block)
}

$bw.Flush()
[System.IO.File]::WriteAllBytes($outPath, $ms.ToArray())
$bw.Dispose()
$ms.Dispose()

Write-Host "Icon generated: $outPath ($([math]::Round((Get-Item $outPath).Length / 1KB, 1)) KB)" -ForegroundColor Green
