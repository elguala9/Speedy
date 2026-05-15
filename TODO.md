# TODO — Speedy

> Stato al 2026-05-15. Architettura: **un solo daemon globale** + worker `speedy.exe` + thin client `speedy-cli` + server MCP `speedy-mcp`. IPC su **local socket** (`interprocess`). Documenti di riferimento: [`flow.md`](./flow.md), [`commands.md`](./commands.md), [`README.md`](./README.md), [`docs/ipc-protocol.md`](./docs/ipc-protocol.md).

Le sezioni §1-§6 sono state chiuse nel grande refactor 2026-05-14. La pulizia degli item §Test ulteriori / §Robustezza / §Build è stata completata il 2026-05-14. Il batch di nice-to-have (sync-on-add, prune periodico, reload watcher, --json universale, search cross-progetto, re-embedding selettivo, benchmark suite) è stato chiuso il 2026-05-15. Vedi `git log` per i dettagli.

## Chiusi 2026-05-15

### Quick wins daemon
- [x] **Sync iniziale su `add`**: `handle_add` ora fa fire-and-forget di `handle_sync` (via `tokio::spawn`) solo per workspace nuovi. `SPEEDY_SKIP_INITIAL_SYNC=1` per disabilitare in test.
- [x] **Auto-prune periodico**: `prune_and_reconcile` gira ogni `PRUNE_EVERY_N_TICKS * HEALTH_TICK_SECS` (≈5 min). Stoppa watcher per path scomparsi e poi chiama `workspace::prune_missing`.
- [x] **Reload automatico su `workspaces.json`**: `spawn_workspaces_json_watcher` osserva la `daemon_dir` con `notify_debouncer_mini` e chiama `reload_from_disk` su qualsiasi modifica al file (1s debounce). Self-write innocuo: `reload_from_disk` è no-op quando in-memory ≡ disk.
- [x] **Output `--json` universale**: copertura completata. `speedy --json` su `workspace list`, `--workspaces`, `--daemons`, `daemon` subcommand. `speedy-cli --json` su `daemon status/list/stop/ping`, `workspace list/add/remove`.

### Feature work
- [x] **Search cross-progetto**: nuovo comando IPC `query-all\t<top_k>\t<query>` (protocol v2). Daemon fa fan-out parallelo con `speedy.exe -p <ws> query <q> -k <K> --json` su ogni workspace registrato, aggrega, ordina per score, taglia a top_k. Ogni risultato porta un campo `workspace`. CLI: `speedy-cli query --all <q>`.
- [x] **Re-embedding selettivo**: tabella `metadata` in `.speedy/vectors.db` con `embedding_model`. All'avvio dell'indexer, mismatch tra modello salvato e configurato → warning. Nuovo comando `speedy reembed` (e proxy in `speedy-cli`) che droppa tutti i chunk e re-indicizza con il modello corrente; aggiorna la metadata su successo.
- [x] **Benchmark suite (criterion)**: refactor `speedy` come lib+bin. Nuovo `packages/speedy/benches/core.rs` con:
  - `chunk_file` su 100/1k/10k linee
  - `similarity_search` con DB precaricato 1k/10k/50k chunk (embeddings d=384 deterministici)
  - `insert_chunks` 100/1k chunk

  Tutte self-contained: niente Ollama, niente daemon. Run: `cargo bench -p speedy`.

## Aperti — out of scope per questa codebase

- [ ] **Editor integration (VSCode extension)**: vive in repo separato (TypeScript). Niente da fare qui finché non si decide se generarla con `@vscode/extension-template` o farla a mano. Schema dei tool MCP già stabile (vedi `speedy-mcp`).

---

## Stato test suite (2026-05-15)

| Package           | Test |
|-------------------|------|
| `speedy-core`     | 57 unit + 2 cross-process ✅ |
| `speedy`          | 78 lib + 15 bin ✅ (incluso metadata roundtrip + clear_all_chunks) |
| `speedy-daemon`   | 57 ✅ (incluso parse_query_all_args ×4 + protocol_version lockstep) |
| `speedy-cli`      | 39 unit + 13 e2e ✅ |
| `speedy-mcp`      | 24 unit + 19 integration ✅ |
| **Totale**        | **304 ✅** |

## Note operative

- Tutti i test passano su Windows (`cargo test --workspace`).
- Test del daemon usano `DAEMON_TEST_LOCK` mutex globale per isolamento.
- Hook test: `SPEEDY_WATCH_LOG` (watcher), `SPEEDY_DAEMON_DIR` (override directory), `SPEEDY_NO_DAEMON` (skip ensure_daemon), `SPEEDY_SKIP_INITIAL_SYNC` (no auto-sync on `add`).
- Test E2E che richiedono Ollama (`test_watcher_index_query_pipeline`, `test_standalone_no_daemon_flag`) auto-skippano se `http://localhost:11434/api/tags` non risponde.
- Cross-process test di `workspaces.json` dipende dal binario `workspace-fixture` (in `packages/testexe`).
- Quando si modifica il protocollo IPC, **bumpare `PROTOCOL_VERSION`** in `speedy-daemon/src/main.rs` e `SUPPORTED_PROTOCOL_VERSION` in `speedy-core/src/daemon_client.rs`, aggiornare in lockstep: `docs/ipc-protocol.md`, `commands.md` §Protocollo IPC, `flow.md`. Esiste già un test (`test_protocol_version_matches_supported`) che fallisce se le due costanti divergono.
- Benchmark: `cargo bench -p speedy` produce report criterion in `target/criterion/`. Test mode rapido: `cargo bench -p speedy --bench core -- --test`.
