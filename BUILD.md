# Build & Dev Commands

## Compilazione rapida

### Solo exe (tutti i binari) — modo consigliato

```powershell
# Incrementale: ricompila solo i pacchetti modificati, copia in dist/
cargo xtask dist
```

```powershell
# Full rebuild (pulisce prima, poi ricompila tutto da zero)
cargo xtask dist --clean
```

### Exe + installer Windows in un colpo solo

```powershell
# Incrementale build + installer
cargo xtask dist --installer

# Full rebuild + installer
cargo xtask dist --clean --installer
```

---

## Script PowerShell alternativi

```powershell
# Solo exe → dist/
.\scripts\build-release.ps1

# Solo installer (assume dist/*.exe già presenti)
.\scripts\build-installer.ps1 -SkipBuild

# Exe + installer completo
.\scripts\build-installer.ps1
```

Script Bash (Linux/macOS/Git Bash):

```bash
./scripts/build-release.sh
```

---

## Compilare un singolo pacchetto

```powershell
cargo build --release -p speedy-daemon
cargo build --release -p speedy-gui
cargo build --release -p speedy-cli
cargo build --release -p speedy-mcp
cargo build --release -p speedy-ai-context
cargo build --release -p speedy-language-context
```

Output in `target/release/<nome>.exe`.

---

## Build debug (sviluppo)

```powershell
# Workspace intero
cargo build

# Singolo pacchetto
cargo build -p speedy-daemon
```

Output in `target/debug/<nome>.exe`.

---

## Installer manuale con iscc (Inno Setup 6)

```powershell
# Installer base
iscc /DMyAppVersion=0.1.0 installer\speedy.iss

# Con icona custom
iscc /DMyAppVersion=0.1.0 /DMySetupIcon=installer\assets\speedy.ico installer\speedy.iss
```

Output: `dist\speedy-setup-<version>.exe`

Prerequisito: `winget install JRSoftware.InnoSetup`

---

## Test & Check

```powershell
# Test workspace intero
cargo test

# Test singolo pacchetto
cargo test -p speedy-daemon

# Check senza compilare (veloce per syntax/type check)
cargo check

# Check singolo pacchetto
cargo check -p speedy-gui

# Clippy (linter)
cargo clippy --workspace
```

---

## GitHub Actions

### CI (`.github/workflows/ci.yml`)

Si attiva su ogni **push/PR verso `main`**. Gira in parallelo su Ubuntu, macOS e Windows.

| Step | Comando |
|---|---|
| Build | `cargo build --workspace` |
| Test | `cargo test --workspace` |
| Check bench | `cargo bench --workspace --no-run` |

Non serve fare nulla manualmente: parte automaticamente.

---

### Release (`.github/workflows/release.yml`)

Si attiva **pushando un tag `v*`** (es. `v0.2.0`).

```powershell
# Crea il tag e pusha — avvia la pipeline di release
git tag v0.2.0
git push origin v0.2.0
```

Produce per ogni piattaforma un archivio `.tar.gz` e lo carica come asset della GitHub Release:

| Target | OS runner | Binari inclusi |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | tutti + GUI |
| `x86_64-pc-windows-msvc` | windows-latest | tutti + GUI |
| `aarch64-apple-darwin` | macos-latest | tutti **senza** GUI |

Gli asset hanno il formato `speedy-<target>.tar.gz` e vengono caricati automaticamente sulla Release con note generate da GitHub.

> **L'installer Windows** (`speedy-setup-*.exe`) **non** è prodotto dalla CI — va creato localmente con `cargo xtask dist --installer` e caricato manualmente sulla Release.

#### Re-release (tag o release fallita)

Se la release è fallita o il tag esiste già, eliminare e ricreare:

```powershell
# 1. Elimina tag locale e remoto
git tag -d v0.2.0
git push GitHub --delete v0.2.0

# 2. Elimina la GitHub Release (se esiste)
gh release delete v0.2.0 --yes

# 3. Ricrea il tag e pusha — riavvia la Action
git tag v0.2.0
git push GitHub v0.2.0
```

---

## Binari prodotti

| Eseguibile | Pacchetto Cargo | Ruolo |
|---|---|---|
| `speedy-daemon.exe` | `speedy-daemon` | Daemon globale — file watcher, IPC server |
| `speedy-ai-context.exe` | `speedy-ai-context` | Worker — indexing, query, embedding, SQLite |
| `speedy-cli.exe` | `speedy-cli` | Client thin — CLI per agent e scripting |
| `speedy-mcp.exe` | `speedy-mcp` | Server MCP (Claude, Cursor, …) |
| `speedy-gui.exe` | `speedy-gui` | GUI desktop — gestione workspaces e log |
| `speedy-language-context.exe` | `speedy-language-context` | Code intelligence |

Tutti i binari finiscono in `dist/` dopo `cargo xtask dist` o gli script.

---

## Output directory

```
dist/
├── speedy-ai-context.exe
├── speedy-daemon.exe
├── speedy-cli.exe
├── speedy-mcp.exe
├── speedy-gui.exe
├── speedy-language-context.exe
├── speedy-setup-<version>.exe       ← installer
└── speedy-uninstall-<version>.exe   ← uninstaller runner
```
