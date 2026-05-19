# Winget — Publish Commands

## Prima submission

```powershell
cargo xtask publish-winget
```

Oppure con versione esplicita:

```powershell
cargo xtask publish-winget --version 0.2.0

.\scripts\submit-winget.ps1 -Version 0.2.0
```

Apre il browser per il login GitHub e crea la PR su `microsoft/winget-pkgs`.
Review Microsoft: **1–3 giorni lavorativi**.

---

## Aggiornamento versione

```powershell
cargo xtask publish-winget --update
```

Oppure con versione esplicita:

```powershell
cargo xtask publish-winget --update --version 0.3.0

.\scripts\submit-winget.ps1 -Version 0.3.0 -Update
```

---

## Prerequisiti prima di pubblicare

```powershell
# 1. Builda il tag e fai push (avvia GitHub Actions → produce i .tar.gz)
git tag v0.2.0
git push GitHub v0.2.0

# 2. Builda l'installer localmente (non prodotto da CI)
cargo xtask dist --installer

# 3. Carica l'installer sulla Release GitHub manualmente
#    oppure tramite gh CLI:
gh release upload v0.2.0 dist\speedy-setup-0.2.0.exe
```

A quel punto l'installer è raggiungibile all'URL atteso da wingetcreate:
`https://github.com/elguala9/Speedy/releases/download/v0.2.0/speedy-setup-0.2.0.exe`

---

## Installa wingetcreate (se mancante)

```powershell
winget install --id Microsoft.WingetCreate
```

Lo script `submit-winget.ps1` lo installa automaticamente se mancante.
