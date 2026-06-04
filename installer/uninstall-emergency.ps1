<#
.SYNOPSIS
    Emergency uninstall of Speedy.

.DESCRIPTION
    Use IF and ONLY IF the standard Inno Setup uninstaller (unins000.exe)
    hangs or won't start. This script does everything by hand:
      1. Kill all speedy-* and unins* processes
      2. Remove the autostart and Start Menu links
      3. Remove the Speedy entry from the user PATH
      4. Clean up the HKCU registry keys
      5. Remove the entry from "Installed apps"
      6. Delete the installation folder with .NET File.Delete
         (bypasses the spurious locks of PowerShell/Inno Setup)

    Does NOT touch user data (workspaces.json, .speedy/ indexes, configuration).
    To remove those as well, pass the -PurgeUserData flag.

.PARAMETER PurgeUserData
    Also deletes %APPDATA%\speedy\, %APPDATA%\Speedy GUI\, %USERPROFILE%\.speedy\

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\uninstall-emergency.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\uninstall-emergency.ps1 -PurgeUserData
#>
param(
    [switch]$PurgeUserData
)

$ErrorActionPreference = 'Continue'
$installDir = "$env:LOCALAPPDATA\Programs\Speedy"

Write-Host ''
Write-Host '==> Speedy emergency uninstall' -ForegroundColor Cyan
Write-Host ''

# ------------------------------------------------------------------
# 1. Kill processes
# ------------------------------------------------------------------
Write-Host '[1/6] Killing Speedy and uninstaller processes...' -ForegroundColor Yellow
Get-Process -Name "speedy*","unins*","_unins*" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 1500
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 2. Shortcuts: startup, Start Menu, Desktop
# ------------------------------------------------------------------
Write-Host '[2/6] Removing shortcuts...' -ForegroundColor Yellow
$shortcuts = @(
    "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\Speedy Daemon.lnk"
    "$env:USERPROFILE\Desktop\Speedy.lnk"
)
foreach ($s in $shortcuts) {
    if (Test-Path $s) { Remove-Item $s -Force -ErrorAction SilentlyContinue }
}
Remove-Item "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Speedy" -Recurse -Force -ErrorAction SilentlyContinue
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 3. User PATH
# ------------------------------------------------------------------
Write-Host '[3/6] Removing entry from user PATH...' -ForegroundColor Yellow
$currentPath = [System.Environment]::GetEnvironmentVariable('Path', 'User')
if ($currentPath) {
    $newPath = ($currentPath -split ';' | Where-Object { $_ -notlike "*Programs\Speedy*" -and $_ -ne '' }) -join ';'
    if ($newPath -ne $currentPath) {
        [System.Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    }
}
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 4. HKCU registry
# ------------------------------------------------------------------
Write-Host '[4/6] Cleaning up registry keys...' -ForegroundColor Yellow
Remove-Item 'HKCU:\Software\Speedy' -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\Parresia\Speedy' -Recurse -Force -ErrorAction SilentlyContinue
# Remove Parresia only if empty
$parresia = 'HKCU:\Software\Parresia'
if (Test-Path $parresia) {
    if ((Get-ChildItem $parresia -ErrorAction SilentlyContinue).Count -eq 0) {
        Remove-Item $parresia -Force -ErrorAction SilentlyContinue
    }
}
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 5. Entry in Installed apps (Add/Remove Programs)
# ------------------------------------------------------------------
Write-Host '[5/6] Removing "Installed apps" entry...' -ForegroundColor Yellow
Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
    Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName -like '*Speedy*' } |
    Remove-Item -Recurse -Force
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 6. Delete installation folder
#    Uses [System.IO.File]::Delete directly: bypasses the spurious locks
#    that stop Remove-Item / Inno Setup (antivirus handles, File Explorer
#    thumbnail cache, Restart Manager, etc.).
# ------------------------------------------------------------------
Write-Host '[6/6] Removing installation folder...' -ForegroundColor Yellow
if (Test-Path $installDir) {
    $locked = @()
    Get-ChildItem $installDir -Recurse -Force -File -ErrorAction SilentlyContinue | ForEach-Object {
        try { [System.IO.File]::Delete($_.FullName) }
        catch { $locked += $_.FullName }
    }
    Get-ChildItem $installDir -Recurse -Force -Directory -ErrorAction SilentlyContinue |
        Sort-Object { $_.FullName.Length } -Descending | ForEach-Object {
            try { [System.IO.Directory]::Delete($_.FullName, $true) } catch {}
        }
    try { [System.IO.Directory]::Delete($installDir, $true) } catch {}

    if ($locked.Count -gt 0) {
        Write-Host "      Some files are locked and will be deleted on the next reboot:" -ForegroundColor Yellow
        foreach ($f in $locked) { Write-Host "        $f" -ForegroundColor DarkYellow }
        Write-Host '      Restart Windows to complete the removal.' -ForegroundColor Yellow
    } elseif (Test-Path $installDir) {
        Write-Host '      WARNING: the folder still exists.' -ForegroundColor Red
    } else {
        Write-Host '      OK' -ForegroundColor DarkGray
    }
} else {
    Write-Host '      Folder already removed.' -ForegroundColor DarkGray
}

# ------------------------------------------------------------------
# 7. (optional) User data
# ------------------------------------------------------------------
if ($PurgeUserData) {
    Write-Host ''
    Write-Host '==> Removing user data (-PurgeUserData)' -ForegroundColor Yellow
    $userData = @(
        "$env:APPDATA\speedy"
        "$env:APPDATA\Speedy GUI"
        "$env:LOCALAPPDATA\Speedy GUI"
        "$env:USERPROFILE\.speedy"
    )
    foreach ($d in $userData) {
        if (Test-Path $d) {
            Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue
            Write-Host "      Deleted: $d" -ForegroundColor DarkGray
        }
    }
}

Write-Host ''
Write-Host '========================================' -ForegroundColor Green
Write-Host ' Speedy uninstalled.' -ForegroundColor Green
if (-not $PurgeUserData) {
    Write-Host ' (user data has been kept - use -PurgeUserData to remove it)' -ForegroundColor DarkGray
}
Write-Host '========================================' -ForegroundColor Green
Write-Host ''
