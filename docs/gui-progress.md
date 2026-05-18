# GUI Project — Progress Log

Working log for `todo-gui.md`. One section per completed macro-task.

---

## 2026-05-15 — §1 daemon prerequisites + §4 serde types

All daemon prerequisites described in `todo-gui.md` §1.1–§1.3 and §4 are
implemented and compile. The 57 daemon tests and 57 core tests remain green.

### Structured logging (§1.1)

- Daily rotating log files in `<daemon_dir>/logs/daemon.log.YYYY-MM-DD` via
  `tracing-appender::rolling::daily` with a non-blocking writer. The guard is
  intentionally leaked (`Box::leak`) so the write thread stays active for the
  entire process lifetime, no losses at shutdown.
- JSON layer on file (`tracing_subscriber::fmt::layer().json()`), text layer
  on stderr for interactive debug.
- Custom `BroadcastLayer` (`tokio::sync::broadcast<LogLine>`, capacity 1024)
  that feeds all active `subscribe-log` connections. A `FieldVisitor` impl of
  `tracing::field::Visit` extracts the `message` separately from extra fields.
- Added traces `target: "watcher"`, `target: "sync"`, `target: "index"`,
  `target: "ipc"` with structured fields (`workspace`, `ms`, etc.). Existing
  `error!` calls already contained the path.

### New IPC commands (§1.2)

- `tail-log [n]` → JSON array of `LogLine` (default 200). Finds the most
  recent log file in `<daemon_dir>/logs/` and parses the last N lines.
- `subscribe-log` → long-lived. The daemon responds `ok\n` then one JSON line
  per event until the client closes. `handle_connection` now explicitly
  recognizes this command as the only non-one-shot one.
- `scan\t<root>[\t<max_depth>]` → `walkdir` with skip on `target`, `.git`,
  `node_modules`, `dist`, etc. For each directory containing
  `.speedy/index.sqlite` returns a `ScanResult`.
- `reindex <path>` → spawns `speedy index .` with `cwd=<path>` and
  `SPEEDY_NO_DAEMON=1`. Increments `metrics.indexes`.
- `workspace-status <path>` → `WorkspaceStatus` with `watcher_alive`,
  `last_event_at`, `last_sync_at`, `index_size_bytes`. `chunk_count` is
  `None` for now (would require opening the DB from here — deferred).

### Shared serde types (§4)

New module `speedy-core/src/types.rs` with `DaemonStatus`, `Metrics`,
`WorkspaceStatus`, `ScanResult`, `LogLine`. `daemon_client::DaemonStatus`
is now `pub use` of the types in `types`, no shape duplication.

### Protocol version (§1.3)

Was already at 2 (bumped for `query-all` on 2026-05-14). No further work.

### DaemonClient

New methods: `reload`, `scan`, `reindex`, `workspace_status`, `tail_log`,
`subscribe_log`. The latter returns `(UnboundedReceiver<LogLine>, JoinHandle<()>)`
— dropping the receiver closes the background task.

### Files touched

- `packages/speedy-core/Cargo.toml` — added `walkdir`.
- `packages/speedy-core/src/lib.rs` — exposes `types`.
- `packages/speedy-core/src/types.rs` — **new**.
- `packages/speedy-core/src/daemon_client.rs` — uses types, +6 methods.
- `packages/speedy-daemon/Cargo.toml` — `tracing-subscriber[json]`,
  `tracing-appender`, `walkdir`, `chrono`.
- `packages/speedy-daemon/src/main.rs` — `BroadcastLayer`, `FieldVisitor`,
  `WatcherHandle` enriched with `last_event_at`/`last_sync_at`, new
  commands in `dispatch_command`, `handle_connection` with `subscribe-log`
  branch, `main()` reinitializes tracing.
- `docs/ipc-protocol.md` — documented the 6 new commands.

### What was NOT done in this round

- `restart` command: the TODO suggests leaving it to the GUI (stop + spawn).
  Agreed, not adding it on the daemon side.
- `chunk_count` in `WorkspaceStatus` remains `None`.
- Integration tests for the new commands (§5) — pass cargo check, but no
  dedicated assertions yet.

