<#
.SYNOPSIS
    Submits the winget manifest for a new version of Speedy to microsoft/winget-pkgs.

.DESCRIPTION
    First submission:  .\scripts\submit-winget.ps1 -Version 0.2.0
    Update:            .\scripts\submit-winget.ps1 -Version 0.3.0 -Update

    The -Update flag uses 'wingetcreate update' instead of 'wingetcreate new'.
    Use -Update for all versions after the first accepted submission.

    Requirements:
      - The GitHub release v<Version> must exist with speedy-setup-<Version>.exe
      - wingetcreate is installed automatically if missing
      - A GitHub account is required to open the PR on winget-pkgs

.PARAMETER Version
    Version to publish, e.g. 0.2.0 or v0.2.0

.PARAMETER Update
    Uses 'wingetcreate update' (for updates after the first submission)

.EXAMPLE
    .\scripts\submit-winget.ps1 -Version 0.2.0
    .\scripts\submit-winget.ps1 -Version 0.3.0 -Update
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [switch]$Update
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Version      = $Version.TrimStart('v')
$vTag         = "v$Version"
$pkgId        = 'Parresia.Speedy'
$installerUrl = "https://github.com/elguala9/Speedy/releases/download/$vTag/speedy-setup-$Version.exe"

Write-Host ""
Write-Host "==> Speedy winget submit v$Version" -ForegroundColor Cyan
Write-Host "    Package ID : $pkgId" -ForegroundColor DarkGray
Write-Host "    URL        : $installerUrl" -ForegroundColor DarkGray
Write-Host "    Mode       : $( if ($Update) { 'update' } else { 'new (first submission)' } )" -ForegroundColor DarkGray
Write-Host ""

# ------------------------------------------------------------------
# 1. Install wingetcreate if missing
# ------------------------------------------------------------------
$wgc = Get-Command 'wingetcreate' -ErrorAction SilentlyContinue
if (-not $wgc) {
    Write-Host "==> wingetcreate not found, installing..." -ForegroundColor Yellow
    winget install --id Microsoft.WingetCreate --silent --accept-source-agreements --accept-package-agreements
    if ($LASTEXITCODE -ne 0) { throw "wingetcreate installation failed" }

    # Reload PATH in the current session
    $env:PATH = [System.Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [System.Environment]::GetEnvironmentVariable('Path', 'User')

    $wgc = Get-Command 'wingetcreate' -ErrorAction SilentlyContinue
    if (-not $wgc) {
        Write-Error "wingetcreate not found after installation. Restart PowerShell and try again."
        exit 1
    }
}

Write-Host "==> wingetcreate: $($wgc.Source)" -ForegroundColor DarkGray
Write-Host ""

# ------------------------------------------------------------------
# 2. Check that the GitHub release exists
# ------------------------------------------------------------------
Write-Host "==> Checking that the GitHub release exists..." -ForegroundColor Yellow
try {
    $response = Invoke-WebRequest -Uri $installerUrl -Method Head -UseBasicParsing -ErrorAction Stop
    Write-Host "    OK ($([Math]::Round($response.Headers['Content-Length'] / 1MB, 1)) MB)" -ForegroundColor DarkGray
} catch {
    Write-Error @"
Release not found: $installerUrl

Make sure you have:
  1. Run scripts\publish.ps1 -Version $Version
  2. Waited for GitHub Actions to finish the build
  3. Verified that the file exists in the release: https://github.com/elguala9/Speedy/releases/tag/$vTag
"@
    exit 1
}

Write-Host ""

# ------------------------------------------------------------------
# 3. Generate manifest and open PR
# ------------------------------------------------------------------
if ($Update) {
    Write-Host "==> wingetcreate update (version update)..." -ForegroundColor Yellow
    Write-Host "    The browser will open to authenticate with GitHub." -ForegroundColor DarkGray
    Write-Host ""
    wingetcreate update $pkgId --version $Version --urls $installerUrl --submit
} else {
    Write-Host "==> wingetcreate new (first submission)..." -ForegroundColor Yellow
    Write-Host "    The browser will open to authenticate with GitHub." -ForegroundColor DarkGray
    Write-Host "    wingetcreate will ask you to fill in some fields interactively." -ForegroundColor DarkGray
    Write-Host ""
    wingetcreate new $installerUrl
}

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "WARNING: wingetcreate returned exit code $LASTEXITCODE" -ForegroundColor Yellow
    Write-Host "The PR may not have been opened. Check the output above." -ForegroundColor Yellow
    exit $LASTEXITCODE
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host " Manifest submitted!" -ForegroundColor Green
Write-Host " Check the PR at: https://github.com/microsoft/winget-pkgs/pulls" -ForegroundColor Green
Write-Host " The winget-pkgs review typically takes 1-3 business days." -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
