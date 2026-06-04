# Installation on Windows

---

## Quick method — Automatic installer (recommended)

Download `speedy-setup-<version>.exe` from the [Releases page](https://github.com/elguala9/Speedy/releases) and run it.

The installer:
- Copies the 5 binaries to `%LOCALAPPDATA%\Programs\Speedy\` (no admin required)
- Adds the folder to the user PATH
- Does **not** start the daemon or register it for login — by default Speedy syncs via git hooks. Enable launch-at-login from the GUI (Dashboard → "Avvio al login") if you want live file-watching.

**Full uninstall:** Control Panel → Programs → *Speedy* → Uninstall.
The wizard will ask whether to also delete user data (registered workspaces, logs, configuration).

> To build the installer from source: `.\scripts\build-installer.ps1` (requires Inno Setup 6).

---

## Manual installation (advanced)

Follow these steps if you prefer to install manually without using the installer.

---

## Prerequisites

1. **Ollama** — [ollama.com](https://ollama.com) — must be running in the background.
2. Download the embedding model (once only):
   ```powershell
   ollama pull all-minilm
   ```

---

## The 5 binaries and their roles

| Binary | Role | Lifecycle |
|---|---|---|
| `speedy-ai-context.exe` | Worker — indexing, query, embedding, SQLite | Launched by the daemon, or standalone |
| `speedy-daemon.exe` | Global daemon — file watcher, IPC server | **Always running** (one per user) |
| `speedy-cli.exe` | Thin client — scriptable, for AI agents | Launched on-demand |
| `speedy-mcp.exe` | MCP server for AI agents (Claude, Cursor, …) | Launched by the MCP agent |
| `speedy-gui.exe` | Desktop GUI — workspace management and logs | Launched on-demand |

The first four (`speedy-ai-context`, `speedy-cli`, `speedy-mcp`, `speedy-gui`) launch and terminate in seconds. The daemon is the only one that must stay alive at all times.

---

## Recommended layout

```
C:\Program Files\Speedy\        ← or %LOCALAPPDATA%\Programs\Speedy\ without admin
├── speedy-ai-context.exe
├── speedy-daemon.exe
├── speedy-cli.exe
├── speedy-mcp.exe
└── speedy-gui.exe
```

All 5 in the same folder: this way `speedy-gui.exe` finds the daemon automatically via auto-detect (looks for `speedy-daemon.exe` next to itself).

---

## Step 1 — Copy the binaries

Download the `.exe` files from the [Releases page](https://github.com/elguala9/Speedy/releases) (or build them with `cargo build --release --workspace`) and copy them to the folder:

```powershell
$dir = 'C:\Program Files\Speedy'
New-Item -ItemType Directory -Force $dir

Copy-Item dist\speedy-ai-context.exe $dir
Copy-Item dist\speedy-daemon.exe $dir
Copy-Item dist\speedy-cli.exe    $dir
Copy-Item dist\speedy-mcp.exe    $dir
Copy-Item dist\speedy-gui.exe    $dir
```

> If you don't have admin rights use `$dir = "$env:LOCALAPPDATA\Programs\Speedy"`.

---

## Step 2 — Add the folder to PATH

Optional but recommended: allows typing `speedy-cli daemon status` from any shell and referencing `speedy-mcp` in MCP clients without an absolute path.

```powershell
$dir = 'C:\Program Files\Speedy'
[Environment]::SetEnvironmentVariable(
    'Path',
    [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + $dir,
    'User')
```

Open a new terminal afterwards.

---

## Step 3 — (Optional) Autostart the daemon at login

The daemon is **optional** — by default Speedy syncs via git hooks and needs no daemon. Set this up only if you want live file-watching (auto-reindex on save). The easiest way is the GUI (Dashboard → "Avvio al login"); to do it by hand, create a **shortcut in the user's Startup folder**.

Open the Startup folder: `Win + R` → type `shell:startup` → Enter.

Then create the shortcut via PowerShell:

```powershell
$startup = [Environment]::GetFolderPath('Startup')
$target  = 'C:\Program Files\Speedy\speedy-daemon.exe'
$ws      = New-Object -ComObject WScript.Shell
$lnk     = $ws.CreateShortcut("$startup\speedy-daemon.lnk")
$lnk.TargetPath = $target
$lnk.Save()
```

At the next login the daemon starts in the background without a window (`CREATE_NO_WINDOW` flag already active). To start it immediately without re-logging in:

```powershell
Start-Process 'C:\Program Files\Speedy\speedy-daemon.exe' -WindowStyle Hidden
```

---

## Step 4 — Verify

```powershell
speedy-cli daemon ping      # should respond: pong
speedy-cli daemon status    # JSON with pid, uptime, watcher_count
```

---

## Step 5 — First workspace

```powershell
# Register the project (the daemon starts a file watcher)
speedy-cli workspace add C:\path\to\project

# Index
speedy-cli index

# Search
speedy-cli query "how does authentication work?" -k 10
```

Alternatively open `speedy-gui.exe` → **Workspaces** tab → **Add**.

For each workspace you can enable/disable the two subsystems directly from the GUI (checkboxes **Speedy Indexer** and **Language Context** in the workspace row) or by editing the config file directly:

```toml
# <workspace>\.speedy\config.toml
[features]
speedy_indexer = true    # file indexer (speedy-ai-context)
language_context = true  # code intelligence (speedy-language-context)
```

The GUI reads and writes the same `config.toml`: changes are immediately visible in both directions.

> Quick CLI commands (`speedy enable speedy`, `speedy enable slc`, …) are planned but not yet implemented.

---

## Configure an AI agent (MCP)

Add to your MCP client config (e.g. `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "speedy": {
      "command": "C:\\Program Files\\Speedy\\speedy-mcp.exe",
      "args": [],
      "env": {
        "SPEEDY_BIN": "speedy-cli",
        "SPEEDY_DEFAULT_SOCKET": "speedy-daemon",
        "SPEEDY_MCP_TOP_K": "10"
      }
    }
  }
}
```

> If you added the folder to PATH you can use just `"command": "speedy-mcp"`.

---

## Removal

```powershell
# Remove the autostart shortcut
Remove-Item "$([Environment]::GetFolderPath('Startup'))\speedy-daemon.lnk" -ErrorAction SilentlyContinue

# Stop the daemon (if running)
speedy-cli daemon stop

# Delete the binaries
Remove-Item -Recurse -Force 'C:\Program Files\Speedy'
```

The workspace database (`.speedy/index.sqlite`) lives inside each project and is not touched by this procedure.
