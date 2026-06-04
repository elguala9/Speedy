param(
    [Parameter(Mandatory)][string]$Version
)
$ErrorActionPreference = 'Stop'

# Normalize: accept "0.2.0" or "v0.2.0"
if ($Version -notmatch '^v') { $Version = "v$Version" }

# 1. Check branch
$branch = git rev-parse --abbrev-ref HEAD
if ($branch -ne 'master') {
    Write-Error "You must be on master (current branch: $branch)"
    exit 1
}

# 2. Check that the working tree is clean
$dirty = git status --porcelain
if ($dirty) {
    Write-Error "Working tree not clean. Commit or stash your changes first."
    exit 1
}

# 3. Check that the tag does not already exist
$existing = git tag -l $Version
if ($existing) {
    Write-Error "Tag $Version already exists."
    exit 1
}

Write-Host "`n==> Release $Version from master" -ForegroundColor Green

# 4. Push the commits
Write-Host "==> Push master..." -ForegroundColor Yellow
git push GitHub master
if ($LASTEXITCODE -ne 0) { throw 'git push master failed' }

# 5. Create and push the tag — triggers the release.yml workflow on GitHub Actions
Write-Host "==> Tag $Version and push..." -ForegroundColor Yellow
git tag $Version
git push GitHub $Version
if ($LASTEXITCODE -ne 0) { throw 'git push tag failed' }

Write-Host "`n✅ Tag $Version pushed. GitHub Actions is building the exe files." -ForegroundColor Green
Write-Host "   Follow the build at: https://github.com/elguala9/Speedy/actions" -ForegroundColor Cyan
