# Speedy — Todo generale

Stato al 2026-05-18. Copre test, feature, tech-debt, GUI, infrastruttura.

---

## 1. Test mancanti

### 1a. speedy-daemon — test ancora assenti

#### [ ] Self-write loop prevention
- File: `packages/speedy-daemon/src/main.rs`
- I processi figli devono partire con `SPEEDY_NO_DAEMON=1`
- Il watcher deve ignorare `.speedy/` (no loop su DB writes)
- `active_pids` deve prevenire doppio avvio dello stesso workspace

#### [ ] Workspace reconciliation
- Ricarica da `workspaces.json` modificato esternamente
- `prune_every_n_ticks`: workspace orfane rimosse dopo N ticks
- Shutdown con subprocessi in-flight → nessun processo zombie

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

### 1e. speedy-core/workspace.rs — cross-process edge case
- File: `packages/speedy-core/tests/workspace_cross_process.rs`
- Recovery da `workspaces.json` corrotto (JSON malformato)
- `prune_missing()` con directory cancellate mid-test
- Rimozione concorrente: 8 processi con operazioni miste add+remove

---

## 2. Feature incomplete o mancanti

### [x] `chunk_count` in WorkspaceStatus è sempre `None`
- File: `packages/speedy-daemon/src/main.rs:1417`
- **Done (2026-05-18)**: aggiunto `rusqlite` al daemon; `handle_workspace_status` apre
  il DB in read-only e popola `chunk_count` con `SELECT COUNT(*) FROM chunks`.

### [ ] Kotlin non supportato nel parser language-context
- File: `packages/speedy-language-context/src/parser/tree_sitter_parser.rs`
- Commento inline: `"kt" | "kts" — tree-sitter-kotlin not yet compatible with tree-sitter 0.23`
- **Azione**: monitorare release di `tree-sitter-kotlin` compatibile con 0.23+

### [ ] Language context: nessuna ricerca semantica (solo BM25)
- File: `packages/speedy-language-context/src/search.rs`
- La ricerca è keyword-only (BM25 approssimato sui nomi simbolo)
- `run_pipeline` in `mcp.rs` non usa embeddings
- **Proposta**: integrare speedy-ai-context per ricerca ibrida (vettori + BM25),
  oppure aggiungere FTS5 su signature/docstring

### [ ] speedy-mcp è un proxy troppo thin
- File: `packages/speedy-mcp/src/main.rs`
- È un proxy JSON-RPC che esegue `speedy-cli` come sottoprocesso
- Non espone i tool di speedy-language-context
- **Proposta**: valutare unificazione dei due MCP server in uno solo,
  o far passare il proxy anche ai tool di speedy-language-context

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

### [ ] Chiarire il ruolo di `testexe`
- Package: `packages/testexe/`
- Contiene due binari: `testexe` e `workspace-fixture`
- Usa `reqwest` + `speedy-core`; sembra infrastruttura di test E2E
- **Azione**: aggiungere un breve commento in `Cargo.toml` o `src/main.rs`,
  oppure rimuovere se obsoleto

---

## 4. Infrastruttura test da costruire

### [x] `MockEmbeddingProvider`
- **Done (2026-05-18)**: aggiunto in `packages/speedy-ai-context/src/embed.rs` (modulo test)
- Vettori deterministici via FNV-1a hash, `set_fail_next`, `set_latency_ms`, tracciamento chiamate
- 6 nuovi test coprono: determinismo, diversità, bounds, fail injection, call tracking

### [ ] Fixture workspace Rust per language-context
- Posizione: `packages/speedy-language-context/tests/fixtures/sample_project/`
- File `.rs` con: funzioni, struct, trait, impl, chiamate cross-file, ciclo A→B→A
- Usato dai test dei tool MCP e dai test del parser/impact
- Sblocca test behavior di `get_skeleton`, `run_pipeline`, cycle detection

---

## 5. GUI — cose rinviate

### [ ] Smoke E2E manuale (richiede macchina fisica)
- Checklist dettagliata in `todo/os/TODO-platform.md`
- Verificare: tray icon, notifiche sistema su errore, tema chiaro/scuro,
  persistenza settings tra riavvii, daemon-exe override, export log

### [ ] macOS: GUI esclusa dal release workflow
- File: `.github/workflows/release.yml`
- Il job macOS builda senza `-p speedy-gui`
- **Azione**: verificare se le dep GUI sono disponibili in CI macOS
  (eframe/winit hanno supporto macOS); se sì, aggiungere

---

## 6. Distribuzione

### [ ] Installer Windows (Inno Setup) — verificare nomi binari
- Verificare che l'installer punti ai nuovi nomi:
  `speedy-ai-context.exe`, `speedy-language-context.exe`, `speedy-cli.exe`
- Allineare `scripts/build-release.{ps1,sh}` se necessario

### [ ] Nessun tag release recente
- Le grandi feature (MCP a due server, GUI, language-context, sqlite-vec)
  non hanno un tag semantico associato
- **Azione**: creare tag `v0.x.0` con changelog (il workflow release si
  attiva su `push: tags: ["v*"]`)

---

## Priorità suggerita

| Priorità | Item |
|----------|------|
| ~~Alta~~ | ~~`chunk_count` in WorkspaceStatus~~ — **done** |
| ~~Alta~~ | ~~`MockEmbeddingProvider`~~ — **done** |
| ~~Alta~~ | ~~Test daemon: loop prevention + reconciliation~~ — **done** (prune-missing) |
| ~~Media~~ | ~~Test daemon_client.rs timeout/protocol~~ — **done** |
| ~~Media~~ | ~~Fix `_indexer` non usato → esporre IndexStats~~ — **done** |
| ~~Media~~ | ~~`CascadeEmbeddingProvider` + test~~ — **done** |
| Alta | Test daemon: loop prevention + reconciliation |
| Media | Test daemon_client.rs timeout/protocol |
| Media | Fixture workspace Rust (sblocca test MCP tools) |
| ~~Media~~ | ~~Rimuovere `winreg` inutilizzato~~ — **già rimosso** |
| Media | Fix `_indexer` non usato → esporre IndexStats |
| Bassa | Kotlin support (dipende da release esterna) |
| Bassa | Ricerca semantica in language-context |
| Bassa | Smoke E2E GUI manuale |
| Bassa | Tag release |
