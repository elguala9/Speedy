# Speedy — General Todo

Status as of 2026-05-19. Covers tests, features, tech-debt, GUI, infrastructure.

---

## 1. Missing tests

### 1a. speedy-daemon — tests still absent

#### [x] Self-write loop prevention
- **Done (2026-05-19)**:
- `test_active_pids_no_duplicates` — HashSet deduplicates in-flight PIDs
- `test_active_pids_remove_cleans_up` — PID cleanup after child exit
- `.speedy/` already covered by `test_should_ignore_watch_path_speedy_internal` (pre-existing)

#### [x] Workspace reconciliation
- **Done (2026-05-19)**:
- `test_prune_and_reconcile_removes_missing_workspace` — orphaned workspace removed
- `test_prune_and_reconcile_keeps_existing_workspace` — valid workspace preserved
- `test_stop_all_watchers_sets_stop_flags_and_clears_map` — clean shutdown
- Reload from workspaces.json covered by `test_reload_picks_up_new_workspace` (pre-existing)

#### [x] New IPC commands
- `subscribe-log`: already tested by `test_stream_log_handshake_and_forward`
- `reindex`: already tested by `test_reindex_missing_path_errors`
- `prune-missing`: tested by `test_prune_missing_returns_json_with_removed_and_paths_keys`
  and `test_prune_missing_removes_orphaned_watcher` (done 2026-05-18)

### [x] 1b. speedy-core/daemon_client.rs — timeout and protocol version
- **Done (2026-05-18)**:
- `CONNECT_TIMEOUT` / `CMD_TIMEOUT` already tested by `test_is_alive_false_when_server_never_replies`
  and `test_cmd_connect_refused_returns_error` (pre-existing tests)
- Added `DaemonClient::check_protocol_version()` + 4 tests (same/older/newer/legacy-0)
  for the mismatch case → readable "upgrade speedy" error

### [x] 1c. speedy-ai-context/embed.rs — provider failures
- **Done (2026-05-18)**:
- Added `CascadeEmbeddingProvider` (vec of providers, automatic fallback on error)
- 4 cascade tests: first-healthy, fallback, all-fail, empty
- `set_latency_ms` tested with a real latency measurement (≥40ms)
- HTTP timeout of the real provider: would require a mock HTTP server, deferred

### 1d. speedy-language-context — tool behavior (not just protocol)
The tests in `mcp_binary_test.rs` only verify the JSON-RPC protocol.
Add tests that verify the actual output of the tools:

- `index_status` on an empty workspace vs an indexed workspace
- `get_skeleton` with real `.rs` files in the test workspace
- `run_pipeline` — search results on the test workspace
- `save_observation` / `search_observations` — FTS round-trip and persistence

**Pattern**: use `temp_workspace()`, already present in the test file, add real
`.rs` files, call the tools after indexing.
**Required infrastructure**: Rust workspace fixture (see §4)

### [x] 1f. speedy-ai-context/db.rs + indexer.rs + speedy-core/types.rs
- **Done (pre-existing, verified 2026-05-19)**:
- `db.rs`: 13 integration tests + 7 mock tests — BLOB→vec0 migration, idempotency, insert/retrieve, clear, persist, metadata
- `indexer.rs`: 6 integration tests + 4 mock tests — reembed+model_metadata, embed_cache dedup, content-change re-embed, deleted file, sync_all
- `types.rs`: 9 unit tests — serde roundtrip for DaemonStatus, Metrics, WorkspaceStatus, ScanResult, LogLine

### [x] 1e. speedy-core/workspace.rs — cross-process edge case
- **Done (2026-05-19)**:
- `test_workspace_json_recovery_from_malformed`: verifies that `list` on malformed JSON
  returns a handleable error (not a panic)
- `test_prune_missing_with_deleted_directory`: a real workspace survives, a non-existent path
  is removed; verified via `workspace-fixture prune`
- File: `packages/speedy-core/tests/workspace_cross_process.rs`

---

## 2. Incomplete or missing features

### [x] `chunk_count` in WorkspaceStatus is always `None`
- File: `packages/speedy-daemon/src/main.rs:1417`
- **Done (2026-05-18)**: added `rusqlite` to the daemon; `handle_workspace_status` opens
  the DB read-only and populates `chunk_count` with `SELECT COUNT(*) FROM chunks`.

### [x] Kotlin supported in the language-context parser
- **Done (2026-05-19)**:
- Used `fwcd/tree-sitter-kotlin` main (v0.4.0, not yet on crates.io) — uses
  `tree-sitter-language = "0.1"` (ABI-agnostic, compatible with tree-sitter 0.25).
  Pinned to rev `f66d2908` for reproducibility.
- `extract_kotlin()`: `function_declaration` → Function, `class_declaration` →
  Class or Interface (distinguished via the `interface` token), `object_declaration` → Struct.
- 2 tests: `parse_kotlin_fun_and_class`, `parse_kotlin_object_and_interface` — OK.
- **Note**: when `tree-sitter-kotlin` publishes to crates.io (≥ ABI 14), replace
  the git dep with the crates.io version to avoid a git dep in production.


