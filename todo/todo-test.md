# Test Roadmap — Speedy

## Situazione attuale

| Package | Test esistenti | Copertura stimata |
|---------|---------------|-------------------|
| speedy-mcp | 39 integration + unit (inline) | ~80% — ottima |
| speedy-cli | 13 E2E | ~80% (via daemon) — buona |
| speedy-core | 4 cross-process + 9 serde unit | ~40% — buona |
| speedy-language-context | 4 protocollo + 7 indexer unit | ~35% — migliorata |
| speedy-ai-context | 23 db + 10 indexer + 6 embed + 12 text + 10 ignore | ~65% — buona |
| speedy-daemon | 5+ diretti | ~60% — migliorata |
| speedy-gui | 0 | 0% — non prioritario |

---

## Priorità 1 — Logica business critica (rischio danni / corruzione dati)

### [x] speedy-ai-context: db.rs — schema migration
- **Done (preesistente, verificato 2026-05-19)**: 13 integration + 7 mock tests.
  Coprono migration BLOB→vec0, idempotenza, insert/retrieve, clear, metadata.

### [x] speedy-ai-context: indexer.rs — reembed on model change
- **Done (preesistente, verificato 2026-05-19)**: 6 integration + 4 mock tests.
  Coprono reembed+model_metadata, embed_cache dedup, content-change, deleted file, sync_all.

### [x] speedy-language-context: impact.rs — cycle detection nel call graph
- **Done (preesistente, verificato 2026-05-19)**: `test_find_impact_cycle_terminates` e altri
  6 test coprono cicli, blast radius, root node, deduplicazione, fan-out.

### [x] speedy-daemon: self-write loop prevention
- **Done (2026-05-19)**: `test_active_pids_no_duplicates`, `test_active_pids_remove_cleans_up`,
  `test_prune_and_reconcile_removes_missing_workspace`,
  `test_prune_and_reconcile_keeps_existing_workspace`,
  `test_stop_all_watchers_sets_stop_flags_and_clears_map`.

---

## Priorità 2 — Concorrenza e affidabilità

### [x] speedy-core: workspace.rs — expand cross-process tests
- **Done (2026-05-19)**:
- `test_workspace_json_recovery_from_malformed`: fixture `list` su JSON corrotto → exit failure (non panic)
- `test_prune_missing_with_deleted_directory`: path reale sopravvive, path ghost rimosso,
  verificato via `workspace-fixture prune` (nuovo subcommand aggiunto al fixture)
- File: `packages/speedy-core/tests/workspace_cross_process.rs`

### [x] speedy-daemon: workspace reconciliation
- **Done (2026-05-19)**: `test_prune_and_reconcile_removes_missing_workspace` e
  `test_stop_all_watchers_sets_stop_flags_and_clears_map` coprono i casi principali.

### [x] speedy-ai-context: embed.rs — provider failures (parziale)
- **Done (2026-05-18)**: `CascadeEmbeddingProvider` con 4 test cascade (first-healthy, fallback, all-fail, empty). `set_latency_ms` testato.
- **Rimane**: HTTP timeout del provider reale (richiederebbe mock-HTTP-server).

### [x] speedy-core: daemon_client.rs — timeout e versione protocollo
- **Done (2026-05-18)**: `CONNECT_TIMEOUT`/`CMD_TIMEOUT` testati; `check_protocol_version()` + 4 test mismatch aggiunti.

---

## Priorità 3 — Correttezza unità minori

### [x] speedy-ai-context: text.rs — chunking
- **Done (preesistente, verificato 2026-05-19)**: 12 test coprono edge case (whitespace,
  unicode CJK, emoji, no punctuation, empty), by_paragraphs e by_sentences.

### [x] speedy-ai-context: ignore.rs — pattern matching
- **Done (preesistente, verificato 2026-05-19)**: 10 test (binary detection, .speedyignore,
  filtered_files con gitignore e subdirectory).

### [x] speedy-language-context: indexer.rs — incremental
- **Done (2026-05-19)**: 7 test aggiunti: skip estensione non supportata (no crash),
  file vuoto `.rs` non crasha, simboli trovati in file reale, lista vuota ok.

### [x] speedy-language-context: skeleton.rs — detail levels
- **Done (preesistente, verificato 2026-05-19)**: 14 test già presenti coprono
  minimal/standard/detailed, from_str, simboli pubblici/privati, line numbers.

