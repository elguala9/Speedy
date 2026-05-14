# TODO — Speedy

> Stato al 2026-05-14. Architettura: **un solo daemon globale** + worker `speedy.exe` + thin client `speedy-cli` + server MCP `speedy-mcp`. IPC su **local socket** (`interprocess`). Documenti di riferimento: [`flow.md`](./flow.md), [`commands.md`](./commands.md), [`README.md`](./README.md), [`docs/ipc-protocol.md`](./docs/ipc-protocol.md).

Le sezioni §1-§6 sono state chiuse nel grande refactor 2026-05-14. Vedi `git log` per i dettagli.

## Aperti (out-of-scope per il refactor)

### Test ulteriori
- [ ] **Ciclo completo watcher → index → query** (E2E reale, non hookato via `SPEEDY_WATCH_LOG`): workspace `add` → scrittura file → daemon spawna `speedy.exe index` → `speedy-cli query "<contenuto>"` ritrova il chunk.
- [ ] **`speedy.exe` standalone (no daemon)** con `SPEEDY_NO_DAEMON=1`: `index`, `query`, `context`, `sync` su una tempdir, senza che parta nessun daemon.
- [ ] **`speedy-cli` end-to-end via processo reale**: oggi i test di `speedy-cli` usano in parte mock listener. Aggiungere E2E che spawna il daemon vero e verifica.
- [ ] **Concorrenza `workspaces.json` cross-process**: il test in-process esiste già (`test_concurrent_add_no_corruption`); manca uno scenario subprocess che dimostri il file-lock funziona anche fra processi distinti.
- [ ] **Conflitto sul socket name** fra due daemon che partono in race (oggi coperto solo il listener pre-esistente).
- [ ] **Health check restart**: oggi c'è la logica `WATCHER_DEAD_TICKS` ma nessun test che la triggeri. Servirebbe un mock watcher che smette di battere heartbeat.

### Robustezza
- [ ] **Graceful shutdown con watcher attivi che hanno child in-flight**: oggi `stop_all_watchers` setta il flag stop e `taskkill`-a gli `active_pids`. Verificare corse fra "child sta scrivendo sul DB" e "daemon esce".
- [ ] **`canonicalize` su path Windows con prefisso `\\?\`**: aggiungere test esplicito che path con/senza prefisso UNC matchino.
- [ ] **PID-tracking lato watcher**: oggi è usato solo per `taskkill` allo shutdown. Decidere se rimuoverlo o se serve davvero per la difesa self-write (ignore `.speedy/` è già la difesa primaria).

### Build / release
- [ ] **GitHub Actions release workflow**: build cross-platform (Windows x86_64, Linux x86_64, macOS arm64) → upload binari su Release.
- [ ] **`cargo install speedy-mcp` in clean env**: testare che `SPEEDY_BIN` con default `speedy-cli` faccia trovare il binario tramite `PATH`.

### Nice-to-have / feature work
- [ ] **Benchmark suite**: latency di `query` end-to-end, throughput di `index` su repo da 1k / 10k / 100k file, memoria del daemon con N watcher attivi.
- [ ] **`reload` automatico via watcher su `workspaces.json`**: oggi è comando esplicito. Si potrebbe aggiungere un watcher sul file di registry così modifiche esterne vengono picked up senza chiamare `reload`.
- [ ] **Auto-prune workspace cancellati periodico**: oggi `prune_missing` gira solo all'avvio del daemon. Aggiungere check periodico (es. ogni `HEALTH_TICK_SECS * 10`).
- [ ] **Output `--json` universale**: verificare che ogni subcomando di `speedy-cli` e `speedy.exe` produca JSON valido con `--json`, non testo + JSON parziale.
- [ ] **Search cross-progetto**: query unica su N workspace contemporaneamente. Richiede aggregazione lato daemon.
- [ ] **Re-embedding selettivo**: se cambi `SPEEDY_MODEL`, oggi gli embedding esistenti restano. Aggiungere check di compatibilità + flag `speedy reembed`.
- [ ] **Editor integration**: VSCode extension che parla con `speedy-cli`.
- [ ] **Sync iniziale su `add`**: confermare se il daemon fa `sync_all` subito dopo `add` o lascia il primo sync al watcher.

---

## Stato test suite

| Package           | Test |
|-------------------|------|
| `speedy-core`     | 57 ✅ |
| `speedy`          | 91 ✅ |
| `speedy-daemon`   | 48 ✅ (incluso `should_ignore_watch_path`, `metrics`) |
| `speedy-cli`      | 39 unit + 11 e2e ✅ |
| `speedy-mcp`      | 23 unit + 19 integration ✅ |
| **Totale**        | **288 ✅** |

## Note operative

- Tutti i test passano su Windows (`cargo test --workspace`).
- Test del daemon usano `DAEMON_TEST_LOCK` mutex globale per isolamento.
- Hook test: `SPEEDY_WATCH_LOG` (watcher), `SPEEDY_DAEMON_DIR` (override directory), `SPEEDY_NO_DAEMON` (skip ensure_daemon).
- Quando si modifica il protocollo IPC, **bumpare `PROTOCOL_VERSION`** in `speedy-daemon/src/main.rs` e `SUPPORTED_PROTOCOL_VERSION` in `speedy-core/src/daemon_client.rs`, aggiornare in lockstep: `docs/ipc-protocol.md`, `commands.md` §Protocollo IPC, `flow.md`.
