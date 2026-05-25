<#
.SYNOPSIS
    Installa silenziosamente l'installer Speedy appena buildato in %LOCALAPPDATA%\Programs\Speedy.

.DESCRIPTION
    1. Ammazza tutti i processi Speedy in esecuzione (per evitare restartreplace pendente)
    2. Legge la versione da Cargo.toml
    3. Esegue dist\speedy-setup-<version>.exe in /VERYSILENT
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
$cargoToml = Join-Path $root 'Cargo.toml'

$match = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) { throw "Campo 'version' non trovato in $cargoToml" }
$version = $match.Matches[0].Groups[1].Value

$setup = Join-Path $root "dist\speedy-setup-$version.exe"
if (-not (Test-Path $setup)) { throw "Installer non trovato: $setup. Esegui prima 'just dist'." }

Write-Host "==> Stop processi Speedy in esecuzione..." -ForegroundColor Yellow
$names = @('speedy-daemon','speedy-gui','speedy-cli','speedy-ai-context','speedy-ai-context-mcp','speedy-language-context','speedy-language-context-mcp')
Get-Process -Name $names -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "    kill $($_.Name) (PID $($_.Id))" -ForegroundColor DarkGray
    try { Stop-Process -Id $_.Id -Force -ErrorAction Stop } catch { Write-Host "    warning: $_" -ForegroundColor DarkYellow }
}

# Piccola pausa per permettere a Windows di liberare i file lock
Start-Sleep -Milliseconds 500

Write-Host ""
Write-Host "==> Installazione silenziosa: $setup" -ForegroundColor Cyan
$proc = Start-Process -FilePath $setup -ArgumentList '/VERYSILENT','/NORESTART','/SUPPRESSMSGBOXES','/CLOSEAPPLICATIONS' -Wait -PassThru
if ($proc.ExitCode -ne 0) { throw "Installer exit code $($proc.ExitCode)" }

$installed = Join-Path $env:LOCALAPPDATA 'Programs\Speedy\speedy-gui.exe'
Write-Host ""
Write-Host "==> Installazione completata." -ForegroundColor Green
Write-Host "    Binari in: $env:LOCALAPPDATA\Programs\Speedy" -ForegroundColor Green
Write-Host "    Avvia: $installed" -ForegroundColor Green
