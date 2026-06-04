# TODO: Centralized Hash Registry

## Goal

Centralize file hash checking in a single shared registry (in `speedy-core`),
keeping each context **independent**: each one writes and reads only its own rows.
The daemon will no longer need to delegate change detection to the subprocesses — it can do it before
spawning them, avoiding starting unnecessary processes if the file has not changed.

---

## Current state

| Package | Hash | Storage | Mtime? | Change detection |
|---|---|---|---|---|
| `speedy-ai-context` | SHA256 | `sac.sqlite` table `chunks` (col. `hash`) | Yes (`last_modified`) | mtime → hash fallback |
| `speedy-language-context` | **blake3** | `slc.sqlite` table `slc_files` (col. `content_hash`) | Yes (`mtime` unix sec) | hash only |
| `speedy-text` | SHA256 | `stext.sqlite` table `indexed_files` (col. `hash`) | No | hash only |

Problems:
- The daemon always spawns the subprocess even if the file has not changed — the check happens *inside* the child process.
- Three different algorithms/storages: SHA256 (sac, stext) vs blake3 (slc).
- `speedy-text` has no mtime → always reads from disk.
- No cross-cutting view: there is no way to know "which contexts have already indexed this file".

---

## Proposed architecture

### 1. Shared Hash Registry in `speedy-core`

New module: `speedy-core/src/hash_registry.rs`

Database: `.speedy/hashes.sqlite` (one per workspace)

Schema:
```sql
CREATE TABLE file_hashes (
    file_path   TEXT    NOT NULL,   -- path relative to the workspace root
    context     TEXT    NOT NULL,   -- "ai-context" | "language-context" | "text"
    hash        TEXT    NOT NULL,   -- SHA256 hex
    mtime_secs  INTEGER NOT NULL,   -- unix timestamp seconds
    updated_at  INTEGER NOT NULL,   -- unix timestamp seconds
    PRIMARY KEY (file_path, context)
);
CREATE INDEX idx_file ON file_hashes(file_path);
```

Public API (`HashRegistry` struct):
```rust
pub fn open(workspace: &Path) -> Result<Self>
pub fn get(file: &str, context: &str) -> Option<FileHashEntry>
pub fn set(file: &str, context: &str, hash: &str, mtime_secs: u64) -> Result<()>
pub fn delete_file(file: &str) -> Result<()>                    // file removed
pub fn delete_context(context: &str) -> Result<()>             // full re-index
pub fn get_all_for_context(context: &str) -> HashMap<String, FileHashEntry>
pub fn needs_reindex(file: &Path, context: &str) -> Result<bool>  // mtime + hash check
```

`needs_reindex` implements two-level logic:
1. Reads mtime from disk → if equal to `mtime_secs` → `false` (fast path)
2. Reads content → SHA256 → if equal to `hash` → `false`, updates mtime
3. Otherwise → `true`

### 2. Standardize on SHA256

`speedy-language-context` uses blake3. Migrate to SHA256 in order to:
- Share the same implementation from `speedy-core`
- Avoid different dependencies for the same purpose
- Note: blake3 is faster, but the difference is negligible on small files — and consistency is worth more

Action: remove the `blake3` dependency from `speedy-language-context/Cargo.toml`,
use `speedy_core::hash_registry::hash_file()`.

### 3. Daemon: pre-check before spawning

In `speedy-daemon/src/main.rs` in the `start_workspace_watcher()` function (lines 413-487):

**Before** spawning the subprocess, check the shared registry:

```rust
// Pseudo-code
let registry = HashRegistry::open(&workspace_path)?;
let changed_for_sac = registry.needs_reindex(&file, "ai-context")?;
let changed_for_slc = registry.needs_reindex(&file, "language-context")?;
let changed_for_text = registry.needs_reindex(&file, "text")?;

if features.speedy_indexer && changed_for_sac {
    spawn_sac(&file, ...);
}
if features.language_context && changed_for_slc {
    spawn_slc(&file, ...);
}
if features.speedy_text && changed_for_text {
    spawn_text(&file, ...);
}
```

Benefit: if a file is saved twice without changes (redundant editor saves),
**no subprocess** is spawned.

### 4. Each context removes its own internal hash check

After adopting the registry, the redundant check inside each subprocess
can be simplified (but not removed entirely — the subprocess must still
update the registry after completing indexing).

Flow for each context after the change:
1. Subprocess started by the daemon (already pre-filtered)
2. Subprocess does its own work
3. Subprocess calls `registry.set(file, context, hash, mtime)` when done
4. The internal hash check can become an optional assert/sanity check

---

## Tasks

### speedy-core

- [x] Add `rusqlite` and `sha2` dependencies in `speedy-core/Cargo.toml`
- [x] Create `speedy-core/src/hash_registry.rs` with the `HashRegistry` struct and complete API
- [x] Implement `needs_reindex()` with mtime → SHA256 logic, `mtime_unchanged()` for the daemon
- [x] Implement `hash_file()` and `hash_bytes()` as public static methods
- [x] Add `hash_registry` to the `pub mod` declarations in `speedy-core/src/lib.rs`
- [x] Unit tests: new file, unchanged, modified, delete_context, independent contexts

### speedy-ai-context

- [x] `speedy-core` already present as a dependency
- [x] Added `mtime_secs` to `PreparedFile`, computed in `prepare_file()`
- [x] `reembed()` calls `registry.delete_context("ai-context")` before the wipe
- [x] `index_directory()` calls `registry.set_indexed()` for each file after the batch DB write
- [x] `index_file()` calls `registry.set_indexed()` after the DB write
- [x] `hash.rs` kept for internal backward compatibility (same SHA256 algorithm)

### speedy-language-context

- [x] `speedy-core` already present as a dependency
- [x] Removed the `blake3` dependency from `Cargo.toml`
- [x] `index_one_file()`: uses `HashRegistry::needs_reindex()` (adds mtime fast-path)
- [x] After upsert: calls `registry.set_indexed()`
- [x] `full_index_blocking()` calls `registry.delete_context("language-context")` before the walk
- [x] `slc_files.content_hash` kept for internal FK integrity (will now contain SHA256)

### speedy-text

- [x] Added `speedy-core` dependency in `Cargo.toml`
- [x] Removed the `sha2` dependency (now uses `HashRegistry::hash_bytes()`)
- [x] `sync()`: uses `mtime_unchanged()` fast-path + hash fallback via `HashRegistry`
- [x] `index()`: calls `registry.delete_context("text")` + `registry.set_indexed()` for each file
- [x] Mtime tracking added in the same step (does not require changing the stext.sqlite schema)

### speedy-daemon

- [x] Added `use speedy_core::hash_registry::HashRegistry`
- [x] Opens `HashRegistry` inside each spawned event-handler thread (Connection is !Send)
- [x] Pre-check `mtime_unchanged()` for `"ai-context"` and `"language-context"` before each spawn
- [x] `debug` log when a subprocess is skipped due to unchanged mtime

---

## Notes / Open decisions

- **Concurrency**: `hashes.sqlite` will be written by multiple processes (daemon + subprocess). Use WAL mode. Subprocesses write only their own `(file_path, context)` row → no conflicts between different contexts.
- **Hash pre-computed by the daemon**: the daemon could compute the hash once and pass it to the subprocess (env `SPEEDY_FILE_HASH=<hex>`), avoiding each subprocess re-reading the file. Evaluate whether it is worth the added complexity.
- **Migration**: on the first run with the new code, `hashes.sqlite` is empty → all contexts will do a one-time full re-index. Acceptable.
- **speedy-mcp / speedy-cli**: do not index directly, no changes needed.
