# Install the Speedy plugin MCP binaries (speedy-language-context-mcp and
# speedy-text-context-mcp) into a directory on PATH. They ship inside the
# per-target Speedy release tarball, so this downloads that tarball and extracts
# just the two MCP binaries.
#
# Usage:
#   ./install.ps1                       # latest release
#   ./install.ps1 -Tag v0.2.2
#   ./install.ps1 -BinDir C:\tools\bin
param(
    [string]$Tag = "",
    [string]$BinDir = "$env:LOCALAPPDATA\Programs\speedy-plugins"
)
$ErrorActionPreference = "Stop"

$repo = "elguala9/Speedy"
$target = "x86_64-pc-windows-msvc"   # only Windows target published
$asset = "speedy-$target.tar.gz"

if ([string]::IsNullOrEmpty($Tag)) {
    $url = "https://github.com/$repo/releases/latest/download/$asset"
} else {
    $url = "https://github.com/$repo/releases/download/$Tag/$asset"
}

$tmp = Join-Path $env:TEMP ("speedy-plugins-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    Write-Host "Downloading $asset ..."
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmp $asset)
    # tar ships with Windows 10+ and handles .tar.gz.
    & tar -xzf (Join-Path $tmp $asset) -C $tmp "speedy-language-context-mcp.exe" "speedy-text-context-mcp.exe"

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    foreach ($bin in @("speedy-language-context-mcp.exe", "speedy-text-context-mcp.exe")) {
        Copy-Item -Force (Join-Path $tmp $bin) (Join-Path $BinDir $bin)
    }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Installed to ${BinDir}:"
Write-Host "  speedy-language-context-mcp.exe"
Write-Host "  speedy-text-context-mcp.exe"

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$BinDir*") {
    Write-Host ""
    Write-Host "NOTE: $BinDir is not on your PATH. Add it with:"
    Write-Host "  [Environment]::SetEnvironmentVariable('Path', `"$BinDir;`$([Environment]::GetEnvironmentVariable('Path','User'))`", 'User')"
}
