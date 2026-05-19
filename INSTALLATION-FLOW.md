# Speedy — Installation Flow

This document describes every step performed by `speedy-setup-<version>.exe`
(Inno Setup installer), whether launched directly or via `winget install Parresia.Speedy`.

---

## Entry Points

| Method | Notes |
|--------|-------|
| **Direct download** | Run `speedy-setup-<version>.exe` — interactive wizard |
| **winget** | `winget install Parresia.Speedy` — silent by default (`/SILENT`) |

---

## Step-by-Step Flow

### 1. Copy Binaries

Destination: `%LOCALAPPDATA%\Programs\Speedy\`

| Binary | Role |
|--------|------|
| `speedy-daemon.exe` | Background indexing daemon |
| `speedy-gui.exe` | Desktop GUI |
| `speedy-cli.exe` | CLI interface |
| `speedy-ai-context.exe` | AI context engine |
| `speedy-ai-context-mcp.exe` | MCP server for AI context |
| `speedy-language-context.exe` | Language/LSP context engine |
| `speedy-language-context-mcp.exe` | MCP server for language context |

Documentation also copied: `README.txt`, `INSTALLATION.md`, `FOR-IA.md`.

---

### 2. Optional Tasks (wizard checkboxes)

All tasks are **checked by default** unless noted. In silent/winget installs the
user never sees the wizard — tasks run based on defaults + the `Check:` condition.

#### `addtopath` — Add to PATH *(checked by default)*
Appends `%LOCALAPPDATA%\Programs\Speedy` to `HKCU\Environment\Path`.
Skipped if the path is already present (duplicate check).

#### `autostart` — Start daemon at login *(checked by default)*
Creates a shortcut in `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\`
pointing to `speedy-daemon.exe`.

#### `desktopicon` — Desktop shortcut *(unchecked by default)*
Creates `%USERPROFILE%\Desktop\Speedy.lnk` → `speedy-gui.exe`.

#### `installoollama` — Install Ollama *(unchecked by default — opt-in)*

Shown only when `%LOCALAPPDATA%\Programs\Ollama\ollama.exe` is absent (hidden if already installed).
In silent/winget installs this task is **never executed** unless the user explicitly passes `/Tasks="installoollama"`.

1. Downloads `OllamaSetup.exe` from `https://ollama.com/download/OllamaSetup.exe`
   into the installer's temp folder (PowerShell, hidden).
2. Launches `OllamaSetup.exe` and waits for it to complete.
   > **Note:** Ollama's own installer opens a window even during a silent Speedy
   > install (no `/SILENT` flag is passed to it — see [known limitations](#known-limitations)).

#### `pullmodel` — Download default AI model *(unchecked by default — opt-in)*

Shown only when
`%USERPROFILE%\.ollama\models\manifests\registry.ollama.ai\library\nomic-embed-text\latest`
is absent.

- Waits 5 seconds (allows Ollama service to start after a fresh install).
- Runs: `ollama pull nomic-embed-text` (~270 MB download).
- Executed via PowerShell, hidden window.

---

### 3. Post-Install Actions (automatic)

| Action | Behavior |
|--------|----------|
| **Start daemon** | `speedy-daemon.exe` is launched immediately in the background (`runhidden`, no window). Does not wait for the next login. |
| **Open GUI** *(optional)* | Checkbox on the Finish page of the wizard. Skipped in silent mode. |
| **Open README** *(optional)* | Checkbox on the Finish page of the wizard. Skipped in silent mode. |

---

## Silent / winget Install Summary

```
winget install Parresia.Speedy
   │
   ├─ Copies binaries to %LOCALAPPDATA%\Programs\Speedy\
   ├─ Adds Speedy to user PATH
   ├─ Adds daemon to Windows Startup
   │
   ├─ [Ollama: NOT installed — opt-in, skipped in silent mode]
   │
   ├─ [nomic-embed-text: NOT installed — opt-in, skipped in silent mode]
   │
   └─ Start speedy-daemon.exe in background
```

---

## Uninstall Flow

Run via **Add/Remove Programs** or `winget uninstall Parresia.Speedy`.

1. Graceful daemon shutdown via `speedy-cli daemon stop`.
2. Force-kill of remaining processes (`taskkill /F`).
3. Removes `%LOCALAPPDATA%\Programs\Speedy` from user PATH.
4. Deletes Speedy registry keys under `HKCU\Software`.
5. Dialog asks whether to delete user data:
   - `%APPDATA%\speedy\` — workspaces, daemon logs, config
   - `%APPDATA%\Speedy GUI\` — GUI preferences
   - `%USERPROFILE%\.speedy\` — global user config
   - Project-level `.speedy\` folders are **never** touched.

> Ollama and downloaded models are **not** removed by Speedy's uninstaller.

---

## Known Limitations

- **Ollama installer is not silent**: during a `/VERYSILENT` winget install, Ollama's
  own setup wizard still opens a window. Fix: pass `/SILENT` to `OllamaSetup.exe`
  in `installer/speedy.iss` `[Run]` section.
- **Model download has no progress feedback** in silent mode (runs hidden).
- **No retry logic** if the `ollama pull` command fails (network error, etc.).
