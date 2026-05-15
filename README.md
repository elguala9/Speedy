# Speedy

Local Semantic File System — bridges your local filesystem with AI models.

Speedy indexes your codebase into a SQLite vector database, watches for file changes via a single background daemon, and exposes semantic search over your code through a CLI and an MCP server for AI agents (Claude Code, Cursor, opencode, Windsurf, …).

> For the full per-binary option reference see **[`commands.md`](./commands.md)**.
> For the end-to-end runtime flow see **[`flow.md`](./flow.md)**.

## Architecture at a glance

```
┌─────────────────────────────────────────────────────────────┐
│  AI Agent / User                                            │
│       │                                                     │
│       ├─────────────────────┐                               │
│       ▼                     ▼                               │
│  speedy-mcp.exe       speedy-gui.exe   (egui desktop app)   │
│  (MCP / stdio)              │                               │
│       │                     │                               │
│       ▼                     │                               │
│  speedy-cli.exe ◄───────────┘   (thin client, no heavy deps)│
│       │   local socket "speedy-daemon"                      │
│       ▼                                                     │
│  speedy-daemon.exe       ← ONE process, global per user     │
│   • IPC server                                              │
│   • N file watchers (one task per workspace)                │
│   • persistent memory: workspaces.json                      │
│       │                                                     │
│       │ spawn subprocess                                    │
│       ▼                                                     │
│  speedy.exe              ← the worker                       │
│   • indexing, query, embedding, SQLite, chunking            │
│   • can also run standalone (no daemon needed)              │
└─────────────────────────────────────────────────────────────┘
```

- **One daemon for everything.** Never one daemon per workspace. The same `speedy-daemon.exe` watches all your projects and survives across CLI invocations.
- **Persistent memory.** The daemon's workspace registry lives in `workspaces.json` under your config dir. On reboot, the daemon reads it back and restarts every watcher.
- **Worker is autonomous.** `speedy.exe` can run on its own — the daemon just orchestrates it for live updates.

## Download

