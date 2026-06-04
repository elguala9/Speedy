$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
$dist = Join-Path $root 'dist'
$target = Join-Path (Join-Path $root 'target') 'release'

# Cleanup
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force -Path $dist | Out-Null

Write-Host '==> Building release of the 9 binaries...' -ForegroundColor Yellow
cargo build --release -p speedy-ai-context -p speedy-daemon -p speedy-cli -p speedy-ai-context-mcp -p speedy-gui -p speedy-language-context -p speedy-text
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }

# Copy to dist/
@('speedy-ai-context.exe', 'speedy-daemon.exe', 'speedy-cli.exe', 'speedy-ai-context-mcp.exe', 'speedy-gui.exe', 'speedy-language-context.exe', 'speedy-language-context-mcp.exe', 'speedy-text-context.exe', 'speedy-text-context-mcp.exe') | ForEach-Object {
    $src = Join-Path $target $_
    if (Test-Path $src) {
        Copy-Item $src $dist
        Write-Host "  Copied $_" -ForegroundColor Green
    } else {
        Write-Host "  NOT FOUND: $_" -ForegroundColor Red
    }
}

Write-Host "`nBinaries ready in: $dist" -ForegroundColor Green
Get-ChildItem $dist | ForEach-Object { Write-Host "  $($_.Name) ($([math]::Round($_.Length/1KB)) KB)" }