### Next step

§2 scaffolding `speedy-gui` crate with `egui` + `eframe`, then Phase A MVP.

---

## 2026-05-15 (part 2) — §2 GUI + Phase A/B/C/D minimal + §5 tests

Crate `speedy-gui` created and builds in debug and release. The GUI covers all
four phases from the TODO with a set of working features; the visual polish
(animations, refined layout) is yet to be refined but the functional part is there.

### Scaffolding (§2)

New crate `packages/speedy-gui/` added to the workspace. Dependencies:
`eframe` + `egui` 0.28, `tokio` (multi-thread runtime in background),
`rfd` for the native file picker, `tracing`, `dirs`, `chrono`, `serde`.

Final layout:

```
packages/speedy-gui/
├── Cargo.toml
└── src/
    ├── main.rs           # bootstrap eframe, windows_subsystem=windows in release
    ├── app.rs            # SpeedyApp impl eframe::App, persistence via Storage
    ├── daemon.rs         # DaemonBridge: tokio rt + Arc<Mutex<DaemonState>>
    ├── log_stream.rs     # LogStreamHandle with ring buffer (5000 lines)
    └── views/
        ├── mod.rs        # Tab enum
        ├── dashboard.rs  # status + metrics + restart/reload/stop
        ├── workspaces.rs # list + add (file picker) + sync/index + remove confirmation
        ├── scan.rs       # form + table + register-batch
        └── logs.rs       # live tail with filters (level, substring, target, workspace)
```

### Sync ⇄ async architecture

egui is immediate-mode so the main loop cannot block on IPC. Solution:

- `DaemonBridge` owns a `tokio::runtime::Runtime` multi-thread (2 workers)
  and a shared `Arc<Mutex<DaemonState>>`.
- Each public method (`refresh_all`, `add_workspace`, `sync_workspace`,
  `scan`, …) is sync, calls `inc_busy()`, spawns a task on the runtime, and
  writes the result into state when the task completes.
- `App::update()` clones the state each frame (it's a cheap `Clone` of
  moderate Vec/HashMap), then views read from the snapshot — no Mutex held
  while drawing.
- `ctx.request_repaint_after(500ms)` ensures the UI reflects background
  updates even when the mouse is not moving.

### Phase A (MVP)

- **Topbar** with app name + tabs + daemon indicator (green/red/probing) +
  spinner when IPC calls are in flight + light/dark theme toggle.
- **"Start daemon" banner** when ping fails (calls `spawn_daemon_process`).
- **Dashboard**: PID, uptime, version, protocol_version, workspace_count,
  watcher_count, cumulative metrics, clickable link to config dir
  (opens Explorer on Windows, `open` on macOS, `xdg-open` on Linux).
- **Workspaces**: scrollable table, watcher status badge, DB size, "event N
  ago" and "sync N ago" from `workspace-status`, Index/Sync/Open
  folder/Remove buttons (with confirmation and note that on-disk DB is not touched).
- **Native file picker** via `rfd::FileDialog` for "Add workspace".

### Phase B (Operations)

- Index/Sync per workspace — with green/red toast on completion.
- **Scan**: form with root path + max depth (`DragValue` 1..=20), Scan button
  → results table with "Registered" column colored, selection checkboxes
  (disabled for already-registered ones), "Register selected" in batch.
  NO option to delete `.speedy/` on disk — unregister only, as decided.
- **Restart**: stop IPC → polling `is_alive` with 200ms backoff (max 10s)
  → detached spawn of the daemon binary. All in background, responsive UI.
- **Reload** and **Stop daemon** with colored confirmation.

### Phase C (Log viewer)

- `LogStreamHandle` connected to `subscribe-log` IPC. Ring buffer cap 5000
  to avoid RAM explosion. Automatic reconnection every 2s if the pipe dies
  (e.g. daemon restarted).
- Filters: levels (5 checkboxes), case-insensitive substring, target
  (also substring), workspace (reads `workspace` field from LogLine).
- Follow tail toggle (egui ScrollArea::stick_to_bottom).
- Color coding by level (error red, warn orange, info blue, debug/trace grey).
- "Clear buffer" and "Restart stream" buttons.

