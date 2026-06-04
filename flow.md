# Speedy Flow — how it should run

> Project comprehension verification document. It describes, step by step, **what does what** and **who calls whom** in the various scenarios. If something here is wrong, it's a point where I misunderstood the project.

## ⚠️ Current default: no daemon, everything opt-in

> **Update (branch `feature/no-daemon`).** By default Speedy **does not use
> the daemon**: synchronization happens **only via git hooks** (commit /
> checkout / merge / rebase), which invoke the worker in standalone mode
> (`SPEEDY_NO_DAEMON=1`). The daemon remains available but is **never
> started automatically** — it must be launched explicitly (`speedy daemon` or from the
> "Start daemon" button in the GUI).
>
> Moreover **all contexts are opt-in**: `speedy_indexer`, `language_context`
> and `text_context` are **disabled by default** for a new workspace. Their
> respective workers are no-ops until the feature is enabled
> (`speedy enable speedy` / `speedy enable slc`). See `[features]` in
> `.speedy/config.toml`.
>
> The following section describes the **historical daemon model**, valid only
> when the daemon is started manually.

## Core principle (daemon model, now opt-in)

**A single daemon. Global. For everything.**

- There is **only one** `speedy-daemon.exe` running per user, never one per
  workspace. All of the user's workspaces are managed by that single
  process, each with its own internal task watcher.
- The daemon has a **fixed persistent memory** on disk
  (`~/.config/speedy/workspaces.json` on Linux/macOS,
  `%APPDATA%\speedy\workspaces.json` on Windows) where it keeps the list of
  registered workspaces: canonical path + any metadata for each.
- On startup (even after a PC reboot) the daemon **re-reads this
  memory** and rebuilds the state in RAM: for each path that still exists it
  restarts a watcher; orphans are purged.
- `workspaces.json` is the **source of truth**. The daemon's in-RAM state is
  a mirror. CLI / MCP / external scripts never write directly
  to `workspaces.json` — they always go through the daemon (`add` / `remove`),
  which is the only one authorized to mutate it.

---

## 1. The actors (5 .exe + 1 lib)

```
speedy-core (lib)        ← lightweight shared library
                           DaemonClient, workspace registry, config,
                           local-socket helpers, shared serde types
                           (DaemonStatus, Metrics, WorkspaceStatus,
                            ScanResult, LogLine), embedding type

speedy-ai-context.exe (worker)      ← ALL the heavy logic inline
                           indexer, query, embedding, SQLite, chunking,
                           hashing, ignore, file filter, real watcher
                           can run standalone, or be spawned
                           as a subprocess by the daemon

speedy-daemon.exe        ← A SINGLE process, global per user
                           manages ALL workspaces together
                           (never a daemon-per-workspace)
                           IPC server on local socket "speedy-daemon"
                           N file-watchers (one per workspace) INSIDE
                           the same process, as tokio tasks
                           Does NOT do embedding/indexing: delegates to speedy-ai-context.exe
                           via subprocess
                           deploy target: Windows Startup folder
                           (starts at user login)

speedy-cli.exe           ← CLI front-end (depends on speedy-core)
                           daemon-if-alive → routes requests to the daemon
                           daemon-absent → orchestrates the three context
                           workers in-process (speedy_core::contexts): reindex fans out to
                           ai-context + language-context + text-context, each
                           gated by its feature. The daemon is NEVER
                           auto-started.

speedy-ai-context-mcp.exe           ← MCP server (JSON-RPC over stdio) for ai-context (semantic search)
                           uses SPEEDY_BIN (default: speedy-cli) to
                           run the tools → daemon → speedy-ai-context.exe

speedy-language-context-mcp.exe ← MCP server (JSON-RPC over stdio) for code intelligence
                           operates directly on GraphStore SQLite, without daemon

speedy-gui.exe           ← desktop GUI (egui + eframe) for manual management
                           uses speedy-core's DaemonClient directly
                           via a tokio runtime in the background, does NOT go
                           through speedy-cli. 4 tabs: Dashboard / Workspaces /
                           Scan / Logs. System tray icon.
```

### Dependencies between crates

| Binary           | Depends on                                       |
|------------------|--------------------------------------------------|
| `speedy-ai-context` | `speedy-core` + all the heavy logic           |
| `speedy-daemon`  | `speedy-core` + all the heavy logic              |
| `speedy-cli`     | only `speedy-core` (DaemonClient + local_sock)   |
| `speedy-ai-context-mcp`     | only `speedy-core` (calls `SPEEDY_BIN`)         |
| `speedy-language-context-mcp` | `speedy-language-context` lib (GraphStore, mcp) |
| `speedy-gui`     | only `speedy-core` (DaemonClient + types) + egui |

