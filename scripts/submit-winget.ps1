<#
.SYNOPSIS
    Invia il manifest winget per una nuova versione di Speedy a microsoft/winget-pkgs.

.DESCRIPTION
    Prima submission:  .\scripts\submit-winget.ps1 -Version 0.2.0
    Aggiornamento:     .\scripts\submit-winget.ps1 -Version 0.3.0 -Update

    Il flag -Update usa 'wingetcreate update' invece di 'wingetcreate new'.
    Usa -Update per tutte le versioni successive alla prima submission accettata.

    Requisiti:
      - La release GitHub v<Version> deve esistere con speedy-setup-<Version>.exe
      - wingetcreate viene installato automaticamente se mancante
      - Serve un account GitHub per aprire la PR su winget-pkgs

.PARAMETER Version
    Versione da pubblicare, es. 0.2.0 oppure v0.2.0

.PARAMETER Update
    Usa 'wingetcreate update' (per aggiornamenti dopo la prima submission)

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
Write-Host "    Modalita'  : $( if ($Update) { 'update' } else { 'new (prima submission)' } )" -ForegroundColor DarkGray
Write-Host ""

# ------------------------------------------------------------------
# 1. Installa wingetcreate se mancante
# ------------------------------------------------------------------
$wgc = Get-Command 'wingetcreate' -ErrorAction SilentlyContinue
if (-not $wgc) {
    Write-Host "==> wingetcreate non trovato, installo..." -ForegroundColor Yellow
    winget install --id Microsoft.WingetCreate --silent --accept-source-agreements --accept-package-agreements
    if ($LASTEXITCODE -ne 0) { throw "Installazione wingetcreate fallita" }

    # Ricarica PATH nella sessione corrente
    $env:PATH = [System.Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [System.Environment]::GetEnvironmentVariable('Path', 'User')

    $wgc = Get-Command 'wingetcreate' -ErrorAction SilentlyContinue
    if (-not $wgc) {
        Write-Error "wingetcreate non trovato dopo l'installazione. Riavvia PowerShell e riprova."
        exit 1
    }
}

Write-Host "==> wingetcreate: $($wgc.Source)" -ForegroundColor DarkGray
Write-Host ""

# ------------------------------------------------------------------
# 2. Verifica che la release GitHub esista
# ------------------------------------------------------------------
Write-Host "==> Verifico che la release GitHub esista..." -ForegroundColor Yellow
try {
    $response = Invoke-WebRequest -Uri $installerUrl -Method Head -UseBasicParsing -ErrorAction Stop
    Write-Host "    OK ($([Math]::Round($response.Headers['Content-Length'] / 1MB, 1)) MB)" -ForegroundColor DarkGray
} catch {
    Write-Error @"
Release non trovata: $installerUrl

Assicurati di aver:
  1. Eseguito scripts\publish.ps1 -Version $Version
  2. Atteso che GitHub Actions completi la build
  3. Verificato che il file esista nella release: https://github.com/elguala9/Speedy/releases/tag/$vTag
"@
    exit 1
}

Write-Host ""

# ------------------------------------------------------------------
# 3. Genera manifest e apri PR
# ------------------------------------------------------------------
if ($Update) {
    Write-Host "==> wingetcreate update (aggiornamento versione)..." -ForegroundColor Yellow
    Write-Host "    Si aprira' il browser per autenticarsi con GitHub." -ForegroundColor DarkGray
    Write-Host ""
    wingetcreate update $pkgId --version $Version --urls $installerUrl --submit
} else {
    Write-Host "==> wingetcreate new (prima submission)..." -ForegroundColor Yellow
    Write-Host "    Si aprira' il browser per autenticarsi con GitHub." -ForegroundColor DarkGray
    Write-Host "    wingetcreate chiedera' di riempire alcuni campi interattivamente." -ForegroundColor DarkGray
    Write-Host ""
    wingetcreate new $installerUrl
}

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "ATTENZIONE: wingetcreate ha restituito exit code $LASTEXITCODE" -ForegroundColor Yellow
    Write-Host "La PR potrebbe non essere stata aperta. Controlla l'output sopra." -ForegroundColor Yellow
    exit $LASTEXITCODE
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host " Manifest inviato!" -ForegroundColor Green
Write-Host " Controlla la PR su: https://github.com/microsoft/winget-pkgs/pulls" -ForegroundColor Green
Write-Host " La review di winget-pkgs richiede tipicamente 1-3 giorni lavorativi." -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
