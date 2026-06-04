# Test Roadmap — Speedy

## Current situation

| Package | Existing tests | Estimated coverage |
|---------|---------------|-------------------|
| speedy-mcp | 39 integration + unit (inline) | ~80% — excellent |
| speedy-cli | 13 E2E | ~80% (via daemon) — good |
| speedy-core | 4 cross-process + 9 serde unit | ~40% — good |
| speedy-language-context | 4 protocol + 7 indexer unit | ~35% — improved |
| speedy-ai-context | 23 db + 10 indexer + 6 embed + 12 text + 10 ignore | ~65% — good |
| speedy-daemon | 5+ direct | ~60% — improved |
| speedy-gui | 0 | 0% — not a priority |

---

## Priority 1 — Business-critical logic (risk of damage / data corruption)

### [x] speedy-ai-context: db.rs — schema migration
- **Done (pre-existing, verified 2026-05-19)**: 13 integration + 7 mock tests.
  Cover BLOB→vec0 migration, idempotency, insert/retrieve, clear, metadata.

### [x] speedy-ai-context: indexer.rs — reembed on model change
- **Done (pre-existing, verified 2026-05-19)**: 6 integration + 4 mock tests.
  Cover reembed+model_metadata, embed_cache dedup, content-change, deleted file, sync_all.

### [x] speedy-language-context: impact.rs — cycle detection in the call graph
- **Done (pre-existing, verified 2026-05-19)**: `test_find_impact_cycle_terminates` and another
  6 tests cover cycles, blast radius, root node, deduplication, fan-out.

### [x] speedy-daemon: self-write loop prevention
- **Done (2026-05-19)**: `test_active_pids_no_duplicates`, `test_active_pids_remove_cleans_up`,
  `test_prune_and_reconcile_removes_missing_workspace`,
  `test_prune_and_reconcile_keeps_existing_workspace`,
  `test_stop_all_watchers_sets_stop_flags_and_clears_map`.

---

## Priority 2 — Concurrency and reliability

### [x] speedy-core: workspace.rs — expand cross-process tests
- **Done (2026-05-19)**:
- `test_workspace_json_recovery_from_malformed`: fixture `list` on corrupted JSON → exit failure (no panic)
- `test_prune_missing_with_deleted_directory`: real path survives, ghost path removed,
  verified via `workspace-fixture prune` (new subcommand added to the fixture)
- File: `packages/speedy-core/tests/workspace_cross_process.rs`

### [x] speedy-daemon: workspace reconciliation
- **Done (2026-05-19)**: `test_prune_and_reconcile_removes_missing_workspace` and
  `test_stop_all_watchers_sets_stop_flags_and_clears_map` cover the main cases.

### [x] speedy-ai-context: embed.rs — provider failures (partial)
- **Done (2026-05-18)**: `CascadeEmbeddingProvider` with 4 cascade tests (first-healthy, fallback, all-fail, empty). `set_latency_ms` tested.
- **Remaining**: HTTP timeout of the real provider (would require a mock-HTTP-server).

### [x] speedy-core: daemon_client.rs — timeout and protocol version
- **Done (2026-05-18)**: `CONNECT_TIMEOUT`/`CMD_TIMEOUT` tested; `check_protocol_version()` + 4 mismatch tests added.

---

## Priority 3 — Correctness of minor units

### [x] speedy-ai-context: text.rs — chunking
- **Done (pre-existing, verified 2026-05-19)**: 12 tests cover edge cases (whitespace,
  unicode CJK, emoji, no punctuation, empty), by_paragraphs and by_sentences.

### [x] speedy-ai-context: ignore.rs — pattern matching
- **Done (pre-existing, verified 2026-05-19)**: 10 tests (binary detection, .speedyignore,
  filtered_files with gitignore and subdirectory).

### [x] speedy-language-context: indexer.rs — incremental
- **Done (2026-05-19)**: 7 tests added: skip unsupported extension (no crash),
  empty `.rs` file does not crash, symbols found in a real file, empty list ok.

