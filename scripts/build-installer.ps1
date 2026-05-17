<#
.SYNOPSIS
    Builda i binari Speedy e produce l'installer e l'uninstaller Windows (.exe) con Inno Setup.

.DESCRIPTION
    Sequenza completa:
      1. Legge la versione da Cargo.toml (workspace root)
      2. Esegue build-release.ps1 → dist\*.exe  (saltato con -SkipBuild)
      3. Genera installer\assets\speedy.ico se non esiste (via make-icon.ps1)
      4. Cerca iscc.exe (PATH, percorsi standard, winget/choco)
      5. Compila installer\speedy.iss → dist\speedy-setup-<version>.exe
      6. Compila installer\speedy-uninstall.iss → dist\speedy-uninstall-<version>.exe

.PARAMETER SkipBuild
    Salta la build dei binari (usa i .exe già presenti in dist\).
    Usato da cargo xtask dist che ha già compilato i binari.

.OUTPUTS
    dist\speedy-setup-<version>.exe
    dist\speedy-uninstall-<version>.exe

.EXAMPLE
    .\scripts\build-installer.ps1
    .\scripts\build-installer.ps1 -SkipBuild
#>
param(
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root       = Split-Path $PSScriptRoot -Parent
$dist       = Join-Path $root 'dist'
$assetsDir  = Join-Path $root 'installer\assets'
$iconPath   = Join-Path $assetsDir 'speedy.ico'
$issPath    = Join-Path $root 'installer\speedy.iss'

# ------------------------------------------------------------------
# 1. Leggi versione da Cargo.toml
# ------------------------------------------------------------------
$cargoToml = Join-Path $root 'Cargo.toml'
if (-not (Test-Path $cargoToml)) { throw "Cargo.toml non trovato in $root" }

$match = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) { throw "Campo 'version' non trovato in $cargoToml" }
$version = $match.Matches[0].Groups[1].Value

Write-Host ''
Write-Host "==> Speedy installer build - versione $version" -ForegroundColor Cyan
Write-Host ''

# ------------------------------------------------------------------
# 2. Build binari (saltato se -SkipBuild)
# ------------------------------------------------------------------
if ($SkipBuild) {
    Write-Host '==> [1/3] Build binari skippata (-SkipBuild).' -ForegroundColor DarkGray
} else {
    Write-Host '==> [1/3] Build binari release...' -ForegroundColor Yellow
    $buildScript = Join-Path $PSScriptRoot 'build-release.ps1'
    & $buildScript
}

Write-Host ''

# ------------------------------------------------------------------
# 3. Icona: genera se mancante
# ------------------------------------------------------------------
Write-Host '==> [2/3] Verifica icona...' -ForegroundColor Yellow

if (Test-Path $iconPath) {
    $iconSizeKB = [Math]::Round((Get-Item $iconPath).Length / 1KB, 1)
    Write-Host "    speedy.ico gia' presente ($iconSizeKB KB)" -ForegroundColor DarkGray
} else {
    $makeIconScript = Join-Path $assetsDir 'make-icon.ps1'
    if (Test-Path $makeIconScript) {
        Write-Host '    Generazione icona con make-icon.ps1...' -ForegroundColor DarkGray
        & $makeIconScript
    } else {
        Write-Warning "make-icon.ps1 non trovato - l'installer usera' l'icona di default di Inno Setup"
    }
}

Write-Host ''

# ------------------------------------------------------------------
# 4. Cerca iscc.exe (Inno Setup 6 compiler)
# ------------------------------------------------------------------
Write-Host '==> [3/4] Ricerca iscc.exe...' -ForegroundColor Yellow

$isccExe = $null

# Prova nel PATH di sistema
$cmd = Get-Command 'iscc.exe' -ErrorAction SilentlyContinue
if ($cmd) {
    $isccExe = $cmd.Source
    Write-Host "    Trovato nel PATH: $isccExe" -ForegroundColor DarkGray
}

