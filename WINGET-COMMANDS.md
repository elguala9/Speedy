# Winget — Publish Commands

## First submission

```powershell
cargo xtask publish-winget
```

Or with an explicit version:

```powershell
cargo xtask publish-winget --version 0.2.0

.\scripts\submit-winget.ps1 -Version 0.2.0
```

Opens the browser for GitHub login and creates the PR on `microsoft/winget-pkgs`.
Microsoft review: **1–3 business days**.

---

## Version update

```powershell
cargo xtask publish-winget --update
```

Or with an explicit version:

```powershell
cargo xtask publish-winget --update --version 0.3.0

.\scripts\submit-winget.ps1 -Version 0.3.0 -Update
```

---

## Prerequisites before publishing

```powershell
# 1. Build the tag and push it (triggers GitHub Actions → produces the .tar.gz files)
git tag v0.2.0
git push GitHub v0.2.0

# 2. Build the installer locally (not produced by CI)
cargo xtask dist --installer

# 3. Upload the installer to the GitHub Release manually
#    or via the gh CLI:
gh release upload v0.2.0 dist\speedy-setup-0.2.0.exe
```

At that point the installer is reachable at the URL expected by wingetcreate:
`https://github.com/elguala9/Speedy/releases/download/v0.2.0/speedy-setup-0.2.0.exe`

---

## Install wingetcreate (if missing)

```powershell
winget install --id Microsoft.WingetCreate
```

The `submit-winget.ps1` script installs it automatically if missing.
