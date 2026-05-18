# Build & Dev Commands

## Quick build

### All binaries only — recommended method

```powershell
# Incremental: only recompiles modified packages, copies to dist/
cargo xtask dist
```

```powershell
# Full rebuild (cleans first, then recompiles everything from scratch)
cargo xtask dist --clean
```

### Binaries + Windows installer in one shot

```powershell
# Incremental build + installer
cargo xtask dist --installer

# Full rebuild + installer
cargo xtask dist --clean --installer
```

---

## Alternative PowerShell scripts

```powershell
# Binaries only → dist/
.\scripts\build-release.ps1

# Installer only (assumes dist/*.exe already present)
.\scripts\build-installer.ps1 -SkipBuild

# Full binaries + installer
.\scripts\build-installer.ps1
```

Bash script (Linux/macOS/Git Bash):

```bash
./scripts/build-release.sh
```

---

## Build a single package

```powershell
cargo build --release -p speedy-daemon
cargo build --release -p speedy-gui
cargo build --release -p speedy-cli
cargo build --release -p speedy-mcp
cargo build --release -p speedy-ai-context
cargo build --release -p speedy-language-context
```

Output in `target/release/<name>.exe`.

---

## Debug build (development)

```powershell
# Entire workspace
cargo build

# Single package
cargo build -p speedy-daemon
```

Output in `target/debug/<name>.exe`.

---

## Manual installer with iscc (Inno Setup 6)

```powershell
# Basic installer
iscc /DMyAppVersion=0.1.0 installer\speedy.iss

# With custom icon
iscc /DMyAppVersion=0.1.0 /DMySetupIcon=installer\assets\speedy.ico installer\speedy.iss
```

Output: `dist\speedy-setup-<version>.exe`

Prerequisite: `winget install JRSoftware.InnoSetup`

---

## Test & Check

```powershell
# Test entire workspace
cargo test

# Test single package
cargo test -p speedy-daemon

# Check without compiling (fast for syntax/type check)
cargo check

# Check single package
cargo check -p speedy-gui

# Clippy (linter)
cargo clippy --workspace
```

---

## GitHub Actions

### CI (`.github/workflows/ci.yml`)

Triggers on every **push/PR to `main`**. Runs in parallel on Ubuntu, macOS and Windows.

| Step | Command |
|---|---|
| Build | `cargo build --workspace` |
| Test | `cargo test --workspace` |
| Check bench | `cargo bench --workspace --no-run` |

No manual action needed: starts automatically.

---

### Release (`.github/workflows/release.yml`)

Triggers by **pushing a `v*` tag** (e.g. `v0.2.0`).

```powershell
# Create the tag and push — starts the release pipeline
git tag v0.2.0
git push origin v0.2.0
```

Produces a `.tar.gz` archive for each platform and uploads it as a GitHub Release asset:

| Target | OS runner | Included binaries |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | all + GUI |
| `x86_64-pc-windows-msvc` | windows-latest | all + GUI |
| `aarch64-apple-darwin` | macos-latest | all **without** GUI |

Assets are named `speedy-<target>.tar.gz` and are uploaded automatically to the Release with GitHub-generated notes.

> **The Windows installer** (`speedy-setup-*.exe`) **is not** produced by CI — build it locally with `cargo xtask dist --installer` and upload it manually to the Release.

#### Re-release (failed tag or release)

If the release failed or the tag already exists, delete and recreate:

```powershell
# 1. Delete local and remote tag
git tag -d v0.2.0
git push GitHub --delete v0.2.0

# 2. Delete the GitHub Release (if it exists)
gh release delete v0.2.0 --yes

# 3. Recreate the tag and push — restarts the Action
git tag v0.2.0
git push GitHub v0.2.0
```

---

## Produced binaries

| Executable | Cargo package | Role |
|---|---|---|
| `speedy-daemon.exe` | `speedy-daemon` | Global daemon — file watcher, IPC server |
| `speedy-ai-context.exe` | `speedy-ai-context` | Worker — indexing, query, embedding, SQLite |
| `speedy-cli.exe` | `speedy-cli` | Thin client — CLI for agents and scripting |
| `speedy-mcp.exe` | `speedy-mcp` | MCP server (Claude, Cursor, …) |
| `speedy-gui.exe` | `speedy-gui` | Desktop GUI — workspace and log management |
| `speedy-language-context.exe` | `speedy-language-context` | Code intelligence |

All binaries end up in `dist/` after `cargo xtask dist` or the scripts.

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