### Phase D (Minimal polish)

- Light/dark theme toggle in topbar.
- Persistent settings via `eframe::Storage`: selected tab, dark mode,
  socket name.
- Status bar at the bottom with toasts (6s lifetime) and current socket name.

**Deferred:** system tray icon (requires integration with `tray-icon` crate +
a separate event loop; non-trivial with winit/eframe — worth a dedicated round),
system autostart (HKCU\Run on Windows), system notifications on error.

### Tests (§5)

8 new tests in `speedy-daemon/src/main.rs` (`tests` module):

- `test_workspace_status_unknown_path_errors` — canonicalize fail → `error:`
- `test_workspace_status_known_path_reports_no_watcher` — JSON with
  `watcher_alive=false` and `index_size_bytes=0`
- `test_scan_finds_directory_with_index_sqlite` — creates
  `<root>/proj-a/.speedy/index.sqlite`, verifies `scan` finds it
- `test_scan_missing_root_returns_empty_array` — `walkdir` on
  nonexistent path → `[]`
- `test_reindex_missing_path_errors` — canonicalize fail on missing path
- `test_tail_log_returns_empty_when_no_logs` — empty `logs/` directory
- `test_tail_log_parses_json_lines` — mix of JSON and junk lines; junk
  is skipped, JSON lines are parsed correctly
- `test_stream_log_handshake_and_forward` — uses `tokio::io::duplex` as
  a fake socket, verifies `ok\n` handshake + JSON LogLine serialization

All 65 daemon tests pass (57 existing + 8 new). The 57 `speedy-core` tests
remain green. Workspace `cargo check --all-targets` green, release build green.

### Files touched in this round

- `Cargo.toml` (root) — added `packages/speedy-gui` to members.
- `packages/speedy-gui/Cargo.toml` — **new**.
- `packages/speedy-gui/src/{main.rs, app.rs, daemon.rs, log_stream.rs,
  views/{mod.rs, dashboard.rs, workspaces.rs, scan.rs, logs.rs}}` — **new**.
- `packages/speedy-daemon/src/main.rs` — 8 new `#[tokio::test]` in `tests`.

### What was NOT done

- System tray icon (`tray-icon` crate).
- Daemon auto-start at user login (HKCU\Run on Windows, plist on
  macOS, `.desktop` autostart on Linux).
- System notifications on `error` event (configurable, off by default).
- Log selection export to `.log`/`.json` file (just needs the buffer; the
  button is to be added — 5 lines of `rfd::FileDialog::save_file` +
  `serde_json::to_writer`).
- Log history: dropdown with `daemon.log.*` files in `<daemon_dir>/logs/`
  and a read-only view of the selected file.
- GUI backend tests with mock `DaemonClient` (the `DaemonBridge` struct has no
  dedicated unit tests; coverage is indirect via daemon tests).
- Manual E2E smoke test: the binary builds but was not launched (non-interactive session).

### How to try it

```powershell
cargo run --release -p speedy-gui
```

If the daemon is not running you will see the "Start daemon" banner.

### Final TODO task status

- [x] §1.1 structured logging + IPC streaming
- [x] §1.2 new commands (scan, reindex, workspace-status, tail-log, subscribe-log)
- [x] §1.3 protocol_version=2 (was already OK)
- [x] §2 speedy-gui crate scaffolding (egui)
- [x] §4 shared serde types in speedy-core
- [x] Phase A MVP (connection/dashboard/workspace)
- [x] Phase B operations (index/sync/scan/restart)
- [x] Phase C log viewer
- [~] Phase D polish — theme + persistence done; tray/autostart/notifications deferred
- [~] §5 tests — 8 daemon integration OK; GUI mock deferred

---

## 2026-05-15 (part 3) — closing deferred items + cargo build-all

The user asked to complete all still-open deferred items and to add a unified
command for building all binaries. All done in one session.

### Cargo alias `build-all`

New file `.cargo/config.toml`:
```
[alias]
build-all = "build --release -p speedy -p speedy-daemon -p speedy-cli -p speedy-mcp -p speedy-gui"
```

