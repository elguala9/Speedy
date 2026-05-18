# Test Roadmap — Speedy

## Situazione attuale

| Package | Test esistenti | Copertura stimata |
|---------|---------------|-------------------|
| speedy-mcp | 19 integration + 24 unit (inline) | ~70% — buona |
| speedy-cli | 13 E2E | ~80% (via daemon) — buona |
| speedy-core | 2 cross-process | ~20% — parziale |
| speedy-language-context | 4 (solo protocollo MCP) | ~10% — scarsa |
| speedy-ai-context | 0 | 0% — assente |
| speedy-daemon | 0 diretti | ~50% indiretto — scarsa |
| speedy-gui | 0 | 0% — non prioritario |

---

## Priorità 1 — Logica business critica (rischio danni / corruzione dati)

### [ ] speedy-ai-context: db.rs — schema migration
- **File**: `packages/speedy-ai-context/src/db.rs`
- Testare migrazione da schema BLOB a vec0 virtual table
- Testare che dati pre-esistenti non vengano persi
- Testare comportamento con DB già aggiornato (idempotenza)
- Testare accesso concorrente durante migrazione
- **Infrastruttura**: `tempfile::TempDir` per DB isolato, `criterion` già nel Cargo.toml

### [ ] speedy-ai-context: indexer.rs — reembed on model change
- **File**: `packages/speedy-ai-context/src/indexer.rs`
- Testare che al cambio di modello embedding venga rilevato e segnalato
- Testare che un reindex parziale (interruzione) sia recuperabile
- Testare deduplication via hash: file non modificato non re-embedda
- **Mock**: creare `MockEmbeddingProvider` con vettori deterministici

### [ ] speedy-language-context: impact.rs — cycle detection nel call graph
- **File**: `packages/speedy-language-context/src/impact.rs`
- Testare rilevamento cicli (A→B→C→A) senza stack overflow
- Testare blast radius corretto su grafo aciclico semplice
- Testare funzione con zero caller (root node)
- **Infrastruttura**: `GraphStore` in-memory (`:memory:` SQLite)

### [ ] speedy-daemon: self-write loop prevention
- **File**: `packages/speedy-daemon/src/main.rs`
- Testare che i processi figli vengano avviati con `SPEEDY_NO_DAEMON=1`
- Testare che il watcher ignori `.speedy/` (nessun loop su DB writes)
- Testare che PID tracking in `active_pids` prevenga doppio avvio

---

## Priorità 2 — Concorrenza e affidabilità

### [ ] speedy-core: workspace.rs — expand cross-process tests
- **File**: `packages/speedy-core/tests/workspace_cross_process.rs`
- Aggiungere test: rimozione concorrente (8 processi, remove + add misti)
- Testare recovery da JSON corrotto in `workspaces.json`
- Testare `prune_missing()` con directory cancellate mid-test
- I test cross-process esistenti sono buoni: espanderli, non riscriverli

### [ ] speedy-daemon: workspace reconciliation
- Testare che la modifica esterna di `workspaces.json` venga ricaricata
- Testare `prune_every_n_ticks` (workspace orfane vengono rimosse)
- Testare shutdown con subprocessi in-flight (no orphan processes)

### [ ] speedy-ai-context: embed.rs — provider failures
- **File**: `packages/speedy-ai-context/src/embed.rs`
- Testare HTTP timeout dell'embedding provider
- Testare retry logic (se presente)
- Testare fallback tra provider (se configurato)
- **Mock**: HTTP server locale con `wiremock` o mock del trait direttamente

### [ ] speedy-core: daemon_client.rs — timeout e versione protocollo
- **File**: `packages/speedy-core/src/daemon_client.rs`
- Testare CONNECT_TIMEOUT = 2s (daemon non disponibile)
- Testare CMD_TIMEOUT = 10s (daemon risponde lentamente)
- Testare mismatch di versione protocollo → errore chiaro

---

## Priorità 3 — Correttezza unità minori

### [ ] speedy-ai-context: text.rs — chunking
- **File**: `packages/speedy-ai-context/src/text.rs`
- Edge case: file vuoto, solo whitespace
- Edge case: unicode multibyte (emoji, CJK)
- Edge case: file > MAX_CHUNK_SIZE (split corretto)
- Edge case: nessun newline (testo unico)

### [ ] speedy-ai-context: ignore.rs — pattern matching
- **File**: `packages/speedy-ai-context/src/ignore.rs`
- Testare `.speedyignore` vs `.gitignore` precedenza
- Testare glob pattern negati (`!`)
- Testare directory vs file pattern (`/build` vs `build`)

### [ ] speedy-language-context: indexer.rs — incremental
- **File**: `packages/speedy-language-context/src/indexer.rs`
- Testare che file cancellati vengano rimossi dal grafo
- Testare che file non modificati non vengano re-parsati
- Testare comportamento su estensione non supportata (no crash)

### [ ] speedy-language-context: skeleton.rs — detail levels
- **File**: `packages/speedy-language-context/src/skeleton.rs`
- Testare output `minimal` vs `standard` vs `detailed`
- Testare truncation su simboli molto profondi
- Testare che struttura sia valida per ogni livello

### [ ] speedy-core: types.rs — serialization roundtrip
- **File**: `packages/speedy-core/src/types.rs`
- Testare serde roundtrip per `DaemonStatus`, `WorkspaceStatus`, ecc.
- Verificare compatibilità con versioni precedenti (no breaking changes silenziosi)

---

## Priorità 4 — Language context: tool implementations (MCP)

I test esistenti in `packages/speedy-language-context/tests/mcp_binary_test.rs` verificano
solo il protocollo. Aggiungere test che verifichino il comportamento reale degli strumenti:

### [ ] `index_status` tool — workspace indicizzato vs vuoto
### [ ] `get_skeleton` tool — workspace con simboli Rust reali
### [ ] `run_pipeline` tool — input/output su workspace di test
### [ ] `save_observation` / `search_observations` — persistenza FTS

**Pattern consigliato**: usare `temp_workspace()` già presente nel test file,
aggiungere file `.rs` reali, chiamare gli strumenti dopo avere indicizzato.

---

## Infrastruttura da costruire

### [ ] `MockEmbeddingProvider`
- Localizzazione: `packages/speedy-ai-context/src/embed.rs` (test module) o `tests/common/mod.rs`
- Ritorna vettori deterministici basati sull'hash dell'input
- Supporta iniezione di latenza / errori controllati

### [ ] `InMemoryGraphStore`
- Localizzazione: `packages/speedy-language-context/src/graph/store.rs` (test module)
- SQLite `:memory:` già supportato da rusqlite — basta passare `":memory:"` come path

### [ ] Test workspace Rust di esempio
- Localizzazione: `packages/speedy-language-context/tests/fixtures/sample_project/`
- File `.rs` con funzioni, struct, trait, implementazioni, chiamate cross-file
- Usato da tutti i test di language-context che richiedono parsing reale

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