---

## 2. IPC — protocol

- **Transport**: local socket via the `interprocess` crate.
  - Windows → Named Pipe `\\.\pipe\speedy-daemon`
  - Unix    → Unix Domain Socket `speedy-daemon` (generic namespace)
- **Default name**: `speedy-daemon`. Override with `--daemon-socket`.
- **Wire**: one request per connection, line-based UTF-8.
  - request:  `<cmd>[ args...]\n`
  - response: `<line>\n`
  - the server closes the connection after the response.
- **`exec` with paths containing spaces** → tab-separated form:
  ```
  exec\t<cwd>\t<arg1>\t<arg2>...
  ```
  `<cwd>` may be empty. The whitespace form `exec <args>` is still accepted for legacy.

### Commands

| Command                  | Response                                                   | Dispatch on the daemon side                     |
|--------------------------|------------------------------------------------------------|-------------------------------------------------|
| `ping`                   | `pong`                                                     | inline                                          |
| `status`                 | JSON `{pid, uptime_secs, workspace_count, watcher_count, version}` | inline                                  |
| `list`                   | JSON `["/path/1", "/path/2"]`                              | inline (from the watcher map)                   |
| `watch-count`            | `N`                                                        | inline                                          |
| `daemon-pid`             | `N`                                                        | inline                                          |
| `is-workspace <path>`    | `true` / `false`                                           | inline                                          |
| `add <path>`             | `ok` / `error: ...`                                        | registers in `workspaces.json` + spawns watcher |
| `remove <path>`          | `ok` / `error: ...`                                        | abort watcher + deregister                      |
| `sync <path>`            | `ok` / `error: ...`                                        | spawns `speedy-ai-context.exe -p <path> sync` (incremental) |
| `reload`                 | `ok: N workspaces reloaded`                                | re-reads workspaces.json + sync watcher         |
| `exec <args>`            | stdout of `speedy-ai-context.exe`                                     | spawns `speedy-ai-context.exe <args>` with `SPEEDY_NO_DAEMON=1` |
| `stop`                   | `ok` (then graceful shutdown)                              | abort all watchers, exits the accept loop       |
| any other                | `error: unknown command: <cmd>`                            | —                                               |

Daemon operational notes:
- `accept()` has a 1s timeout → it can check the `running` flag and exit cleanly within one tick after `stop`.
- `exec` sets `SPEEDY_NO_DAEMON=1` in the child's env → the worker never re-enters the daemon (no fork-bomb).
- On startup, entries in `workspaces.json` with a non-existent path are purged.

> Additional commands supporting the GUI (`metrics`, `scan`, `reindex`,
> `workspace-status`, `tail-log`, `subscribe-log`, `query-all`) are
> documented in detail in [`docs/ipc-protocol.md`](./docs/ipc-protocol.md).
> All one-shot except `subscribe-log`, which is long-lived: the daemon sends
> `ok\n` as a handshake and then one JSON `LogLine` per event until the
> client closes the connection.

---

## 3. "First command after PC boot" flow

```
PC restarts
  ~/.config/speedy/workspaces.json  → intact on disk
  ~/.config/speedy/daemon.pid       → stale
  named pipe "speedy-daemon"        → does not exist

$ speedy-cli query "auth flow"
  │
  ├─ DaemonClient::is_alive()
  │    ├─ LocalStream::connect("speedy-daemon")
  │    ├─ write "ping\n" + shutdown
  │    ├─ read_line with 2s timeout
  │    └─ accept only if response == "pong"   ← avoids half-open pipe
  │
  │   → connect fail → false
  │
  ├─ ensure_daemon()
  │    ├─ kill_existing_daemon()  ← removes stale daemon.pid,
  │    │                            taskkill stale PID if needed
  │    ├─ spawn speedy-daemon.exe  (CREATE_NO_WINDOW on Windows,
  │    │                            stdout/stderr to null)
  │    └─ waits for is_alive() to become true (poll with timeout)
  │
  ├─ daemon.start()
  │    ├─ writes daemon.pid
  │    ├─ reads workspaces.json
  │    ├─ for each existing ws → spawns watcher (tokio task)
  │    └─ Listener::bind("speedy-daemon"), loop accept()
  │
  ├─ DaemonClient::is_workspace(CWD)?  → false
  ├─ DaemonClient::add_workspace(CWD)
  │    ├─ daemon receives "add <canonical>"
  │    ├─ workspace::add() on workspaces.json
  │    ├─ spawns watcher
  │    └─ (optional) initial sync_all via speedy-ai-context.exe sync
  │
  └─ DaemonClient::cmd("exec\t<CWD>\tquery\tauth flow")
       ├─ daemon spawns: speedy-ai-context.exe -p <CWD> query "auth flow"
       │                 with SPEEDY_NO_DAEMON=1
       ├─ speedy-ai-context.exe runs the query on the SQLite DB
       ├─ stdout returns to the daemon
       └─ daemon forwards it to the cli → cli prints it
```

