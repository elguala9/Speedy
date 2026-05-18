# Speedy — Manual Installation

> **Already installed via `speedy-setup-<version>.exe`?**
> You're all set — the installer handles everything automatically.
> This document is extra reference only: it explains what the installer
> does under the hood and how to replicate it manually on any platform.

---

This archive contains pre-built binaries for Speedy.
Follow the steps below for your platform.

---

## Contents of this archive

| File | Description |
|------|-------------|
| `speedy-daemon` | Background daemon — watches workspaces, serves requests |
| `speedy-cli` | Thin CLI client — what you call from the terminal |
| `speedy-ai-context-mcp` | MCP server for semantic search |
| `speedy-language-context-mcp` | MCP server for code graph analysis |
| `speedy-ai-context` | Indexing/query worker (called by the daemon) |
| `speedy-gui` | Desktop GUI |
| `README.txt` | Usage reference |

On Windows every file has an `.exe` extension.

---

## Linux / macOS

### 1. Extract the archive

```bash
tar xzf speedy-x86_64-unknown-linux-gnu.tar.gz
```

### 2. Move binaries to a directory on your PATH

```bash
sudo mv speedy-* /usr/local/bin/
# or, without sudo, into your home:
mkdir -p ~/.local/bin
mv speedy-* ~/.local/bin/
# make sure ~/.local/bin is on PATH:
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

### 3. Make them executable (Linux/macOS only)

```bash
chmod +x /usr/local/bin/speedy-*
```

### 4. Start the daemon

```bash
speedy-daemon &
```

Verify it is running:

```bash
speedy-cli daemon status
```

### 5. Autostart the daemon at login (systemd — Linux)

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/speedy-daemon.service << 'EOF'
[Unit]
Description=Speedy semantic search daemon
After=network.target

[Service]
ExecStart=%h/.local/bin/speedy-daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now speedy-daemon
```

### 5b. Autostart at login (launchd — macOS)

```bash
INSTALL_DIR="$HOME/.local/bin"
cat > ~/Library/LaunchAgents/com.speedy.daemon.plist << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>             <string>com.speedy.daemon</string>
  <key>ProgramArguments</key>  <array><string>$INSTALL_DIR/speedy-daemon</string></array>
  <key>RunAtLoad</key>         <true/>
  <key>KeepAlive</key>         <true/>
</dict>
</plist>
EOF

launchctl load ~/Library/LaunchAgents/com.speedy.daemon.plist
```

---

## Windows (manual)

### 1. Extract the archive

Right-click the `.tar.gz` file → Extract All, or use 7-Zip / WSL:

```powershell
tar xzf speedy-x86_64-pc-windows-msvc.tar.gz
```

### 2. Move binaries to a permanent location

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Programs\Speedy"
Move-Item speedy-*.exe "$env:LOCALAPPDATA\Programs\Speedy\"
```

### 3. Add to PATH (current user, no admin required)

```powershell
$installDir = "$env:LOCALAPPDATA\Programs\Speedy"
$current = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($current -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("PATH", "$current;$installDir", "User")
}
```

Restart your terminal for PATH to take effect.

### 4. Start the daemon

```powershell
Start-Process speedy-daemon.exe -WindowStyle Hidden
```

Verify:

```powershell
speedy-cli daemon status
```

### 5. Autostart the daemon at login (Task Scheduler)

```powershell
$installDir = "$env:LOCALAPPDATA\Programs\Speedy"
$action  = New-ScheduledTaskAction -Execute "$installDir\speedy-daemon.exe"
$trigger = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit 0
Register-ScheduledTask -TaskName "Speedy Daemon" `
    -Action $action -Trigger $trigger -Settings $settings `
    -RunLevel Limited -Force
```

---

## First use

### Register a workspace and index it

```bash
speedy-cli workspace add /path/to/your/project
cd /path/to/your/project
speedy-cli index
```

### Search

```bash
speedy-cli query "authentication middleware"
speedy-cli query "database connection pool" -k 10
```

### Connect an AI agent (MCP)

Add to your agent's config (Claude Code, Cursor, Windsurf, opencode…):

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-ai-context-mcp",
      "args": []
    },
    "speedy-lang": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "/path/to/your/project"]
    }
  }
}
```

If the binaries are not on PATH, use the full path:

```json
{
  "mcpServers": {
    "speedy": {
      "command": "/usr/local/bin/speedy-ai-context-mcp",
      "args": [],
      "env": { "SPEEDY_BIN": "/usr/local/bin/speedy-cli" }
    }
  }
}
```

---

## Uninstall

### Linux / macOS

```bash
# Stop and disable the daemon
systemctl --user disable --now speedy-daemon   # Linux systemd
# or: launchctl unload ~/Library/LaunchAgents/com.speedy.daemon.plist

# Remove binaries
rm /usr/local/bin/speedy-*   # or ~/.local/bin/speedy-*

# Optional: remove data
rm -rf ~/.config/speedy ~/.speedy
```

### Windows

```powershell
# Stop the daemon
speedy-cli daemon stop

# Remove autostart task
Unregister-ScheduledTask -TaskName "Speedy Daemon" -Confirm:$false

# Remove binaries
Remove-Item "$env:LOCALAPPDATA\Programs\Speedy" -Recurse -Force

# Remove from PATH
$path = [Environment]::GetEnvironmentVariable("PATH", "User")
$clean = ($path -split ";") -notlike "*Programs\Speedy*" -join ";"
[Environment]::SetEnvironmentVariable("PATH", $clean, "User")

# Optional: remove data
Remove-Item "$env:APPDATA\speedy" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.speedy" -Recurse -Force -ErrorAction SilentlyContinue
```

Project indexes (`.speedy/` folders inside your projects) are never deleted automatically.

---

## More information

- Full usage reference: see `README.txt` in this archive
- GitHub: <https://github.com/elguala9/Speedy>
- Issues: <https://github.com/elguala9/Speedy/issues>
