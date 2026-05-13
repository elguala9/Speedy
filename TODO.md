# TODO — Completion Status

## CLI alignment with `instructions.md` ✅

All top-level flags from `instructions.md` are now implemented:

| instructions.md | Status | Notes |
|---|---|---|
| `--daemons \| -d` | ✅ | `-d` shorthand added (`--detach` moved to long-only) |
| `--daemon-stop \| -ds` | ✅ | `--daemon-stop <path>` + `--ds` alias |
| `--daemon-restart \| -dr` | ✅ | `--daemon-restart <path>` + `--dr` alias |
| `--daemon-delete \| -dd` | ✅ | `--daemon-delete <path>` + `--dd` alias |
| `--daemon-create \| -dc` | ✅ | `--daemon-create <path>` + `--dc` alias |
| `--force \| -f` | ✅ | `-f` moved from `--file` to `--force`, `--file` is now long-only |
| `--workspaces \| -w` | ✅ | `-w` shorthand added |
| `--workspace-create \| -wc` | ✅ | `--workspace-create <path>` + `--wc` alias |
| `--workspace-delete` | ✅ | `--workspace-delete <path>` |

## Missing features ✅

- [x] **Global `-p` path flag** — resolved: subcommands use `-p` short only (no `--path` long to avoid collision)
- [x] **`--daemon-status` flag** — added as `--daemon-status <path>`
- [x] **Daemon requires workspace validation** — `install()` now checks `workspace::is_registered()`
- [x] **`-d` flag collision** — resolved: `--detach` is long-only, `-d` = `--daemons`

## Code quality ✅

- [x] **Mock TCP server** (`server.rs`, `handler.rs`, `router.rs`) — removed entirely
- [x] **Demo CLI binary** (`bin/cli.rs`) — removed entirely
- [x] **`CachedEntry.id` field** (db.rs) — removed (unused)
- [x] **`daemon.rs` `task_name` variable** — now used in Windows uninstall path
- [x] **`handler.rs` unused `HashMap` import** — file removed
- [x] **`Cargo.lock` in `.gitignore`** — removed from `.gitignore`
- [x] **`now_rfc3339()`** — now uses `chrono::Utc::now().to_rfc3339()` (no more hardcoded 2024)
- [x] **`.speedyignore` documentation** — added to README

## Testing ✅

- [x] **Integration tests** — 5 integration tests in `tests/integration_test.rs`
- [x] **Tests for watcher** — 2 tests (extension filtering)
- [x] **Tests for daemon lifecycle** — 3 tests (read/write/update info)
- [x] **Tests for CLI parsing** — 22 tests covering all flags and subcommands
- [x] **Cross-platform** — daemon read/write tests work on all platforms

## Enhancements ✅

- [x] **Config file support** — `speedy.toml` / `.speedy/config.toml` with env var override
- [x] **Better error messages** — added `context()` from anyhow in indexer, db, workspace, hash
- [x] **Embedding cache** — avoids recomputing embeddings for identical chunk content
- [x] **Progress reporting** — `indicatif` progress bar in `index_directory`
- [x] **Graceful shutdown for watcher** — `ctrlc` signal handler in `start_watcher`
- [x] **Windows daemon as proper service** — now uses proper Windows Service via `sc.exe` (create/delete/start/stop) + `windows-service` crate (service control handler)

## Summary

- **67 tests** passing (62 unit + 5 integration)
- **0 warnings** on build
- All `instructions.md` CLI flags supported (top-level and subcommands)
- Mock/stub code removed
- Config file support added
- Embedding cache and progress reporting implemented