---

## 4. "File saved from the editor" flow

```
User saves src/lib.rs
  │
  ├─ notify (in the workspace's watcher) generates an event
  │
  ├─ daemon: debounce + ignore filter (.gitignore + .speedyignore)
  │
  ├─ daemon computes the file's SHA-256 hash
  │    ├─ hash same as before?  → skip
  │    └─ hash different?       → continue
  │
  ├─ PID-check anti-loop:
  │    ├─ was the file touched by a PID present in active_pids?
  │    └─ (i.e.: one of our writes via speedy-ai-context.exe?)  → skip
  │
  └─ daemon spawns: speedy-ai-context.exe -p <ws> index ./src/lib.rs
       (SPEEDY_NO_DAEMON=1)
       │
       ├─ inserts the PID into active_pids
       ├─ waits for the child to finish (in a tokio task)
       └─ removes the PID from active_pids
```

### Safety: self-write

```
speedy-ai-context.exe writes to the DB (.speedy/index.sqlite)
  → notify detects the changes to the DB file
  → but the ignore-rules contain ".speedy/"  → skip

speedy-ai-context.exe does not write to the user's sources → no loop possible
```

The PID-check serves as a second defensive layer, in case one day the worker should rewrite some file.

---

## 5. "AI Agent via MCP" flow

Two independent MCP servers, registrable separately in the agents' configs.

### 5a. `speedy-ai-context-mcp.exe` — semantic search (ai-context)

```
Claude / other agent
  │  (stdio JSON-RPC)
  ▼
speedy-ai-context-mcp.exe
  │  for each tool call invokes: SPEEDY_BIN <args>
  │  (default SPEEDY_BIN = speedy-cli.exe)
  ▼
speedy-cli.exe
  │  ensure_daemon() → local socket
  ▼
speedy-daemon.exe
  │  exec <args>  → subprocess
  ▼
speedy-ai-context.exe
  │  query / index / context / sync on SQLite + Ollama
  ▼
stdout bubbles up to the agent as MCP result
```

`SPEEDY_BIN` allows pointing to `speedy-ai-context.exe` directly (bypassing the daemon) for batch / test scenarios.

### 5b. `speedy-language-context-mcp.exe` — code intelligence

```
Claude / other agent
  │  (stdio JSON-RPC)
  ▼
speedy-language-context-mcp.exe
  │  operates directly on GraphStore SQLite
  │  (no daemon, no IPC)
  ▼
.speedy/graph.db  (SQLite local to the workspace)
  │  index_status / get_skeleton / run_pipeline / search_observations / save_observation
  ▼
MCP result bubbles up to the agent
```

