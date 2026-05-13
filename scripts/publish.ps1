$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
$ver = Select-String -Path (Join-Path (Join-Path $root 'packages') 'speedy-core/Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | ForEach-Object { $_.Matches.Groups[1].Value }

Write-Host "`n==> Pubblico speedy v$ver...`n" -ForegroundColor Green

Write-Host '==> Build release...' -ForegroundColor Yellow
cargo build --release --workspace
if ($LASTEXITCODE -ne 0) { throw 'Build fallito' }

Write-Host "`n==> Publish speedy su crates.io..." -ForegroundColor Yellow
cargo publish -p speedy
if ($LASTEXITCODE -ne 0) { throw 'cargo publish speedy fallito' }

Write-Host "`n==> Publish speedy-mcp su crates.io..." -ForegroundColor Yellow
cargo publish -p speedy-mcp
if ($LASTEXITCODE -ne 0) { throw 'cargo publish speedy-mcp fallito' }

Write-Host "`n==> Creo GitHub Release v$ver..." -ForegroundColor Yellow
gh release create "v$ver" `
    'target/release/speedy.exe' `
    'target/release/speedy-daemon.exe' `
    'target/release/speedy-cli.exe' `
    'target/release/speedy-mcp.exe' `
    --title "Speedy v$ver" --notes "Vedi CHANGELOG per i dettagli." --generate-notes
if ($LASTEXITCODE -ne 0) { throw 'GitHub Release fallita' }

Write-Host '`n==> Aggiorno README.md...' -ForegroundColor Yellow
$oldUrl = 'https://github.com/elguala9/Speedy/releases/download/v[^/]+/speedy.exe'
$newUrl = "https://github.com/elguala9/Speedy/releases/download/v$ver/speedy.exe"
$readme = Join-Path $root 'README.md'
$content = Get-Content $readme -Raw
if ($content -match $oldUrl) {
    $content = $content -replace $oldUrl, $newUrl
    Set-Content $readme -Value $content -NoNewline
    Write-Host "README.md aggiornato a v$ver" -ForegroundColor Green
} else {
    Write-Host 'Pattern URL non trovato in README.md, aggiornalo manualmente' -ForegroundColor Red
}

git add README.md
git commit -m "release: v$ver"
git tag "v$ver"

Write-Host "`n==> Push tag su remote..." -ForegroundColor Yellow
git push
git push --tags

Write-Host "`n✅ Pubblicazione v$ver completata!`n" -ForegroundColor Green
