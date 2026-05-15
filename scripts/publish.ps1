param(
    [Parameter(Mandatory)][string]$Version
)
$ErrorActionPreference = 'Stop'

# Normalizza: accetta "0.2.0" o "v0.2.0"
if ($Version -notmatch '^v') { $Version = "v$Version" }

# 1. Controlla branch
$branch = git rev-parse --abbrev-ref HEAD
if ($branch -ne 'master') {
    Write-Error "Devi essere su master (branch attuale: $branch)"
    exit 1
}

# 2. Controlla working tree pulito
$dirty = git status --porcelain
if ($dirty) {
    Write-Error "Working tree non pulito. Committa o stasha le modifiche prima."
    exit 1
}

# 3. Controlla che il tag non esista gia'
$existing = git tag -l $Version
if ($existing) {
    Write-Error "Tag $Version esiste gia'."
    exit 1
}

Write-Host "`n==> Release $Version da master" -ForegroundColor Green

# 4. Push dei commit
Write-Host "==> Push master..." -ForegroundColor Yellow
git push GitHub master
if ($LASTEXITCODE -ne 0) { throw 'git push master fallito' }

# 5. Crea e pusha il tag — parte il workflow release.yml su GitHub Actions
Write-Host "==> Tag $Version e push..." -ForegroundColor Yellow
git tag $Version
git push GitHub $Version
if ($LASTEXITCODE -ne 0) { throw 'git push tag fallito' }

Write-Host "`n✅ Tag $Version pushato. GitHub Actions sta buildando gli exe." -ForegroundColor Green
Write-Host "   Segui la build su: https://github.com/elguala9/Speedy/actions" -ForegroundColor Cyan