Example configuration (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "speedy-ai": {
      "command": "speedy-ai-context-mcp",
      "args": []
    },
    "speedy-lang": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "/path/to/project"]
    }
  }
}
```

---

## 5b. "Desktop GUI (`speedy-gui.exe`)" flow

> **Update (no-daemon).** The GUI must work **even without a daemon**.
> For this reason the **operations** (sync, reindex, workspace add/remove) no longer
> go directly through `DaemonClient`: the GUI spawns **`speedy-cli`**, and it is the cli
> that decides — daemon-if-alive, otherwise in-process orchestration of the three
> context workers (`speedy_core::contexts`). The GUI continues to use
> `DaemonClient` **only** for live monitoring (status, metrics,
> log-stream), which exists only with the daemon: when the daemon is down the GUI enters
> "standalone mode" (flag `DaemonState.standalone`), hides the
> metrics and disables Scan, but remains fully operational.

For live monitoring the GUI talks to the daemon directly with
`speedy-core::DaemonClient`; for actions it goes through `speedy-cli`
(`GUI → speedy-cli → (daemon | worker)`).

```
User launches speedy-gui.exe
  │
  ├─ main thread: TrayHandle::try_new() (Windows/macOS want it here)
  │   └─ eframe::run_native → SpeedyApp::new
  │        ├─ DaemonBridge::new
  │        │   ├─ tokio::runtime::Runtime (multi-thread, 2 workers)
  │        │   └─ Arc<Mutex<DaemonState>>  ← shared snapshot
  │        └─ Loads settings from eframe::Storage (tab, theme, socket)
  │
  ├─ On every frame (≤500ms, ctx.request_repaint_after):
  │   ├─ App::update clones DaemonState (moderate Vec/HashMap: cheap)
  │   ├─ The views (Dashboard / Workspaces / Scan / Logs) read from the snapshot
  │   └─ No Mutex held during drawing
  │
  └─ User clicks "Add workspace":
       │
       ├─ rfd::FileDialog::pick_folder (native file picker)
       │
       ├─ DaemonBridge::add_workspace(path)
       │   ├─ inc_busy()  (shows spinner in topbar)
       │   ├─ runtime.spawn:
       │   │    ├─ DaemonClient::add_workspace(path)  →  IPC "add <canonical>"
       │   │    └─ writes the result into DaemonState.last_op_result
       │   └─ returns IMMEDIATELY (UI does not block)
       │
       └─ The next frame reads the snapshot:
            ├─ if ok → green toast + refresh workspace list
            └─ if err → red toast with the daemon's message
```

### Log streaming ("Logs" tab)

```
LogStreamHandle::start
  ├─ tokio task: DaemonClient::subscribe_log
  │    ├─ opens the pipe, sends "subscribe-log\n", reads "ok\n"
  │    └─ then reads one JSON LogLine per line → mpsc::UnboundedSender
  │
  ├─ ring buffer cap 5000 in the main thread (drain the receiver in update())
  │
  └─ If the pipe dies (daemon restarted) → automatic reconnection every 2s
```

Filters (levels, substring, target, workspace) operate on the in-memory buffer, no new IPC.

### Key differences vs MCP

- **Operations via `speedy-cli`**: mutating actions spawn `speedy-cli` (which routes daemon-or-standalone). Live monitoring stays in-process via `DaemonClient`.
- **Shared state**: the GUI sees metrics + status + workspace status aggregated into a `DaemonState`, and updates them asynchronously.
- **Autostart**: handled at the OS level (Startup folder on Windows, equivalents on macOS/Linux). The GUI does not write to the registry or to LaunchAgents — the user places `speedy-daemon.exe` (or a shortcut to it) in the Startup folder.
- **Tray + notifications**: `tray-icon` for quick-actions (Open / Restart / Quit), `notify-rust` for system popups on `error` levels of the log stream (opt-in toggle).

### When the daemon is down

The GUI detects the `ping` failure and shows a "Start daemon" banner; the click calls `spawn_daemon_process` (same logic as `ensure_daemon` on the cli side: looks for `speedy-daemon{EXE_SUFFIX}` next to the GUI binary, spawns detached, polls `is_alive` with backoff up to 10s).

---

## 6. "speedy-ai-context.exe standalone, no daemon" flow

```
$ speedy-ai-context index .
  │
  ├─ should_skip_daemon_check()?  → yes
  │    (specific subcommands like index/query/context/sync from the direct CLI,
  │     or env SPEEDY_NO_DAEMON=1, or flag --no-daemon)
  │
  └─ runs everything in-process
       ├─ loads Config (env + speedy.toml / .speedy/config.toml)
       ├─ opens SQLite in .speedy/index.sqlite
       ├─ EmbeddingProvider (Ollama or agent)
       ├─ scan + ignore + chunking + embedding + insert
       └─ terminates
