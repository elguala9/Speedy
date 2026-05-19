# Speedy — Todo generale

Stato al 2026-05-19. Copre test, feature, tech-debt, GUI, infrastruttura.

---

## 1. Test mancanti

### 1a. speedy-daemon — test ancora assenti

#### [x] Self-write loop prevention
- **Done (2026-05-19)**:
- `test_active_pids_no_duplicates` — HashSet deduplica i PID in-flight
- `test_active_pids_remove_cleans_up` — cleanup PID dopo exit child
- `.speedy/` già coperto da `test_should_ignore_watch_path_speedy_internal` (preesistente)

#### [x] Workspace reconciliation
- **Done (2026-05-19)**:
- `test_prune_and_reconcile_removes_missing_workspace` — workspace orfana rimossa
- `test_prune_and_reconcile_keeps_existing_workspace` — workspace valido preservato
- `test_stop_all_watchers_sets_stop_flags_and_clears_map` — shutdown pulito
- Reload da workspaces.json coperto da `test_reload_picks_up_new_workspace` (preesistente)

#### [x] Nuovi comandi IPC
- `subscribe-log`: già testato da `test_stream_log_handshake_and_forward`
- `reindex`: già testato da `test_reindex_missing_path_errors`
- `prune-missing`: testato da `test_prune_missing_returns_json_with_removed_and_paths_keys`
  e `test_prune_missing_removes_orphaned_watcher` (done 2026-05-18)

### [x] 1b. speedy-core/daemon_client.rs — timeout e versione protocollo
- **Done (2026-05-18)**:
- `CONNECT_TIMEOUT` / `CMD_TIMEOUT` già testati da `test_is_alive_false_when_server_never_replies`
  e `test_cmd_connect_refused_returns_error` (test preesistenti)
- Aggiunto `DaemonClient::check_protocol_version()` + 4 test (same/older/newer/legacy-0)
  per il caso mismatch → errore leggibile "upgrade speedy"

### [x] 1c. speedy-ai-context/embed.rs — fallimenti provider
- **Done (2026-05-18)**:
- Aggiunto `CascadeEmbeddingProvider` (vec di provider, fallback automatico su errore)
- 4 test cascade: first-healthy, fallback, all-fail, empty
- `set_latency_ms` testato con misura di latenza reale (≥40ms)
- HTTP timeout del provider reale: richiederebbe mock-HTTP-server, rimandato

### 1d. speedy-language-context — tool behavior (non solo protocollo)
I test in `mcp_binary_test.rs` verificano solo il protocollo JSON-RPC.
Aggiungere test che verifichino l'output reale degli strumenti:

- `index_status` su workspace vuoto vs workspace indicizzato
- `get_skeleton` con file `.rs` reali nel workspace di test
- `run_pipeline` — risultati di ricerca su workspace di test
- `save_observation` / `search_observations` — FTS round-trip e persistenza

**Pattern**: usare `temp_workspace()` già presente nel test file, aggiungere file
`.rs` reali, chiamare gli strumenti dopo indexing.
**Infrastruttura necessaria**: fixture workspace Rust (vedi §4)

### [x] 1f. speedy-ai-context/db.rs + indexer.rs + speedy-core/types.rs
- **Done (preesistente, verificato 2026-05-19)**:
- `db.rs`: 13 integration tests + 7 mock tests — migration BLOB→vec0, idempotenza, insert/retrieve, clear, persist, metadata
- `indexer.rs`: 6 integration tests + 4 mock tests — reembed+model_metadata, embed_cache dedup, content-change re-embed, deleted file, sync_all
- `types.rs`: 9 unit tests — serde roundtrip per DaemonStatus, Metrics, WorkspaceStatus, ScanResult, LogLine

### [x] 1e. speedy-core/workspace.rs — cross-process edge case
- **Done (2026-05-19)**:
- `test_workspace_json_recovery_from_malformed`: verifica che `list` su JSON malformato
  ritorni errore gestibile (non panic)
- `test_prune_missing_with_deleted_directory`: workspace reale sopravvive, path inesistente
  viene rimosso; verifica via `workspace-fixture prune`
- File: `packages/speedy-core/tests/workspace_cross_process.rs`

---

## 2. Feature incomplete o mancanti

### [x] `chunk_count` in WorkspaceStatus è sempre `None`
- File: `packages/speedy-daemon/src/main.rs:1417`
- **Done (2026-05-18)**: aggiunto `rusqlite` al daemon; `handle_workspace_status` apre
  il DB in read-only e popola `chunk_count` con `SELECT COUNT(*) FROM chunks`.

### [ ] Kotlin non supportato nel parser language-context
- **File**: `packages/speedy-language-context/src/parser/tree_sitter_parser.rs`
- **Problema**: `tree-sitter-kotlin` 0.3.x usa la ABI di tree-sitter 0.20; Speedy è aggiornato
  a tree-sitter 0.25 (ABI 15). Le due versioni non sono linkabili insieme — il crate attuale
  di Kotlin produce errori di compilazione o runtime al caricamento della grammar.
- **Workaround attuale**: le estensioni `.kt` / `.kts` sono silenziosamente skippate dal parser
  (nessun crash, nessun simbolo indicizzato).
- **Sblocco**: aggiungere il supporto quando esce un release di `tree-sitter-kotlin` compatibile
  con tree-sitter ≥ 0.23 (ABI 14+). Monitorare: https://crates.io/crates/tree-sitter-kotlin