Verification: `cargo build-all` → completes in ~45s and produces the 5 .exe in
`target/release/`. The `scripts/build-release.{ps1,sh}` scripts remain the
"all-in-one" solution (build + copy to `dist/`).

### Tray icon (`tray.rs`, new)

`tray-icon = "0.19"` with a 16x16 RGBA icon generated in code (green disk
when the daemon is alive, red when down — no embedded binary files).
Menu: read-only status entry ("Daemon: ● alive/down"), separator, "Open
Speedy", "Restart daemon", separator, "Quit".

Actions are drained each frame from `App::update` via
`MenuEvent::receiver().try_recv()` (non-blocking). `TrayHandle::set_alive`
updates icon + label only when state changes (atomic).

`TrayHandle::try_new` returns `None` if the platform doesn't support the tray
(typically Linux without AppIndicator). The app continues to work without it.
The handle lives in `Arc<TrayHandle>` and is created on the main thread *before*
`eframe::run_native`, a Windows/macOS requirement.

### System notifications on error (`notify-rust = "4.11"`)

"System notifications on error" toggle in Dashboard, persisted via
`eframe::Storage`. When active, `App::notify_new_errors` scans only new lines
from the live stream (delta vs `last_notified_log_count`,
clamped to the actual ring buffer) and for each `error` level calls
`notify_rust::Notification::new().summary(...).body(...).show()`.

### Daemon auto-start at login (`autostart.rs`, new)

Cross-platform module with three cfg-branches:
- **Windows**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` via
  `winreg = "0.52"`. Value = *quoted* path to `speedy-daemon.exe`
  (so spaces in Program Files don't break it).
- **macOS**: `~/Library/LaunchAgents/com.speedy.daemon.plist` with
  `RunAtLoad=true, KeepAlive=false`.
- **Linux**: `~/.config/autostart/speedy-daemon.desktop` with
  `X-GNOME-Autostart-enabled=true`.

API: `is_enabled() -> Result<bool>`, `enable()`, `disable()`. Executable
lookup searches for `speedy-daemon{EXE_SUFFIX}` next to the GUI binary.

UI: checkbox in Dashboard ("Start daemon at user login"). Toggling the
checkbox calls enable/disable and shows a toast (green on success, red on error).

### Log export + log history (`views/logs.rs`, rewritten)

Added a **source** selector at the top of the view:
- "Live (stream)" — previous behavior, `subscribe-log` IPC.
- ComboBox with `daemon.log.*` files found in `<daemon_dir>/logs/`
  (sorted by name descending, most recent first).

When the user selects a different file, it is loaded and parsed once
(cache `history_loaded_path`). The parser is tolerant: first tries the
`LogLine` shape (IPC) then projects the `tracing_subscriber::fmt::layer().json()`
shape (top-level `timestamp`, `level`, `target`, `fields.message`, other `fields.*`).

"Export selection (N)" button always available: uses
`rfd::FileDialog::save_file()` with JSON/JSONL filters. Extension `.jsonl`
→ one record per line (newline-delimited); otherwise pretty-printed JSON array.
Shows a toast with the path on success.

### GUI backend tests (§5, completed)

5 new `#[test]` in `speedy-gui/src/daemon.rs#tests`:

- `daemon_state_toast_helper_round_trips` — `set_toast` correctly sets message/ok
  in `state.toast`.
- `bridge_against_dead_socket_marks_probed_not_alive` — refresh against
  nonexistent socket, polling until `busy==0`; verifies
  `alive=false, probed=true, status=None`.
- `bridge_against_mock_marks_alive_and_loads_status_and_metrics` —
  fake listener on a separate runtime responds to ping/status/metrics/list;
  bridge after `refresh_all` has `alive=true, status.pid==42,
  status.protocol_version==2, metrics=Some(...)`.
- `busy_counter_settles_after_multiple_overlapping_calls` — 5 consecutive
  refreshes against dead socket, busy must return to 0 (no underflow).
- `workspace_status_error_on_dead_socket_surfaces_in_last_error` —
  `refresh_workspace_status` against dead socket → `state.last_error`
  contains "workspace-status".