```

`speedy-ai-context.exe` is completely self-sufficient. The daemon is needed **only** for:
1. Continuous monitoring (auto-reindex on save)
2. Pre-flight check (index always up to date before a query)
3. API server for AI / MCP

---

## 7. CLI commands — who handles them

| Command                         | `speedy-ai-context.exe`           | `speedy-cli.exe`                   |
|---------------------------------|------------------------|------------------------------------|
| `index [<subdir>]`              | runs inline            | daemon alive → exec; absent → spawns `speedy-ai-context index` |
| `query <q>`                     | runs inline            | daemon alive → exec; absent → spawns `speedy-ai-context query` |
| `context`                       | runs inline            | daemon alive → exec; absent → spawns `speedy-ai-context context` |
| `sync`                          | runs inline            | daemon alive → exec; absent → spawns `speedy-ai-context sync`  |
| `reembed`                       | runs inline            | daemon alive → exec; absent → spawns `speedy-ai-context reembed` |
| `reindex [-p <path>]`           | n/a                    | daemon alive → `reindex` IPC; absent → `contexts::reindex_workspace` (fan-out over the 3) |
| `force [-p <path>]`             | n/a (removed)          | daemon alive → sync; absent → spawns `speedy-ai-context sync` |
| `daemon status/ping/stop/list`  | n/a                    | requires daemon alive (error if down) |
| `daemon` (no action)            | starts the central daemon | n/a                              |
| `workspace add/remove`          | n/a (worker: only `list`) | daemon alive → IPC; absent → `speedy_core::workspace` in-process |
| `workspace list`                | n/a (worker: only `list`) | `speedy_core::workspace::list()` in-process |

---

## 8. Files on disk — the daemon's "fixed memory"

```
~/.config/speedy/                      (Windows: %APPDATA%\speedy)
├── workspaces.json     ← PERSISTENT MEMORY of the global daemon:
│                         list of ALL the user's workspaces
│                         [{ "path": "C:/a/proj1", ... },
│                          { "path": "C:/b/proj2", ... }, ...]
└── daemon.pid          ← PID of the current daemon (only one)

