<#
.SYNOPSIS
    Builds the Speedy binaries and produces the Windows installer and uninstaller (.exe) with Inno Setup.

.DESCRIPTION
    Full sequence:
      1. Reads the version from Cargo.toml (workspace root)
      2. Runs build-release.ps1 → dist\*.exe  (skipped with -SkipBuild)
      3. Generates installer\assets\speedy.ico if it does not exist (via make-icon.ps1)
      4. Looks for iscc.exe (PATH, standard paths, winget/choco)
      5. Compiles installer\speedy.iss → dist\speedy-setup-<version>.exe
      6. Compiles installer\speedy-uninstall.iss → dist\speedy-uninstall-<version>.exe

.PARAMETER SkipBuild
    Skips building the binaries (uses the .exe files already present in dist\).
    Used by cargo xtask dist which has already compiled the binaries.

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
# 1. Read version from Cargo.toml
# ------------------------------------------------------------------
$cargoToml = Join-Path $root 'Cargo.toml'
if (-not (Test-Path $cargoToml)) { throw "Cargo.toml not found in $root" }

$match = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) { throw "'version' field not found in $cargoToml" }
$version = $match.Matches[0].Groups[1].Value

Write-Host ''
Write-Host "==> Speedy installer build - version $version" -ForegroundColor Cyan
Write-Host ''

# ------------------------------------------------------------------
# 2. Build binaries (skipped if -SkipBuild)
# ------------------------------------------------------------------
if ($SkipBuild) {
    Write-Host '==> [1/3] Binary build skipped (-SkipBuild).' -ForegroundColor DarkGray
} else {
    Write-Host '==> [1/3] Building release binaries...' -ForegroundColor Yellow
    $buildScript = Join-Path $PSScriptRoot 'build-release.ps1'
    & $buildScript
}

Write-Host ''

# ------------------------------------------------------------------
# 3. Icon: generate if missing
# ------------------------------------------------------------------
Write-Host '==> [2/3] Checking icon...' -ForegroundColor Yellow

if (Test-Path $iconPath) {
    $iconSizeKB = [Math]::Round((Get-Item $iconPath).Length / 1KB, 1)
    Write-Host "    speedy.ico already present ($iconSizeKB KB)" -ForegroundColor DarkGray
} else {
    $makeIconScript = Join-Path $assetsDir 'make-icon.ps1'
    if (Test-Path $makeIconScript) {
        Write-Host '    Generating icon with make-icon.ps1...' -ForegroundColor DarkGray
        & $makeIconScript
    } else {
        Write-Warning "make-icon.ps1 not found - the installer will use the default Inno Setup icon"
    }
}

Write-Host ''

# ------------------------------------------------------------------
# 4. Look for iscc.exe (Inno Setup 6 compiler)
# ------------------------------------------------------------------
Write-Host '==> [3/4] Searching for iscc.exe...' -ForegroundColor Yellow

$isccExe = $null

# Try the system PATH
$cmd = Get-Command 'iscc.exe' -ErrorAction SilentlyContinue
if ($cmd) {
    $isccExe = $cmd.Source
    Write-Host "    Found in PATH: $isccExe" -ForegroundColor DarkGray
}

# Try the standard installation paths
if (-not $isccExe) {
    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\iscc.exe",
        "$env:ProgramFiles\Inno Setup 6\iscc.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\iscc.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) {
            $isccExe = $c
            Write-Host "    Found: $isccExe" -ForegroundColor DarkGray
            break
        }
    }
}

if (-not $isccExe) {
    Write-Host ''
    Write-Host 'ERROR: iscc.exe not found.' -ForegroundColor Red
    Write-Host ''
    Write-Host 'Install Inno Setup 6 with one of these commands:' -ForegroundColor Yellow
    Write-Host '  winget install JRSoftware.InnoSetup'
    Write-Host '  choco install innosetup -y'
    Write-Host '  or download from: https://jrsoftware.org/isdl.php'
    Write-Host ''
    exit 1
}

# ------------------------------------------------------------------
# 5. Compile the installer
# ------------------------------------------------------------------
Write-Host ''
Write-Host '==> [4/4] Compiling installer and uninstaller...' -ForegroundColor Yellow
Write-Host "    Installer v$version..." -ForegroundColor DarkGray

$isccArgs = [System.Collections.Generic.List[string]]::new()
$isccArgs.Add("/DMyAppVersion=$version")
$isccArgs.Add($issPath)
$isccArgs.Add("/O$dist")

if (Test-Path $iconPath) {
    $isccArgs.Add("/DMySetupIcon=$iconPath")
    Write-Host "    With icon: $iconPath" -ForegroundColor DarkGray
} else {
    Write-Host "    Without custom icon (uses the default Inno Setup one)" -ForegroundColor DarkGray
}

& $isccExe @isccArgs

if ($LASTEXITCODE -ne 0) {
    throw "iscc.exe returned exit code $LASTEXITCODE - installer build failed"
}

# ------------------------------------------------------------------
# 6. Compile the uninstaller runner
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
    throw "iscc.exe returned exit code $LASTEXITCODE - uninstaller build failed"
}

# ------------------------------------------------------------------
# Final output
# ------------------------------------------------------------------
Write-Host ''

$outFile        = Join-Path $dist "speedy-setup-$version.exe"
$outUninstFile  = Join-Path $dist "speedy-uninstall-$version.exe"

$allOk = (Test-Path $outFile) -and (Test-Path $outUninstFile)

if ($allOk) {
    $sizeMB        = [Math]::Round((Get-Item $outFile).Length / 1MB, 1)
    $uninstSizeKB  = [Math]::Round((Get-Item $outUninstFile).Length / 1KB, 1)
    Write-Host "========================================" -ForegroundColor Green
    Write-Host " Build complete!" -ForegroundColor Green
    Write-Host " Installer:    $outFile ($sizeMB MB)" -ForegroundColor Green
    Write-Host " Uninstaller:  $outUninstFile ($uninstSizeKB KB)" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
} else {
    Write-Host "Warning: one or more output files are missing." -ForegroundColor Yellow
    Write-Host 'Files present in dist\:' -ForegroundColor Yellow
    Get-ChildItem $dist | ForEach-Object {
        Write-Host "  $($_.Name)  ($([Math]::Round($_.Length/1KB)) KB)"
    }
}