Pre-built binaries are available on the [Releases page](https://github.com/elguala9/Speedy/releases).

| Binary              | Role                                      |
|---------------------|-------------------------------------------|
| `speedy.exe`        | Worker — indexing, query, embedding       |
| `speedy-daemon.exe` | Single global daemon + file watcher       |
| `speedy-cli.exe`    | Thin client (what AI agents / scripts call) |
| `speedy-mcp.exe`    | MCP server for AI coding agents           |
| `speedy-gui.exe`    | Desktop GUI (egui) — manage daemon, workspaces, live logs |

## Install

### Build the binaries

```bash
# 1. Install Rust: https://rustup.rs/
# 2. Install Ollama and pull an embedding model
ollama pull all-minilm:l6-v2

# 3. Build all 5 binaries in one shot
cargo build-all
# (alias for: cargo build --release -p speedy -p speedy-daemon \
#                            -p speedy-cli -p speedy-mcp -p speedy-gui)
```

Or run `scripts/build-release.ps1` / `scripts/build-release.sh` — they build
**and** copy every `.exe` into `dist/`.

### Recommended layout (Windows)

Speedy ships as **5 binaries with two distinct lifecycles**: a long-running
background daemon and four short-lived front-end tools. Install them
accordingly.

**1. The user-facing tools — in one folder, on `PATH`**

Pick a stable install dir, e.g. `C:\Program Files\Speedy\` (or
`%LOCALAPPDATA%\Programs\Speedy\` if you don't want admin rights), and copy
the four front-end binaries into it:

```
C:\Program Files\Speedy\
├── speedy.exe          ← worker (also usable standalone)
├── speedy-cli.exe      ← thin client for scripts / MCP
├── speedy-mcp.exe      ← MCP server for AI coding agents
└── speedy-gui.exe      ← desktop GUI
```

Adding this folder to your **`PATH`** is *recommended but not required*. With
it on `PATH` you can type `speedy query …` and `speedy-cli daemon status`
from any shell, and MCP clients can reference `speedy-mcp` without a full
path. To add it permanently (PowerShell, current user only):

```powershell
$dir = 'C:\Program Files\Speedy'
[Environment]::SetEnvironmentVariable(
    'Path',
    [Environment]::GetEnvironmentVariable('Path','User') + ';' + $dir,
    'User')
```

Open a fresh terminal afterwards.

**2. The daemon — in the Windows Startup folder**

`speedy-daemon.exe` is different: it's a single, global, always-on process,
so put it where Windows will launch it automatically at every login. Place
(or shortcut) the binary into the **Startup folder**:

```
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\
```

Quick way to open it: `Win + R` → `shell:startup` → Enter.

Drop a copy of `speedy-daemon.exe` there, or — cleaner — drop a **shortcut**
that points to the binary in your install dir:

```powershell
$startup = [Environment]::GetFolderPath('Startup')
$target  = 'C:\Program Files\Speedy\speedy-daemon.exe'
$ws      = New-Object -ComObject WScript.Shell
$lnk     = $ws.CreateShortcut("$startup\speedy-daemon.lnk")
$lnk.TargetPath = $target
$lnk.Save()
```

After the next login (or by running it once manually) the daemon is up; all
other commands talk to it transparently.

### First run

```bash
# Register the workspace once: the daemon adds a file watcher
# and keeps the index up to date in the background.
speedy-cli workspace add C:\path\to\project

# Index it (the daemon dispatches to speedy.exe under the hood)
speedy-cli index

# Query
speedy-cli query "how does authentication work?" -k 10

# Sanity checks
speedy-cli daemon ping        # → pong
speedy-cli daemon status      # JSON with pid, uptime, watcher_count, …
speedy-cli daemon list        # workspaces currently watched
```

Or just launch `speedy-gui.exe` and add workspaces from the **Workspaces**
tab — same operations, with live tail of the daemon logs.

### Standalone (no daemon)

If you don't want a background process at all, `speedy.exe` works on its own:

```bash
SPEEDY_NO_DAEMON=1 speedy index .
SPEEDY_NO_DAEMON=1 speedy query "find auth"
```

You lose live re-indexing on file changes — every query reflects only what
the last manual `index` / `sync` captured.

## Prerequisites

- [Rust](https://rustup.rs/) (edition 2021)
- [Ollama](https://ollama.ai/) running locally, with an embedding model pulled (default: `all-minilm:l6-v2`)

## CLI reference (summary)

The exhaustive option list is in [`commands.md`](./commands.md). Brief summary below.

### `speedy.exe` — the worker

Common subcommands (each honors `--json` and `-p, --path <PATH>`):

| Command                  | What it does                                              |
|--------------------------|-----------------------------------------------------------|
| `index [<subdir>]`       | Index a directory into the vector DB                      |
| `query <q> [-k <N>]`     | Semantic search (top-K, default 5)                        |
| `context`                | Project context summary                                   |
| `sync`                   | Incremental sync of filesystem changes                    |
| `daemon`                 | Spawn the central daemon                                  |
| `workspace list`         | List registered workspaces                                |

Top-level shortcut flags (alternative to subcommands): `-r/--read`, `-m/--modify --file`, `-d/--daemons`, `-w/--workspaces`. Full table in [`commands.md`](./commands.md).

### `speedy-cli.exe` — the thin client

Global flags: `-p/--path`, `--daemon-socket`, `--json`.

| Command                          | What it does                                                        |
|----------------------------------|---------------------------------------------------------------------|
| `index [<subdir>]`               | Send `exec ... index <subdir>` to the daemon                        |
| `query <q> [-k <N>]`             | Send `exec ... query <q> -k <N>`                                    |
| `context`                        | Send `exec ... context`                                             |
| `sync`                           | Send `exec ... sync`                                                |
| `force [-p <path>]`              | Send `sync <path>` directly (daemon-driven incremental sync)        |
| `daemon {status,list,stop,ping}` | Talk to the daemon directly (no `speedy.exe` involved)              |
| `workspace {list,add,remove}`    | Register/unregister workspaces (note: path is **positional** here)  |

### `speedy-daemon.exe` — the central daemon

Minimal CLI; all management is via the IPC protocol.

| Flag                       | Default          | Purpose                                                          |
|----------------------------|------------------|------------------------------------------------------------------|
| `--daemon-socket <NAME>`   | `speedy-daemon`  | Local socket name (Named Pipe on Windows, UDS elsewhere)        |
| `--daemon-dir <DIR>`       | platform config  | Override the dir holding `daemon.pid` and `workspaces.json`     |

### `speedy-gui.exe` — the desktop GUI

An `egui`-based desktop app that wraps the daemon's IPC. Launch with no
arguments. Four tabs:

| Tab          | What it does                                                                |
|--------------|-----------------------------------------------------------------------------|
| Dashboard    | Daemon status (pid/uptime/version), metrics, restart/reload/stop, "notify on error" toggle |
| Workspaces   | List + add (native file picker) + per-workspace Index/Sync/Open folder/Remove |
| Scan         | Walk a root path looking for existing `.speedy/index.sqlite` and bulk-register what it finds |
| Logs         | Live tail (`subscribe-log` IPC) or historical view of any `daemon.log.*` file, with level/substring/target/workspace filters and JSON/JSONL export |

A system tray icon (green = daemon alive, red = down) provides quick
**Open Speedy / Restart daemon / Quit** actions.

### `speedy-mcp.exe` — the MCP server

Communicates over stdio. Tools exposed:

| Tool             | Args                                       | Underlying call                                       |
|------------------|--------------------------------------------|-------------------------------------------------------|
| `speedy_query`   | `{ query: string, top_k?: number }`        | `$SPEEDY_BIN query <q> -k <top_k> --json`             |
| `speedy_index`   | `{ path?: string }`                        | `$SPEEDY_BIN index <path>`                            |
| `speedy_context` | `{}`                                       | `$SPEEDY_BIN context --json`                          |

Set `SPEEDY_BIN` to choose the underlying binary (default: `speedy-cli`).

#### Example MCP client config

Minimal:

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-mcp",
      "args": [],
      "env": { "SPEEDY_BIN": "speedy-cli" }
    }
  }
}
```

Full example for **Claude Desktop** (`claude_desktop_config.json`):

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

Typical agent flow:

1. User opens their repo and runs `speedy-cli workspace add .` (or just `speedy index .` once — `ensure_daemon` auto-registers).
2. The daemon spawns a watcher and keeps the SQLite index fresh on every file change.
3. The agent (Claude / Cursor / opencode / …) calls `speedy_query { "query": "where is auth handled?", "top_k": 10 }` — the MCP server invokes `speedy-cli query "where is auth handled?" -k 10 --json` and pipes the ranked chunks back to the agent.
4. Subsequent edits trigger a re-index in the background; the next query sees them.

## IPC protocol

The daemon listens on a **local socket** (Windows Named Pipe / Unix Domain Socket via the `interprocess` crate; default name `speedy-daemon`). Wire format: one line of UTF-8 per request, one line per response. Full reference in [`docs/ipc-protocol.md`](./docs/ipc-protocol.md).

| Request                                  | Response                                                        |
|------------------------------------------|-----------------------------------------------------------------|
| `ping`                                   | `pong`                                                          |
| `status`                                 | JSON `{pid, uptime_secs, workspace_count, watcher_count, version}` |
| `list` / `watch-count` / `daemon-pid`    | JSON / number                                                   |
| `is-workspace <path>`                    | `true` / `false`                                                |
| `add <path>` / `remove <path>`           | `ok` / `error: ...`                                             |
| `sync <path>`                            | `ok` / `error: ...`                                             |
| `reload`                                 | `ok: N workspaces reloaded`                                     |
| `exec <args>` (or `exec\t<cwd>\t<args>`) | stdout of `speedy.exe <args>` (with `SPEEDY_NO_DAEMON=1`)       |
| `stop`                                   | `ok` then graceful shutdown                                     |

## Configuration

Speedy reads, in priority order:

1. Environment variables
2. `speedy.toml` or `.speedy/config.toml` in the project root

### Environment variables

| Variable                  | Default                    | Purpose                                                                    |
|---------------------------|----------------------------|----------------------------------------------------------------------------|
| `SPEEDY_NO_DAEMON`        | unset                      | If set, the worker skips `ensure_daemon` (the daemon itself sets this when spawning `speedy.exe`) |
| `SPEEDY_DEFAULT_SOCKET`   | `speedy-daemon`            | Override the default IPC socket name                                       |
| `SPEEDY_DAEMON_DIR`       | platform config dir        | Override the dir for `daemon.pid` / `workspaces.json`                      |
| `SPEEDY_BIN`              | `speedy-cli`               | Binary that `speedy-mcp` invokes for tool calls                            |
| `SPEEDY_MODEL`            | `all-minilm:l6-v2`         | Ollama embedding model                                                     |
| `SPEEDY_OLLAMA_URL`       | `http://localhost:11434`   | Ollama server URL                                                          |
| `SPEEDY_PROVIDER`         | `ollama`                   | Embedding provider (`ollama` or `agent`)                                   |
| `SPEEDY_AGENT_COMMAND`    | *(empty)*                  | External command when `SPEEDY_PROVIDER=agent`                              |
| `SPEEDY_TOP_K`            | `5`                        | Default top-K for `query`                                                  |
| `RUST_LOG`                | *(empty)*                  | Tracing filter                                                             |

### Config file (`speedy.toml` / `.speedy/config.toml`)

```toml
model = "nomic-embed-text"
ollama_url = "http://localhost:11434"
provider_type = "ollama"
top_k = 10
max_chunk_size = 1000
chunk_overlap = 200
watch_delay_ms = 500
ignore_patterns = ["target/", ".git/", "node_modules/"]
```

### Embedding providers

- **`ollama`** (default) — calls Ollama's `/api/embeddings`. Requires Ollama running locally.
- **`agent`** — delegates embedding to `SPEEDY_AGENT_COMMAND`. The command receives the text as its first argument and must output a JSON array of floats on stdout.

## Ignore files

Speedy honors `.gitignore` automatically (via the `ignore` crate). Add a `.speedyignore` in the project root for Speedy-specific rules — same syntax as `.gitignore`.

```gitignore
# .speedyignore
build/
dist/
*.log
```

The daemon's watcher additionally hardcodes ignores for: `target/`, `.git/`, `node_modules/`, `.speedy/`, `.speedy-daemon/`, `.idea/`, `.vscode/`, `dist/`, `build/`, `__pycache__/`, `.cargo/`.

## On-disk layout — the daemon's "memory"

```
~/.config/speedy/                       (Windows: %APPDATA%\speedy)
├── workspaces.json     ← global registry of every workspace the user
│                         has added — the daemon's persistent memory
└── daemon.pid          ← PID of the running daemon (one only)

<workspace>/
├── .speedy/
│   ├── index.sqlite    ← vector store for THIS workspace
│   └── config.toml     ← optional per-workspace overrides
└── .speedyignore       ← optional, gitignore syntax
```

- Only the daemon writes to `workspaces.json`. CLI / MCP / scripts ask the daemon to `add` / `remove`; they never write the file directly.
- `daemon.pid` is used only to clean up a stale instance after a reboot.
- Each workspace has its own `.speedy/index.sqlite` — the daemon centralizes orchestration, not data.

## Project structure

```
Cargo.toml (workspace)
└── packages/
    ├── speedy-core/         # shared lib: DaemonClient, workspace registry,
    │                          config, local-socket helpers, embedding type
    ├── speedy/              # bin = speedy.exe (worker)
    ├── speedy-daemon/       # bin = speedy-daemon.exe (central daemon)
    ├── speedy-cli/          # bin = speedy-cli.exe (thin client)
    ├── speedy-mcp/          # bin = speedy-mcp.exe (MCP server)
    └── testexe/             # internal test fixture
```

## Development

```bash
cargo test --workspace        # run all tests
cargo build --workspace       # debug build
cargo clippy --workspace      # lint
```

Some daemon tests touch process-wide state (cwd, `SPEEDY_DAEMON_DIR`) and serialize on a shared mutex — they may run slower on a single thread.

## Troubleshooting

**`speedy-cli` says "Cannot connect to daemon. Is it running?"**

Either the daemon was never started or its named pipe is orphaned. Start a fresh one:

```bash
speedy daemon                 # spawns the central daemon detached
speedy-cli daemon ping        # should print `pong`
```

**`daemon-pid` file is stale (PID points to a dead process)**

The daemon now takes an advisory lock on `daemon.pid` at startup, so a stale PID file alone won't block a new daemon — `kill_existing_daemon` clears it. If you see "address in use" anyway, kill leftover processes manually:

```powershell
Get-Process speedy-daemon -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item "$env:APPDATA\speedy\daemon.pid" -ErrorAction SilentlyContinue
```

```bash
pkill -f speedy-daemon || true
rm -f ~/.config/speedy/daemon.pid
```

**`speedy-cli daemon status` hangs**

Almost always a named-pipe owned by a frozen daemon. `Stop-Process` / `pkill` the daemon and re-spawn.

**Watcher is not picking up file changes**

Check that the workspace is registered (`speedy-cli daemon list`). If absent, add it: `speedy-cli workspace add <PATH>`. Verify the path is not under an ignored prefix (`target/`, `.git/`, `.speedy/`, `node_modules/`, etc.).

**`speedy-mcp` doesn't see my repo**

The MCP server runs `SPEEDY_BIN` against whatever cwd Claude Desktop sets when launching it — usually not your project. Pass `-p <PATH>` from the agent prompt or `speedy-cli workspace add <PATH>` once to register the repo. After that, `speedy_query` works from anywhere because the daemon already monitors it.

## License

MIT