### [x] speedy-language-context: skeleton.rs — detail levels
- **Done (pre-existing, verified 2026-05-19)**: 14 tests already present cover
  minimal/standard/detailed, from_str, public/private symbols, line numbers.

### [x] speedy-core: types.rs — serialization roundtrip
- **Done (pre-existing, verified 2026-05-19)**: 9 tests (DaemonStatus, Metrics,
  WorkspaceStatus, ScanResult, LogLine with optional fields and legacy fields).

---

## Priority 4 — Language context: tool implementations (MCP)

The existing tests in `packages/speedy-language-context/tests/mcp_binary_test.rs` verify
only the protocol. Add tests that verify the actual behavior of the tools:

### [x] `index_status` tool — indexed vs empty workspace
- **Done (2026-05-26)**: `test_index_status_shows_counts_after_reindex` — after force_reindex checks symbols > 0, files > 0, last_indexed != "never".
### [x] `get_skeleton` tool — workspace with real Rust symbols
- **Done (2026-05-26)**: `test_get_skeleton_after_indexing_returns_symbols` — after force_reindex, the skeleton of lib.rs contains "add".
### [x] `run_pipeline` tool — input/output on a test workspace
- **Done (2026-05-26)**: `test_run_pipeline_on_indexed_workspace` — task "add function" on an indexed workspace returns non-empty matches + impact.
### [x] `save_observation` / `search_observations` — FTS persistence
- **Done (pre-existing, verified 2026-05-26)**: covered by `test_save_observation_and_search_returns_result`, `test_search_observations_on_empty_store_returns_ok`, `test_save_multiple_observations_and_search`.

**Recommended pattern**: use `temp_workspace()` already present in the test file,
add real `.rs` files, call the tools after indexing.

---

## Infrastructure to build

### [x] `MockEmbeddingProvider`
- **Done (2026-05-18)**: added in `packages/speedy-ai-context/src/embed.rs` (test module). Deterministic vectors via FNV-1a hash, `set_fail_next`, `set_latency_ms`, call tracking. 6 tests.

### [x] `InMemoryGraphStore`
- **Done (pre-existing)**: `GraphStore::open(tempdir)` already used in all impact.rs tests.
  SQLite `:memory:` available but not necessary — tempdir is equivalent and already in use.

### [x] Sample Rust test workspace
- **Done (2026-05-19)**: `packages/speedy-language-context/tests/fixtures/sample_project/src/`
  contains `lib.rs`, `utils.rs`, `models.rs` with functions, struct, trait, impl, mixed visibility.

---

## What NOT to touch (already good)

- `packages/speedy-mcp/tests/integration_test.rs` — excellent, keep
- `packages/speedy-mcp/src/main.rs` `#[cfg(test)]` — good, expand only if necessary
- `packages/speedy-cli/tests/e2e_test.rs` — good, the watcher pipeline test is already the most complete
- `packages/speedy-core/tests/workspace_cross_process.rs` — good, just expand the cases

---

## Pattern to follow for new tests

```rust
// 1. Unit test with controlled dependencies
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_something_specific() {
        let dir = TempDir::new().unwrap();
        // minimal setup, deterministic test, no external dependencies
    }
}

// 2. Integration test with a real binary (pattern already used in speedy-mcp)
// - stage_binary() to avoid Windows lock
// - TempProject / DaemonGuard for isolation
// - quiet_command() for CREATE_NO_WINDOW

// 3. Never depend on Ollama in unit/integration tests
//    → MockEmbeddingProvider or conditional skip
```

## Rules for reliable tests

1. **No external dependencies** in unit tests (no Ollama, no network)
2. **Temp directories** always — never write to fixed paths
3. **Unique socket names** for daemon tests — use `uuid` or timestamp
4. **Explicit timeouts** on async operations — no tests that hang forever
5. **Guaranteed cleanup** — `Drop` impl or `defer!` for fixture removal
6. **Deterministic tests** — fixed seeds for PRNG, fixed embedding vectors
7. **Readable errors** — `unwrap_or_else(|e| panic!("context: {e}"))` > `.unwrap()`
