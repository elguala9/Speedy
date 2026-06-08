# Commands

## `speedy-ai-context.exe`

### Subcommands

| Command | What it does |
|---|---|
| `index [<SUBDIR>]` | Index a directory (default `.`) |
| `query <QUERY> [-k <N>]` | Semantic search (default top-K 5) |
| `context` | Workspace summary |
| `sync` | Incremental FS → index sync (this worker only) |
| `reembed` | Drop all embeddings and re-index with the current model |
| `install-hooks [--path <PATH>] [--force]` | Install the Speedy git hooks (call speedy-cli on commit/checkout/merge/rebase) |
| `uninstall-hooks [--path <PATH>]` | Remove the Speedy-managed git hooks (foreign hooks left untouched) |
| `daemon` | Spawn the central daemon |
| `workspace list` | List registered workspaces |

### Flags

| Flag | What it does |
|---|---|
| `-p, --path <PATH>` | Project root (default cwd) |
| `--json` | JSON output |
| `--daemon-socket <NAME>` | Socket name (default `speedy-daemon`) |
| `-r, --read <PROMPT>` | Shortcut for `query` |
| `-m, --modify <CONTENT> --file <PATH>` | Write file and re-index |
| `-d, --daemons` | List workspaces tracked by the daemon |
| `-w, --workspaces` | List registered workspaces |
| `-h, --help` | Help (also for each subcommand) |
| `-V, --version` | Version |

---

## `speedy-cli.exe`

### Subcommands

| Command | What it does |
|---|---|
| `index [<SUBDIR>]` | Index via daemon |
| `query <QUERY> [-k <N>] [--all]` | Semantic search via daemon. `--all`: fan-out across all workspaces, aggregate top-K |
| `context` | Workspace summary via daemon |
| `sync` | **Incremental** sync across **all enabled contexts** (mtime+hash skip; prunes deletions) |
| `update <FILES…>` | **Per-file** incremental update across all enabled contexts (used by the `post-commit` hook) |
| `reindex [-p <PATH>]` | **Full** rebuild across all enabled contexts |
| `reembed` | Drop embeddings and re-index with the current model (via daemon) |
| `force [-p <PATH>]` | Same fan-out incremental sync, targeting an explicit path |
| `daemon status` | Daemon status (PID, uptime, ws/watchers) |
| `daemon list` | Active workspaces on the daemon |
| `daemon stop` | Stop the daemon |
| `daemon ping` | Ping → pong |
| `workspace list` | List registered workspaces |
| `workspace add <PATH>` | Register a workspace; **also installs the Speedy git hooks** if it's a git repo |
| `workspace remove <PATH>` | Unregister a workspace; **also removes the Speedy git hooks** |

### Flags

| Flag | What it does |
|---|---|
| `-p, --path <PATH>` | Project root (default cwd) |
| `--json` | JSON output |
| `--daemon-socket <NAME>` | Socket name (default `speedy-daemon`) |
| `-h, --help` | Help (also for each subcommand) |
| `-V, --version` | Version |

---

## `speedy-daemon.exe`

### Flags

| Flag | What it does |
|---|---|
| `--daemon-socket <NAME>` | Socket to listen on (default `speedy-daemon`) |
| `--daemon-dir <DIR>` | Override directory for `daemon.pid`/`workspaces.json` |
| `-h, --help` | Help |
| `-V, --version` | Version |

---

## `speedy-gui.exe`

No CLI flags. Launched without arguments, opens the egui window.

### Tabs

| Tab | What it does |
|---|---|
| Dashboard | Daemon status (pid/uptime/version), cumulative metrics, restart/reload/stop, system notification toggle on `error` |
| Workspaces | List + add via native file picker + Index/Sync/Open folder/Remove per workspace + "Prune orphans" button (IPC `prune-missing`) |
| Scan | Walk a root path looking for existing `.speedy/index.sqlite`, bulk-register selected ones |
| Logs | Live tail (`subscribe-log` IPC) or historical view of a `daemon.log.*` file, filters by level/substring/target/workspace, JSON/JSONL export |

The Dashboard also exposes a "Daemon executable" field (with `Browse…` /
`Apply` / `Reset to auto`) to force the path of `speedy-daemon` when
auto-detect next to the GUI binary is not enough — useful in dev / custom installs.

System tray icon (green = daemon alive, red = down) with menu
**Open Speedy / Restart daemon / Quit**.

Login autostart is not managed by the GUI: manually place `speedy-daemon.exe`
(or a shortcut to it) in the Windows Startup folder (or equivalent on
macOS/Linux). See README.

Persistent settings via `eframe::Storage`: selected tab, light/dark theme,
socket name, system notification toggle on `error`, daemon path override.

---

## `speedy-mcp.exe`

No CLI flags. Launched by the MCP client, communicates over stdio JSON-RPC.

### Exposed tools

| Tool | Arguments |
|---|---|
| `speedy_query` | `{ query: string, top_k?: number }` |
| `speedy_index` | `{ path?: string }` |
| `speedy_context` | `{}` |

## Environment variables

| Variable | Effect |
|---|---|
| `SPEEDY_NO_DAEMON=1` | Forces the standalone path: `speedy-cli` skips the daemon probe and drives the workers in-process. Set by every git hook. |
| `SPEEDY_SKIP_HOOKS=1` | Makes the Speedy git hooks exit immediately (no indexing on commit/checkout/merge/rebase). |
| `SPEEDY_FORCE=1` | Bypasses a worker's per-workspace opt-in gate (used by explicit `sync`/`reindex`/`update`). |
| `SPEEDY_MODEL` | Embedding model name (default `all-minilm`); run `reembed` after changing it. |