### [x] speedy-mcp is too thin a proxy
- File: `packages/speedy-mcp/src/main.rs`
- **Done (2026-05-19)**: added a passthrough proxy for the speedy-language-context tools
  that have a CLI equivalent (`status` and `skeleton`).
- New tools exposed: `speedy_lc_status` (→ `speedy-language-context -p <path> status --json`)
  and `speedy_lc_skeleton` (→ `speedy-language-context -p <path> skeleton --detail <d> <files...>`)
- Environment variable `SPEEDY_LC_BIN` to override the binary (default: `speedy-language-context`)
- The MCP-only language-context tools (`run_pipeline`, `save_observation`, `search_observations`)
  remain in the separate server (`speedy-language-context serve`): they require in-memory state
  (GraphStore, Memory) that cannot be exposed via subprocess without a long-running server.
- Full unification of the two servers remains possible as a future refactor (it would require
  an async architecture and a dependency on speedy-language-context in speedy-mcp).

### [x] `_indexer` unused in `tool_index_status`
- **Done (2026-05-18)**:
- `full_index_blocking` and `index_files_blocking` now persist `last_files_indexed`,
  `last_symbols_found`, `last_index_duration_ms` in the GraphStore metadata
- `add_indexer_status_method` accepts `indexer: &Arc<Indexer>` and includes `workspace`,
  `last_indexed`, and the `last_run: { files_indexed, symbols_found, duration_ms }` block
- 2 new tests: `tool_index_status_returns_counts` updated + `tool_index_status_shows_last_run_stats_after_indexing`

---

## 3. Tech debt

### [x] Remove the unused `winreg` dependency from speedy-gui
- **Done (verified 2026-05-18)**: `winreg` is not present in `packages/speedy-gui/Cargo.toml`.
  It had already been removed earlier.

### [x] Clarify the role of `testexe`
- **Done (2026-05-19)**: Added a comment in `packages/testexe/Cargo.toml` documenting
  both binaries (`testexe` = manual E2E runner, `workspace-fixture` = cross-process
  fixture for `workspace_cross_process.rs`). Not obsolete — do not remove.

---

## 4. Test infrastructure to build

### [x] `MockEmbeddingProvider`
- **Done (2026-05-18)**: added in `packages/speedy-ai-context/src/embed.rs` (test module)
- Deterministic vectors via FNV-1a hash, `set_fail_next`, `set_latency_ms`, call tracking
- 6 new tests cover: determinism, diversity, bounds, fail injection, call tracking

### [x] Rust workspace fixture for language-context
- **Done (2026-05-19)**: created `lib.rs`, `utils.rs`, `models.rs` in
  `packages/speedy-language-context/tests/fixtures/sample_project/src/`
- Cover: pub/private functions, struct with methods, trait+impl, cross-file calls
- Used by the `indexer.rs` tests (7 new tests added)

---

## 5. GUI — deferred items

### [x] macOS: added to the release workflow
- **Done (2026-05-19)**: added the `x86_64-apple-darwin` / `macos-latest` target to the matrix.
  `speedy-gui` excluded from the macOS build pending eframe/winit validation in CI
  (the binary is omitted from the tar if absent). All other packages included.

---

## 6. Distribution

### [x] Windows installer (Inno Setup) — binary names verified
- **Done (2026-05-19)**: `installer/speedy.iss` already uses all the correct names:
  `speedy-ai-context.exe`, `speedy-daemon.exe`, `speedy-cli.exe`,
  `speedy-ai-context-mcp.exe`, `speedy-gui.exe`, `speedy-language-context.exe`,
  `speedy-language-context-mcp.exe`. No old name to update.

---

## Suggested priority

| Priority | Item |
|----------|------|
| ~~High~~ | ~~`chunk_count` in WorkspaceStatus~~ — **done** |
| ~~High~~ | ~~`MockEmbeddingProvider`~~ — **done** |
| ~~High~~ | ~~Daemon tests: loop prevention + reconciliation~~ — **done** |
| ~~Medium~~ | ~~daemon_client.rs timeout/protocol tests~~ — **done** |
| ~~Medium~~ | ~~Fix unused `_indexer` → expose IndexStats~~ — **done** |
| ~~Medium~~ | ~~`CascadeEmbeddingProvider` + tests~~ — **done** |
| ~~High~~ | ~~Daemon tests: self-write loop prevention + workspace reconciliation~~ — **done** |
| ~~Medium~~ | ~~Rust workspace fixture~~ — **done** |
| ~~Medium~~ | ~~Remove unused `winreg`~~ — **already removed** |
| ~~Medium~~ | ~~Fix unused `_indexer` → expose IndexStats~~ — **done** |
| ~~Low~~ | ~~Kotlin support~~ — **done** (git dep v0.4.0, rev f66d2908) |
| Low | Linux installer (Fedora) — `.rpm` with `cargo-generate-rpm` or AppImage |