- **Implementazione**: aggiungere `tree-sitter-kotlin` a `Cargo.toml`, rimuovere il commento
  di skip in `tree_sitter_parser.rs` e aggiungere il caso `"kt" | "kts"` alla match.


### [x] speedy-mcp è un proxy troppo thin
- File: `packages/speedy-mcp/src/main.rs`
- **Done (2026-05-19)**: aggiunto proxy passthrough per i tool di speedy-language-context
  che hanno equivalente CLI (`status` e `skeleton`).
- Nuovi tool esposti: `speedy_lc_status` (→ `speedy-language-context -p <path> status --json`)
  e `speedy_lc_skeleton` (→ `speedy-language-context -p <path> skeleton --detail <d> <files...>`)
- Variabile d'ambiente `SPEEDY_LC_BIN` per override del binario (default: `speedy-language-context`)
- Tool MCP-only di language-context (`run_pipeline`, `save_observation`, `search_observations`)
  restano nel server separato (`speedy-language-context serve`): richiedono stato in-memory
  (GraphStore, Memory) non esponibile via subprocess senza un server long-running.
- Unificazione completa dei due server rimane possibile come refactor futuro (richiederebbe
  architettura async e dipendenza da speedy-language-context in speedy-mcp).

### [x] `_indexer` non usato in `tool_index_status`
- **Done (2026-05-18)**:
- `full_index_blocking` e `index_files_blocking` ora persistono `last_files_indexed`,
  `last_symbols_found`, `last_index_duration_ms` nei metadati del GraphStore
- `add_indexer_status_method` accetta `indexer: &Arc<Indexer>` e include `workspace`,
  `last_indexed`, e il blocco `last_run: { files_indexed, symbols_found, duration_ms }`
- 2 nuovi test: `tool_index_status_returns_counts` aggiornato + `tool_index_status_shows_last_run_stats_after_indexing`

---

## 3. Tech debt

### [x] Rimuovere dipendenza `winreg` inutilizzata da speedy-gui
- **Done (verificato 2026-05-18)**: `winreg` non è presente in `packages/speedy-gui/Cargo.toml`.
  Era già stato rimosso in precedenza.

### [x] Chiarire il ruolo di `testexe`
- **Done (2026-05-19)**: Aggiunto commento in `packages/testexe/Cargo.toml` che documenta
  entrambi i binari (`testexe` = runner E2E manuale, `workspace-fixture` = fixture
  cross-process per `workspace_cross_process.rs`). Non obsoleto — non rimuovere.

---

## 4. Infrastruttura test da costruire

### [x] `MockEmbeddingProvider`
- **Done (2026-05-18)**: aggiunto in `packages/speedy-ai-context/src/embed.rs` (modulo test)
- Vettori deterministici via FNV-1a hash, `set_fail_next`, `set_latency_ms`, tracciamento chiamate
- 6 nuovi test coprono: determinismo, diversità, bounds, fail injection, call tracking

### [x] Fixture workspace Rust per language-context
- **Done (2026-05-19)**: creati `lib.rs`, `utils.rs`, `models.rs` in
  `packages/speedy-language-context/tests/fixtures/sample_project/src/`
- Coprono: funzioni pub/private, struct con metodi, trait+impl, chiamate cross-file
- Usati dai test di `indexer.rs` (7 nuovi test aggiunti)

---

## 5. GUI — cose rinviate

### [x] macOS: aggiunto al release workflow
- **Done (2026-05-19)**: aggiunto target `x86_64-apple-darwin` / `macos-latest` alla matrix.
  `speedy-gui` escluso dal build macOS in attesa di validazione eframe/winit in CI
  (binario omesso dal tar se assente). Tutti gli altri package inclusi.

---

## 6. Distribuzione

### [x] Installer Windows (Inno Setup) — nomi binari verificati
- **Done (2026-05-19)**: `installer/speedy.iss` usa già tutti i nomi corretti:
  `speedy-ai-context.exe`, `speedy-daemon.exe`, `speedy-cli.exe`,
  `speedy-ai-context-mcp.exe`, `speedy-gui.exe`, `speedy-language-context.exe`,
  `speedy-language-context-mcp.exe`. Nessun vecchio nome da aggiornare.

---

## Priorità suggerita

| Priorità | Item |
|----------|------|
| ~~Alta~~ | ~~`chunk_count` in WorkspaceStatus~~ — **done** |
| ~~Alta~~ | ~~`MockEmbeddingProvider`~~ — **done** |
| ~~Alta~~ | ~~Test daemon: loop prevention + reconciliation~~ — **done** |
| ~~Media~~ | ~~Test daemon_client.rs timeout/protocol~~ — **done** |
| ~~Media~~ | ~~Fix `_indexer` non usato → esporre IndexStats~~ — **done** |
| ~~Media~~ | ~~`CascadeEmbeddingProvider` + test~~ — **done** |
| ~~Alta~~ | ~~Test daemon: self-write loop prevention + workspace reconciliation~~ — **done** |
| ~~Media~~ | ~~Fixture workspace Rust~~ — **done** |
| ~~Media~~ | ~~Rimuovere `winreg` inutilizzato~~ — **già rimosso** |
| ~~Media~~ | ~~Fix `_indexer` non usato → esporre IndexStats~~ — **done** |
| Bassa | Kotlin support (attesa release `tree-sitter-kotlin` ≥ ABI 14) |
| Bassa | Installer Linux (Fedora) — `.rpm` con `cargo-generate-rpm` o AppImage |
