# TODO — Speedy

## Architettura

```
speedy-core (lib)            — libreria leggera condivisa
speedy      (bin worker)     — tutti i moduli pesanti inline
speedy-daemon (bin manager)  — CentralDaemon + watchers + IPC
speedy-cli  (bin thin)       — proxy TCP puro
speedy-mcp  (bin mcp)        — server MCP per AI agents
```

> **Binaries**: `dist/speedy.exe`, `dist/speedy-daemon.exe`, `dist/speedy-cli.exe` compilati.

| Package | Test | Copertura |
|---------|------|-----------|
| `speedy-core` | 24 ✅ | config, workspace, embedding, daemon_client (base) |
| `speedy` | 111 ✅ | CLI parsing, text, document, ignore, embed, db, file, hash, daemon, watcher, ensure_daemon |
| `speedy-daemon` | 22 ✅ | exec, reindex, double_add, remove_nonexistent, graceful_shutdown, watcher_lifecycle, PID, status_full, dispatch tests, port_in_use, port_fallback |
| `speedy-cli` | 33 ✅ (24 inline + 9 e2e) | CLI parsing (tutti subcomandi), --json, --path, --daemon-port, ping/pong, status, stop, workspace add/remove, index/query/context/sync via daemon, force reindex |
| `speedy-mcp` | 33 ✅ | JSON-RPC protocol, tool schemas, lifecycle, SPEEDY_BIN env var |

---

## Test da scrivere

### 1. `speedy-cli` — Test mancanti (priorità alta) ✅

- [x] CLI parsing: testare tutti i subcomandi (Index, Query, Context, Sync, Force, Daemon, Workspace)
- [x] `daemon status` → verifica output via mock TCP
- [x] `daemon ping` → "pong" via mock TCP
- [x] `daemon stop` → daemon si ferma
- [x] `workspace add/remove` → integration test via DaemonClient
- [x] `--json` flag su tutti i comandi
- [x] `--path` / `-p` flag globale
- [x] Connessione: daemon non in esecuzione → errore gestito
- [x] Timeout/errore connessione se daemon non parte

### 2. `speedy-daemon` — Test IPC aggiuntivi (priorità media) ✅

- [x] `exec` command: invia comando a `speedy.exe` e ritorna output
- [x] `reindex` command: spawna `speedy.exe sync`
- [x] `exec` con comando inesistente → dispatch_command gestisce
- [x] `exec` con binary `speedy.exe` non trovato → errore
- [x] Doppia `add` dello stesso workspace → idempotente (non crash)
- [x] `remove` di workspace inesistente → ok (gestito)
- [x] Watcher: fermato su `remove`, riavviato su `add`
- [x] PID tracking: active_pids set, cleanup su stop
- [x] Graceful shutdown: `stop` ferma watcher, termina listener
- [x] Porta occupata → errore (test con mock listener concorrente)
- [x] `status` con uptime, versione, conteggi corretti
- [x] Port fallback: porta occupata → daemon si sposta su quella successiva

### 3. `speedy` — Test worker aggiuntivi (priorità media) ✅ / 🟡

- [x] `ensure_daemon()`: daemon già alive → skip spawn (mock TCP server)
- [x] `ensure_daemon()`: daemon morto → spawn success (solo unit test con mock; e2e su Windows bloccato dal firewall, vedi `docs/windows-firewall-tcp.md`)
- [x] `ensure_daemon()`: daemon morto → spawn failure (exe non trovato)
- [x] `ensure_daemon()`: workspace registrato via mock server
- [x] `should_skip_daemon_check()`: tutti i casi (daemon flags, workspace flags, subcomandi)
- [x] `Commands::Daemon { action: None }` → parsing verificato
- [ ] Test `--as-service` (Windows) → skip (richiede ambiente Windows Service)
- [🟡] Index con file non esistenti → testabile ora (exe in `dist/`)
- [🟡] Watch `--detach` → testabile ora (exe in `dist/`)

### 4. End-to-end integration (priorità alta) ✅ / 🟡

- [x] `speedy-cli index .` → daemon esegue e torna output (via `test_index_and_query_via_daemon`)
- [x] `speedy-cli context` → daemon → `speedy.exe context`
- [x] `speedy-cli sync` → daemon → `speedy.exe sync`
- [x] `speedy-cli daemon list` → `speedy-daemon` → workspace registrati
- [x] `speedy-cli workspace add` → `speedy-daemon` avvia watcher
- [x] `speedy-daemon` standalone: start, IPC ping/pong/status/stop
- [x] `speedy-cli daemon stop` → daemon si ferma
- [x] `--json` flag su tutti i comandi
- [x] `speedy-cli force` → force reindex
- [ ] Ciclo completo: workspace add → file change → watcher → speedy.exe index → query trova risultato
- [ ] `speedy` standalone (no daemon): index, query, context, sync
- [ ] Test su Windows: `sc create/delete`, `--as-service`

### 5. MCP — Test aggiuntivi (priorità bassa) ✅

- [x] `SPEEDY_BIN` env var → binary personalizzato
- [x] `SPEEDY_BIN` non trovato → errore chiaro

---

## Roba da fare / miglioramenti

- [x] **Graceful shutdown figli**: `speedy-daemon` ora traccia i PIDs dei figli spawnati (`active_pids`) e li kill su stop
- [ ] **Concorrenza workspace**: file lock su `workspaces.json` per accesso cross-process (fd-lock)
- [ ] **Test watcher reali**: usare `tempfile` + notify events simulati
- [ ] **Health check periodico migliorato**: daemon verifica watcher attivi, restart automatico
- [ ] **Documentazione API IPC**: proto semplice testo su TCP (esiste in commento, va in docs/)
- [ ] **Benchmark**: tempi di indexing, query latency, memoria
- [x] **Logging strutturato**: già tutto su `tracing`, unico `eprintln!` residuo in `testexe`
- [x] **Port fallback**: se porta 42137 occupata, prova 42138, 42139... (implementato + test)
- [x] **Config reload**: comando IPC `reload` per ricaricare workspaces e sync watcher
- [x] **CLI help**: `speedy-cli --help` funziona già (clap derive)
- [x] **speedy-daemon PID**: già gestito da `kill_existing_daemon()` all'avvio

---

## Note

- `speedy-daemon` watcher spawna `speedy.exe index <path>` per ogni file change
- `speedy-cli` manda `exec <cmd>` via TCP → daemon spawna `speedy.exe <cmd>` e ritorna stdout
- `speedy` worker ha `--as-service` legacy per backward compat (per-workspace Windows service)
- Tutti i test devono passare su Windows (`cargo test`) — alcuni usano `#[cfg(windows)]`
- Test del daemon usano `DAEMON_TEST_LOCK` mutex globale per isolamento
- **Binaries compilati in `dist/`**: `speedy.exe`, `speedy-daemon.exe`, `speedy-cli.exe` (manca `speedy-mcp.exe`)
- **Piano di completamento dettagliato**: `docs/COMPLETION-PLAN.md`