### [x] speedy-core: types.rs — serialization roundtrip
- **Done (preesistente, verificato 2026-05-19)**: 9 test (DaemonStatus, Metrics,
  WorkspaceStatus, ScanResult, LogLine con optional fields e legacy fields).

---

## Priorità 4 — Language context: tool implementations (MCP)

I test esistenti in `packages/speedy-language-context/tests/mcp_binary_test.rs` verificano
solo il protocollo. Aggiungere test che verifichino il comportamento reale degli strumenti:

### [x] `index_status` tool — workspace indicizzato vs vuoto
- **Done (2026-05-26)**: `test_index_status_shows_counts_after_reindex` — dopo force_reindex controlla symbols > 0, files > 0, last_indexed != "never".
### [x] `get_skeleton` tool — workspace con simboli Rust reali
- **Done (2026-05-26)**: `test_get_skeleton_after_indexing_returns_symbols` — dopo force_reindex, skeleton di lib.rs contiene "add".
### [x] `run_pipeline` tool — input/output su workspace di test
- **Done (2026-05-26)**: `test_run_pipeline_on_indexed_workspace` — task "add function" su workspace indicizzato ritorna matches + impact non vuoti.
### [x] `save_observation` / `search_observations` — persistenza FTS
- **Done (preesistente, verificato 2026-05-26)**: coperto da `test_save_observation_and_search_returns_result`, `test_search_observations_on_empty_store_returns_ok`, `test_save_multiple_observations_and_search`.

**Pattern consigliato**: usare `temp_workspace()` già presente nel test file,
aggiungere file `.rs` reali, chiamare gli strumenti dopo avere indicizzato.

---

## Infrastruttura da costruire

### [x] `MockEmbeddingProvider`
- **Done (2026-05-18)**: aggiunto in `packages/speedy-ai-context/src/embed.rs` (modulo test). Vettori deterministici via FNV-1a hash, `set_fail_next`, `set_latency_ms`, tracciamento chiamate. 6 test.

### [x] `InMemoryGraphStore`
- **Done (preesistente)**: `GraphStore::open(tempdir)` già usato in tutti i test di impact.rs.
  SQLite `:memory:` disponibile ma non necessario — tempdir è equivalente e già in uso.

### [x] Test workspace Rust di esempio
- **Done (2026-05-19)**: `packages/speedy-language-context/tests/fixtures/sample_project/src/`
  contiene `lib.rs`, `utils.rs`, `models.rs` con funzioni, struct, trait, impl, visibilità mista.

---

## Cosa NON toccare (già buono)

- `packages/speedy-mcp/tests/integration_test.rs` — eccellente, mantenere
- `packages/speedy-mcp/src/main.rs` `#[cfg(test)]` — buono, espandere solo se necessario
- `packages/speedy-cli/tests/e2e_test.rs` — buono, il test watcher pipeline è già il più completo
- `packages/speedy-core/tests/workspace_cross_process.rs` — buono, solo espandere i casi

---

## Pattern da seguire per nuovi test

```rust
// 1. Test unitario con dipendenze controllate
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_something_specific() {
        let dir = TempDir::new().unwrap();
        // setup minimo, test deterministico, nessuna dipendenza esterna
    }
}

// 2. Test di integrazione con binary reale (pattern già usato in speedy-mcp)
// - stage_binary() per evitare lock Windows
// - TempProject / DaemonGuard per isolation
// - quiet_command() per CREATE_NO_WINDOW

// 3. Mai dipendere da Ollama in unit/integration test
//    → MockEmbeddingProvider o skip condizionale
```

## Regole per test affidabili

1. **Nessuna dipendenza esterna** nei test unitari (no Ollama, no rete)
2. **Temp directories** sempre — mai scrivere in paths fissi
3. **Nomi socket unici** per daemon test — usare `uuid` o timestamp
4. **Timeout espliciti** su operazioni async — no test che pendono infiniti
5. **Cleanup garantito** — `Drop` impl o `defer!` per rimozione fixtures
6. **Test deterministici** — seed fissi per PRNG, vettori embedding fissi
7. **Errori leggibili** — `unwrap_or_else(|e| panic!("context: {e}"))` > `.unwrap()`
