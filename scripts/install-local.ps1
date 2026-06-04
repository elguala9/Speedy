<#
.SYNOPSIS
    Silently installs the freshly built Speedy installer into %LOCALAPPDATA%\Programs\Speedy.

.DESCRIPTION
    1. Kills all running Speedy processes (to avoid a pending restartreplace)
    2. Reads the version from Cargo.toml
    3. Runs dist\speedy-setup-<version>.exe in /VERYSILENT
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
$cargoToml = Join-Path $root 'Cargo.toml'

$match = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) { throw "'version' field not found in $cargoToml" }
$version = $match.Matches[0].Groups[1].Value

$setup = Join-Path $root "dist\speedy-setup-$version.exe"
if (-not (Test-Path $setup)) { throw "Installer not found: $setup. Run 'just dist' first." }

Write-Host "==> Stopping running Speedy processes..." -ForegroundColor Yellow
$names = @('speedy-daemon','speedy-gui','speedy-cli','speedy-ai-context','speedy-ai-context-mcp','speedy-language-context','speedy-language-context-mcp')
Get-Process -Name $names -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "    kill $($_.Name) (PID $($_.Id))" -ForegroundColor DarkGray
    try { Stop-Process -Id $_.Id -Force -ErrorAction Stop } catch { Write-Host "    warning: $_" -ForegroundColor DarkYellow }
}

# Small pause to let Windows release the file locks
Start-Sleep -Milliseconds 500

Write-Host ""
Write-Host "==> Silent installation: $setup" -ForegroundColor Cyan
$proc = Start-Process -FilePath $setup -ArgumentList '/VERYSILENT','/NORESTART','/SUPPRESSMSGBOXES','/CLOSEAPPLICATIONS' -Wait -PassThru
if ($proc.ExitCode -ne 0) { throw "Installer exit code $($proc.ExitCode)" }

$installed = Join-Path $env:LOCALAPPDATA 'Programs\Speedy\speedy-gui.exe'
Write-Host ""
Write-Host "==> Installation complete." -ForegroundColor Green
Write-Host "    Binaries in: $env:LOCALAPPDATA\Programs\Speedy" -ForegroundColor Green
Write-Host "    Launch: $installed" -ForegroundColor Green
