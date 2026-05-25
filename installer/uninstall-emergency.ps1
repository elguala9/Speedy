<#
.SYNOPSIS
    Disinstallazione di emergenza di Speedy.

.DESCRIPTION
    Da usare SE e SOLO SE l'uninstaller standard di Inno Setup (unins000.exe)
    si blocca o non parte. Lo script fa tutto a mano:
      1. Kill di tutti i processi speedy-* e unins*
      2. Rimozione dei link di avvio automatico e Start Menu
      3. Rimozione della voce di Speedy dal PATH utente
      4. Pulizia delle chiavi di registro HKCU
      5. Rimozione della voce in "Installazione applicazioni"
      6. Cancellazione della cartella d'installazione con .NET File.Delete
         (bypassa i lock spuri di PowerShell/Inno Setup)

    NON tocca i dati utente (workspaces.json, indici .speedy/, configurazione).
    Per eliminare anche quelli, passa il flag -PurgeUserData.

.PARAMETER PurgeUserData
    Elimina anche %APPDATA%\speedy\, %APPDATA%\Speedy GUI\, %USERPROFILE%\.speedy\

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
Write-Host '==> Disinstallazione di emergenza Speedy' -ForegroundColor Cyan
Write-Host ''

# ------------------------------------------------------------------
# 1. Kill processi
# ------------------------------------------------------------------
Write-Host '[1/6] Kill processi Speedy e uninstaller...' -ForegroundColor Yellow
Get-Process -Name "speedy*","unins*","_unins*" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 1500
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 2. Shortcut: startup, Start Menu, Desktop
# ------------------------------------------------------------------
Write-Host '[2/6] Rimozione shortcut...' -ForegroundColor Yellow
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
# 3. PATH utente
# ------------------------------------------------------------------
Write-Host '[3/6] Rimozione voce dal PATH utente...' -ForegroundColor Yellow
$currentPath = [System.Environment]::GetEnvironmentVariable('Path', 'User')
if ($currentPath) {
    $newPath = ($currentPath -split ';' | Where-Object { $_ -notlike "*Programs\Speedy*" -and $_ -ne '' }) -join ';'
    if ($newPath -ne $currentPath) {
        [System.Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    }
}
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 4. Registro HKCU
# ------------------------------------------------------------------
Write-Host '[4/6] Pulizia chiavi di registro...' -ForegroundColor Yellow
Remove-Item 'HKCU:\Software\Speedy' -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\Parresia\Speedy' -Recurse -Force -ErrorAction SilentlyContinue
# Rimuove Parresia solo se vuota
$parresia = 'HKCU:\Software\Parresia'
if (Test-Path $parresia) {
    if ((Get-ChildItem $parresia -ErrorAction SilentlyContinue).Count -eq 0) {
        Remove-Item $parresia -Force -ErrorAction SilentlyContinue
    }
}
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 5. Voce in Installazione applicazioni (Add/Remove Programs)
# ------------------------------------------------------------------
Write-Host '[5/6] Rimozione voce "Installazione applicazioni"...' -ForegroundColor Yellow
Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
    Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName -like '*Speedy*' } |
    Remove-Item -Recurse -Force
Write-Host '      OK' -ForegroundColor DarkGray

# ------------------------------------------------------------------
# 6. Cancellazione cartella d'installazione
#    Usa [System.IO.File]::Delete diretto: bypassa i lock spuri che
#    fermano Remove-Item / Inno Setup (handle di antivirus, thumbnail
#    cache di Esplora Risorse, Restart Manager, ecc.).
# ------------------------------------------------------------------
Write-Host '[6/6] Rimozione cartella di installazione...' -ForegroundColor Yellow
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
        Write-Host "      Alcuni file sono lockati e saranno cancellati al prossimo riavvio:" -ForegroundColor Yellow
        foreach ($f in $locked) { Write-Host "        $f" -ForegroundColor DarkYellow }
        Write-Host '      Riavvia Windows per completare la rimozione.' -ForegroundColor Yellow
    } elseif (Test-Path $installDir) {
        Write-Host '      ATTENZIONE: la cartella esiste ancora.' -ForegroundColor Red
    } else {
        Write-Host '      OK' -ForegroundColor DarkGray
    }
} else {
    Write-Host '      Cartella già rimossa.' -ForegroundColor DarkGray
}

# ------------------------------------------------------------------
# 7. (opzionale) Dati utente
# ------------------------------------------------------------------
if ($PurgeUserData) {
    Write-Host ''
    Write-Host '==> Rimozione dati utente (-PurgeUserData)' -ForegroundColor Yellow
    $userData = @(
        "$env:APPDATA\speedy"
        "$env:APPDATA\Speedy GUI"
        "$env:LOCALAPPDATA\Speedy GUI"
        "$env:USERPROFILE\.speedy"
    )
    foreach ($d in $userData) {
        if (Test-Path $d) {
            Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue
            Write-Host "      Eliminata: $d" -ForegroundColor DarkGray
        }
    }
}

Write-Host ''
Write-Host '========================================' -ForegroundColor Green
Write-Host ' Speedy disinstallato.' -ForegroundColor Green
if (-not $PurgeUserData) {
    Write-Host ' (i dati utente sono stati mantenuti — usa -PurgeUserData per eliminarli)' -ForegroundColor DarkGray
}
Write-Host '========================================' -ForegroundColor Green
Write-Host ''
