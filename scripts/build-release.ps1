$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
$dist = Join-Path $root 'dist'
$target = Join-Path (Join-Path $root 'target') 'release'

# Pulizia
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force -Path $dist | Out-Null

Write-Host '==> Build release dei 4 binari...' -ForegroundColor Yellow
cargo build --release -p speedy -p speedy-daemon -p speedy-cli -p speedy-mcp
if ($LASTEXITCODE -ne 0) { throw 'Build fallito' }

# Copia in dist/
@('speedy.exe', 'speedy-daemon.exe', 'speedy-cli.exe', 'speedy-mcp.exe') | ForEach-Object {
    $src = Join-Path $target $_
    if (Test-Path $src) {
        Copy-Item $src $dist
        Write-Host "  Copiato $_" -ForegroundColor Green
    } else {
        Write-Host "  NON TROVATO: $_" -ForegroundColor Red
    }
}

Write-Host "`nBinari pronti in: $dist" -ForegroundColor Green
Get-ChildItem $dist | ForEach-Object { Write-Host "  $($_.Name) ($([math]::Round($_.Length/1KB)) KB)" }