Mock pattern: as in `daemon_client::tests`, but DaemonBridge already has its
own tokio runtime, so the mock lives in a *separate runtime*
(`tokio::runtime::Runtime::new()` local to the test). After the assertion,
dropping the runtime frees the OS socket.

### New dependencies (speedy-gui)

```toml
tray-icon = "0.19"
notify-rust = "4.11"

[target.'cfg(windows)'.dependencies]
winreg = "0.52"
```

### Files touched / new

- `.cargo/config.toml` — **new** (build-all alias).
- `packages/speedy-gui/Cargo.toml` — +3 deps (tray, notify, winreg).
- `packages/speedy-gui/src/main.rs` — creates `TrayHandle::try_new()` before
  `eframe::run_native`, passes `Option<Arc<TrayHandle>>` to `SpeedyApp::new`.
- `packages/speedy-gui/src/app.rs` — `notify_on_error` persisted,
  `handle_tray_actions`, `notify_new_errors`, enriched signature of
  `views::dashboard::render`.
- `packages/speedy-gui/src/tray.rs` — **new**.
- `packages/speedy-gui/src/autostart.rs` — **new**.
- `packages/speedy-gui/src/views/logs.rs` — rewritten: source switch
  (Live / File), export button, history viewer.
- `packages/speedy-gui/src/views/dashboard.rs` — notifications checkbox +
  autostart checkbox (with toast feedback).
- `packages/speedy-gui/src/daemon.rs` — +5 tests.
- `todo-gui.md` — all tasks closed.

### Result

`cargo check --workspace --all-targets` → green.
`cargo build-all` → 5 release binaries in 45s.
`cargo test -p speedy-core --lib` → 57/57.
`cargo test -p speedy-daemon --bin speedy-daemon` → 65/65.
`cargo test -p speedy-gui` → 5/5.

Only the manual E2E smoke test remains (`cargo run --release -p speedy-gui` and
visual verification of tray icon + notifications + autostart), not runnable in a
non-interactive session.

---

## 2026-05-15 (part 4) — autostart removed, daemon-exe override, prune-missing

Decisions from part 3 were partially revised. Current state in the code
(verify `master` HEAD `c642282`):

### Autostart removed from the GUI

`packages/speedy-gui/src/autostart.rs` **no longer exists**. The `winreg`
dependency remains in `Cargo.toml` only for future use (can be removed).
The Dashboard **no longer** has the "Start daemon at user login" checkbox.

Rationale: writing to `HKCU\…\Run` / `LaunchAgents` /
`~/.config/autostart` is invasive for an app distributed as a tarball of
binaries. The user manually places a shortcut to `speedy-daemon` in the
Startup folder (Windows) or equivalent — see README §"Recommended layout".

References in these notes (part 3 §"Daemon auto-start at login",
file `autostart.rs`, checkbox in Dashboard) should be read as "done then rolled back".

### Daemon-exe override

Added a "Daemon executable" field in the Dashboard with `Browse…` /
`Apply` / `Reset to auto`. Persisted as
`PersistedSettings.daemon_exe_path` in `eframe::Storage`. The bridge uses
`spawn_daemon_process_with(exe, socket)` when the override is set.

New API in `speedy-core/src/daemon_util.rs`:
`pub fn resolve_daemon_exe() -> Result<PathBuf>`
`pub fn spawn_daemon_process_with(exe: &Path, socket_name: &str) -> Result<()>`

### Prune-missing (IPC + GUI)

New one-shot IPC command `prune-missing`:
- Daemon: `handle_prune_missing` stops watchers for nonexistent paths,
  removes entries from `workspaces.json`, responds with
  `{"removed": N, "paths": [...]}`.
- Client: `DaemonClient::prune_missing()` in speedy-core.
- GUI: "🧹 Prune orphans" button in the Workspaces tab (plus a
  row with "⚠ missing" badge on workspaces whose path doesn't exist).

### Test status (current HEAD)

`cargo check --workspace --all-targets` → green (1 warning: unused
import `StreamTrait` in `speedy-cli/src/main.rs:147` — trivial fix).

`cargo test --workspace -- --test-threads=1` → in progress at the time
these notes were written; verify the result before trusting the history in part 3.