# Prova nei percorsi standard di installazione
if (-not $isccExe) {
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\iscc.exe",
        "$env:ProgramFiles\Inno Setup 6\iscc.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\iscc.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) {
            $isccExe = $c
            Write-Host "    Trovato: $isccExe" -ForegroundColor DarkGray
            break
        }
    }
}

if (-not $isccExe) {
    Write-Host ''
    Write-Host 'ERRORE: iscc.exe non trovato.' -ForegroundColor Red
    Write-Host ''
    Write-Host 'Installa Inno Setup 6 con uno di questi comandi:' -ForegroundColor Yellow
    Write-Host '  winget install JRSoftware.InnoSetup'
    Write-Host '  choco install innosetup -y'
    Write-Host '  oppure scarica da: https://jrsoftware.org/isdl.php'
    Write-Host ''
    exit 1
}

# ------------------------------------------------------------------
# 5. Compila l'installer
# ------------------------------------------------------------------
Write-Host ''
Write-Host '==> [4/4] Compilazione installer e uninstaller...' -ForegroundColor Yellow
Write-Host "    Installer v$version..." -ForegroundColor DarkGray

$isccArgs = [System.Collections.Generic.List[string]]::new()
$isccArgs.Add("/DMyAppVersion=$version")
$isccArgs.Add($issPath)
$isccArgs.Add("/O$dist")

if (Test-Path $iconPath) {
    $isccArgs.Add("/DMySetupIcon=$iconPath")
    Write-Host "    Con icona: $iconPath" -ForegroundColor DarkGray
} else {
    Write-Host "    Senza icona custom (usa quella di default di Inno Setup)" -ForegroundColor DarkGray
}

& $isccExe @isccArgs

if ($LASTEXITCODE -ne 0) {
    throw "iscc.exe ha restituito exit code $LASTEXITCODE - build installer fallita"
}

# ------------------------------------------------------------------
# 6. Compila l'uninstaller runner
# ------------------------------------------------------------------
Write-Host "    Uninstaller runner v$version..." -ForegroundColor DarkGray

$issUninstallPath = Join-Path $root 'installer\speedy-uninstall.iss'

$isccUninstArgs = [System.Collections.Generic.List[string]]::new()
$isccUninstArgs.Add("/DMyAppVersion=$version")
$isccUninstArgs.Add($issUninstallPath)
$isccUninstArgs.Add("/O$dist")

if (Test-Path $iconPath) {
    $isccUninstArgs.Add("/DMySetupIcon=$iconPath")
}

& $isccExe @isccUninstArgs

if ($LASTEXITCODE -ne 0) {
    throw "iscc.exe ha restituito exit code $LASTEXITCODE - build uninstaller fallita"
}

# ------------------------------------------------------------------
# Output finale
# ------------------------------------------------------------------
Write-Host ''

$outFile        = Join-Path $dist "speedy-setup-$version.exe"
$outUninstFile  = Join-Path $dist "speedy-uninstall-$version.exe"

$allOk = (Test-Path $outFile) -and (Test-Path $outUninstFile)

if ($allOk) {
    $sizeMB        = [Math]::Round((Get-Item $outFile).Length / 1MB, 1)
    $uninstSizeKB  = [Math]::Round((Get-Item $outUninstFile).Length / 1KB, 1)
    Write-Host "========================================" -ForegroundColor Green
    Write-Host " Build completata!" -ForegroundColor Green
    Write-Host " Installer:    $outFile ($sizeMB MB)" -ForegroundColor Green
    Write-Host " Uninstaller:  $outUninstFile ($uninstSizeKB KB)" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
} else {
    Write-Host "Attenzione: uno o piu' file di output mancanti." -ForegroundColor Yellow
    Write-Host 'File presenti in dist\:' -ForegroundColor Yellow
    Get-ChildItem $dist | ForEach-Object {
        Write-Host "  $($_.Name)  ($([Math]::Round($_.Length/1KB)) KB)"
    }
}
