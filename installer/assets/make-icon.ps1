# make-icon.ps1
# Genera installer\assets\speedy.ico usando System.Drawing (.NET Framework).
# Output: speedy.ico nella stessa cartella di questo script.
#
# Requisiti: PowerShell 5.1+ con .NET Framework 4.x (incluso in Windows 10+).
# Eseguire una volta prima di buildare l'installer, oppure lasciare che
# build-installer.ps1 lo chiami automaticamente se speedy.ico non esiste.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$outPath = Join-Path $PSScriptRoot 'speedy.ico'
$sizes   = @(16, 32, 48, 256)
$bgColor = [System.Drawing.Color]::FromArgb(255, 22, 163, 74)   # verde (Tailwind green-600)
$fgColor = [System.Drawing.Color]::White

function New-SpeedyBitmap([int]$sz) {
    $bmp = New-Object System.Drawing.Bitmap($sz, $sz, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g   = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode  = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.Clear([System.Drawing.Color]::Transparent)

    # Cerchio di sfondo verde
    $margin = [Math]::Max(1, [int]($sz * 0.04))
    $brush  = New-Object System.Drawing.SolidBrush($bgColor)
    $g.FillEllipse($brush, $margin, $margin, $sz - 2*$margin - 1, $sz - 2*$margin - 1)
    $brush.Dispose()

    # Lettera "S" bianca al centro (solo per dimensioni >= 24 px)
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

# Converti ogni bitmap in blocco PNG (ICO moderno usa PNG anche per i frame piccoli;
# su Windows 10+ tutti i formati PNG-in-ICO sono supportati)
$pngBlocks = foreach ($sz in $sizes) {
    $bmp = New-SpeedyBitmap $sz
    $ms  = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    ,$ms.ToArray()    # virgola = wrappa in array (evita unrolling nella pipeline)
    $ms.Dispose()
}

# ---- Costruzione manuale del file ICO ----
# Formato ICO: [Header 6 byte] + [Directory count*16 byte] + [Dati immagine]
$count      = $sizes.Count
$dataOffset = 6 + $count * 16     # offset del primo blocco immagine

$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($ms)

# Header
$bw.Write([uint16]0)       # reserved (deve essere 0)
$bw.Write([uint16]1)       # image type: 1 = ICO
$bw.Write([uint16]$count)  # numero di immagini

# Directory entries
for ($i = 0; $i -lt $count; $i++) {
    $sz = $sizes[$i]
    $bw.Write([byte]$(if ($sz -ge 256) { 0 } else { $sz }))   # width  (0 = 256)
    $bw.Write([byte]$(if ($sz -ge 256) { 0 } else { $sz }))   # height
    $bw.Write([byte]0)       # num colors in palette (0 = nessuna palette)
    $bw.Write([byte]0)       # reserved
    $bw.Write([uint16]1)     # color planes
    $bw.Write([uint16]32)    # bit depth
    $bw.Write([uint32]$pngBlocks[$i].Length)  # dimensione dati immagine
    $bw.Write([uint32]$dataOffset)            # offset da inizio file
    $dataOffset += $pngBlocks[$i].Length
}

# Dati immagine
foreach ($block in $pngBlocks) {
    $bw.Write($block)
}

$bw.Flush()
[System.IO.File]::WriteAllBytes($outPath, $ms.ToArray())
$bw.Dispose()
$ms.Dispose()

Write-Host "Icona generata: $outPath ($([math]::Round((Get-Item $outPath).Length / 1KB, 1)) KB)" -ForegroundColor Green