<workspace>/
├── .speedy/
│   ├── index.sqlite    ← vector store of THIS workspace
│   └── config.toml     ← optional, per-workspace config override
└── .speedyignore       ← optional, gitignore format
```

- **`workspaces.json` is the daemon's memory**: global, shared across
  all workspaces, survives restarts. The daemon reads it on startup,
  updates it on every `add`/`remove`, uses it to recreate the watchers after
  a boot.
- **A single `workspaces.json`** per user — not one per project.
- **`daemon.pid`** is used only for cleanup of a dead instance at the next
  boot (the new daemon `taskkill`s the stale PID if it exists).
- **`.speedy/index.sqlite`** instead lives **inside** the individual workspace:
  each project has its own local vector DB. The daemon does not centralize
  the indexed data — it only centralizes the orchestration.
  The DB uses the **sqlite-vec** extension (`vec0` virtual table) for ANN
  cosine similarity search; embeddings are no longer stored as BLOBs
  in `chunks` but in a separate table `vec_chunks`.
- **Concurrency on `workspaces.json`** is still **not** protected by a cross-process
  file-lock (TODO). In any case only the daemon writes to it, so in
  practice the problem only manifests if two daemons start together —
  and that is already excluded by `kill_existing_daemon()` + `is_alive()` check.

---

## 9. Invariants the system must respect

1. **Never two daemons alive simultaneously.** `kill_existing_daemon()` is called both by the cli (before spawning) and by the daemon itself on startup. If the pipe already exists with a live listener that replies `pong`, the spawn is skipped.
2. **`speedy-ai-context.exe` spawned by the daemon always has `SPEEDY_NO_DAEMON=1`** → no recursion.
3. **Watcher and indexer do not write to the user's sources.** Only in `.speedy/`, which is ignored by the watcher via ignore-rules.
4. **`add` is idempotent.** Adding the same workspace twice does not create two watchers.
5. **`remove` of a non-existent workspace is not a fatal error**, it replies `ok` (or `error: ...` but the cli treats it as a no-op).
6. **On boot, entries in `workspaces.json` with a non-existent path are purged** before starting the watchers.
7. **`is_alive()` does not trust connect alone** → it sends `ping` and expects `pong`. A half-open named pipe is not mistaken for a live daemon.
8. **There is no more port-fallback**: with the local socket it's not needed, the name is uniquely resolvable per user/session. (The old TCP fallback 42137→42138 is obsolete.)

---

## 10. What changes relative to DAEMON-GUARD.md / ARCHITETTURA.md (historical)

- **Transport**: TCP `127.0.0.1:42137` → **local socket** (`interprocess`). The old documents talk about TCP; the current code (`daemon_client.rs`, `local_sock.rs`) uses a local socket. The API is the same, only the connector changes.
- **No firewall prompt on Windows** (that was the issue in `docs/windows-firewall-tcp.md`, now removed).
- **No port fallback** for the same reason.

---

## 11. Points where I might have misunderstood — to verify

- **PID-tracking on the watcher side**: the PID-set is used for `taskkill` at shutdown (`packages/speedy-daemon/src/main.rs`, field `CentralDaemon.active_pids`). **Decision 2026-05-14**: it is kept as *defense-in-depth*. The main protection against self-write remains the ignore of `.speedy/`, but `active_pids` allows a deterministic shutdown (zero orphan indexers) even if tomorrow the worker should start writing state files outside `.speedy/`. The cost is minimal (one `HashSet<u32>` per process).
- **Initial sync on `add` — solved 2026-05-15**: `handle_add` now fire-and-forgets `handle_sync` only for new workspaces (already existing on disk but not yet managed). The client's awaiter returns `ok` immediately, the spawn of `speedy-ai-context.exe sync` runs in the background with `SPEEDY_NO_DAEMON=1`. Override for tests: `SPEEDY_SKIP_INITIAL_SYNC=1`.

---

## 12. Auto-reload and periodic prune (added 2026-05-15)

The daemon maintains consistency with the source of truth (`workspaces.json`) in two independent ways:

1. **File watcher on `workspaces.json`**: `spawn_workspaces_json_watcher` watches the `daemon_dir` with `notify_debouncer_mini` (1s debounce). Any modification to the file (even from external tools that don't go through the daemon) triggers `reload_from_disk`, which reconciles in-memory ↔ disk. When it is the daemon itself that writes `workspaces.json`, the notify event makes it re-enter the reload — but `reload_from_disk` is a no-op if the `HashSet<String>` of disk-paths and in-memory-paths are equal.
2. **Periodic tick (`PRUNE_EVERY_N_TICKS = 10`, ≈5 min)**: `prune_and_reconcile` removes the watchers whose paths no longer exist on disk and then calls `workspace::prune_missing` to also align the registry file. It catches the case "I deleted the folder while the daemon was active".

---

## 13. Cross-workspace query (added 2026-05-15, protocol v2)

IPC command `query-all\t<top_k>\t<query>` → returns a JSON-array of aggregated hits. The daemon fans out in parallel (`tokio::spawn` for each registered workspace) running `speedy-ai-context.exe -p <ws> query <q> -k <K> --json` with `SPEEDY_NO_DAEMON=1`, deserializes each response into `Vec<serde_json::Value>`, adds the `workspace` field to each hit, merges everything, sorts by descending score and cuts at top_k.

User CLI: `speedy-cli query --all <q>` (or directly via `DaemonClient::query_all`).

Operational notes:
- The fan-out does not share the `workspaces.json` file lock (per-workspace `vectors.db` are independent).
- If a workspace fails (Ollama down, corrupted DB), it returns an empty array and the others proceed.
- `protocol_version` raised to 2; older clients that go via `cmd("query-all …")` receive `error: unknown command`.

---

## 14. Explicit `prune-missing` (added 2026-05-15)

In addition to the periodic prune of §12, there is now an explicit IPC command
`prune-missing` (one-shot) that does the same cleanup *on demand*:

- Daemon side: stops the watchers for paths that no longer exist, calls
  `workspace::prune_missing` and returns `{"removed": N, "paths": [...]}`.
- Client side: `DaemonClient::prune_missing() -> Result<Vec<String>>`.
- UI: "🧹 Clean orphans" button in the GUI's Workspaces tab.
  It differs from the per-row `Remove` because it doesn't require knowing the
  path: it cleans up everything that no longer exists without confirming one by one.

`protocol_version` remains 2 — it's a new command, not an incompatibility.

---

## 15. GUI: daemon-exe override (added 2026-05-15)

`spawn_daemon_process` in `speedy-core` now has a variant
`spawn_daemon_process_with(exe, socket)` that accepts an explicit path.
Exposed in `daemon_util::resolve_daemon_exe()` for UI/diagnostics.

In the GUI, the Dashboard shows:

- Currently resolved path (custom override or auto-detect).
- Text field + `Browse…` / `Apply` / `Restore automatic`.
- "Open folder" to jump to the folder containing the binary.

The override is persisted in `eframe::Storage` (field `daemon_exe_path`).
When set, `bridge.spawn_daemon()` uses it instead of auto-detect.
Main use case: GUI installed in a folder separate from the daemon
(e.g. `~/.local/bin/` for the GUI and `~/.local/libexec/` for the daemon).

Autostart at login: **removed from the GUI** (commit `c642282`). The GUI no
longer writes to the Windows registry / LaunchAgents / `.desktop`. To
start the daemon at user login, see README — the recommended
move on Windows remains a shortcut in `shell:startup`.
