# Comandi e opzioni di Speedy

> Riferimento completo di **ogni singola opzione** dei 4 eseguibili, ricavato dalle definizioni `clap` reali nel codice (`packages/*/src/{main,cli}.rs`). Quando il README ed il codice divergono, qui vale il codice.

Sommario:

- [`speedy.exe`](#speedyexe-worker) — worker, tutta la logica pesante
- [`speedy-daemon.exe`](#speedy-daemonexe-manager) — daemon centrale unico
- [`speedy-cli.exe`](#speedy-cliexe-thin-client) — thin client che parla col daemon
- [`speedy-mcp.exe`](#speedy-mcpexe-mcp-server) — server MCP per AI agent
- [Variabili d'ambiente comuni](#variabili-dambiente-comuni)
- [Protocollo IPC del daemon](#protocollo-ipc-del-daemon)

---

## `speedy.exe` (worker)

Worker autosufficiente. Può girare standalone (con `SPEEDY_NO_DAEMON=1` o senza daemon attivo) **o** essere spawnato come subprocess dal daemon via comando `exec`. Quando un subcomando "operativo" (`index`, `query`, `context`, `sync`) viene invocato da CLI normale, prima parte un `ensure_daemon()` che si assicura che il daemon sia vivo e che il cwd sia un workspace monitorato.

### Flag globali

| Flag                          | Tipo / Default                | Descrizione                                                                                              |
|-------------------------------|-------------------------------|----------------------------------------------------------------------------------------------------------|
| `--json`                      | bool, off                     | Stampa l'output in JSON (`pretty`). Globale: vale per ogni subcomando.                                  |
| `-p, --path <PATH>`           | path opzionale                | Project root. Se passato, fa `set_current_dir(<PATH>)` prima di eseguire. Default: cwd corrente.        |
| `--daemon-socket <NAME>`      | stringa opzionale             | Nome del local socket del daemon. Default: `speedy-daemon` (override anche via env `SPEEDY_DEFAULT_SOCKET`). |
| `-V, --version`               | —                             | Stampa la versione (da `Cargo.toml`) ed esce.                                                            |
| `-h, --help`                  | —                             | Stampa help ed esce.                                                                                     |

### Flag top-level "shortcut" (alternativi ai subcomandi)

Queste flag permettono di compiere un'azione completa senza usare un subcomando. Vengono valutate **prima** dei subcomandi e fanno `return` non appena eseguite.

| Flag                                   | Alias    | Arg          | Cosa fa                                                                                                                              |
|----------------------------------------|----------|--------------|--------------------------------------------------------------------------------------------------------------------------------------|
| `-r, --read <PROMPT>`                  | —        | string       | Query semantica del workspace con prompt naturale. Ritorna top-5. Onora `--json`.                                                   |
| `-m, --modify <CONTENT>`               | —        | string       | Scrive `<CONTENT>` su `--file <PATH>` (richiesto) e re-indicizza quel file. Senza `--file` stampa solo un suggerimento.            |
| `--file <PATH>`                        | —        | path         | File target per `--modify`. **Richiede** `--modify` (clap `requires = "modify"`).                                                  |
| `-d, --daemons`                        | —        | —            | Lista i workspace tracciati dal daemon (se vivo). Se il daemon è giù, stampa `Daemon not running.`                                  |
| `-w, --workspaces`                     | —        | —            | Lista i workspace registrati in `workspaces.json` (non passa per il daemon).                                                       |

> Nota: `--workspaces`, `--daemons`, e i subcomandi `daemon` / `workspace` **saltano** `ensure_daemon`. Vedi `should_skip_daemon_check` in `packages/speedy/src/main.rs`.

### Subcomandi

#### `index [<SUBDIR>]`
Indicizza una directory nel DB vettoriale del workspace corrente.

| Arg / Flag      | Default | Descrizione                                          |
|-----------------|---------|------------------------------------------------------|
| `subdir`        | `.`     | Sottodirectory da indicizzare, relativa al cwd.     |
| globali         | —       | Onora `--json` (stampa stats `{files, chunks}`).    |

#### `query <QUERY>`
Esegue una ricerca semantica.

| Arg / Flag           | Default | Descrizione                                                                  |
|----------------------|---------|------------------------------------------------------------------------------|
| `query`              | —       | Stringa di ricerca (richiesta).                                              |
| `-k, --top-k <N>`    | `5`     | Numero di risultati da restituire.                                           |
| globali              | —       | Onora `--json`. In testo: stampa `[score=...] <path>:<line>` + snippet.    |

#### `context`
Stampa un riepilogo del workspace: root, file_count, chunk_count, last_indexed. Onora `--json`.

#### `sync`
Sync incrementale dei cambiamenti del filesystem nel DB. Stampa `added/updated/removed`. Onora `--json`.

#### `daemon`
**Senza azione**, `speedy daemon` **spawna il daemon centrale** (`daemon_util::spawn_daemon_process(socket)`) e ritorna subito stampando `Daemon started on socket <socket>`. Non ha sotto-azioni: tutta la gestione del daemon vivo passa per `speedy-cli daemon` (vedi sotto) o per il protocollo IPC diretto.

#### `workspace <ACTION>`

| Sotto-comando | Cosa fa                                            |
|---------------|----------------------------------------------------|
| `list`        | Lista i workspace registrati in `workspaces.json`. |

> Per `add`/`remove` di workspace usa `speedy-cli workspace add/remove <PATH>` — quei comandi parlano direttamente col daemon via IPC.

---

## `speedy-daemon.exe` (manager)

L'unico processo long-running. CLI minimale: tutta la logica di gestione passa per il **protocollo IPC** sul local socket, non per i flag.

### Flag

| Flag                              | Default          | Descrizione                                                                                                                  |
|-----------------------------------|------------------|------------------------------------------------------------------------------------------------------------------------------|
| `--daemon-socket <NAME>`          | `speedy-daemon`  | Nome del local socket su cui ascoltare (Named Pipe Windows / UDS Unix, namespace generico via crate `interprocess`).        |
| `--daemon-dir <DIR>`              | —                | Override della directory dove tiene `daemon.pid` e `workspaces.json`. Se passato, viene propagato anche via `SPEEDY_DAEMON_DIR`. Utile per test e installazioni isolate. |
| `-V, --version` / `-h, --help`    | —                | Standard clap.                                                                                                               |

### Comportamento all'avvio

1. Crea la `daemon_dir` se manca.
2. Chiama `kill_existing_daemon(&daemon_dir)` — taskkill del PID stale se presente.
3. Acquisisce un advisory lock su `daemon.pid` (Windows: `LockFileEx`, Unix: `flock`). Se occupato → errore fatale (un altro daemon è vivo).
4. Scrive `daemon.pid` con il PID corrente.
5. Purga da `workspaces.json` le entry con path inesistente (`workspace::prune_missing`).
6. Per ogni workspace rimasto, lancia un watcher in un thread dedicato.
7. Bind del local socket. Se occupato → errore fatale (no port-fallback con local socket).
8. Loop: `accept()` con timeout 1s + health check ogni 30s.

### Health check dei watcher

- Heartbeat: ogni watcher aggiorna `last_heartbeat` ogni secondo.
- Warn se silente da `2 * 30s = 60s`.
- Restart automatico se silente da `4 * 30s = 120s`.

### Variabili d'ambiente lette dal daemon

| Env                         | Effetto                                                                                                                      |
|-----------------------------|------------------------------------------------------------------------------------------------------------------------------|
| `SPEEDY_DAEMON_DIR`         | Sovrascrive la directory di `daemon.pid` / `workspaces.json`. Impostata anche automaticamente quando si passa `--daemon-dir`. |
| `SPEEDY_WATCH_LOG`          | Test hook: se settata, il watcher **non spawna** `speedy.exe index`, scrive solo il path su quel file. Solo per i test E2E.  |
| `RUST_LOG`                  | Filtro `tracing_subscriber::EnvFilter` (default-env).                                                                         |

---

## `speedy-cli.exe` (thin client)

Client leggerissimo che parla col daemon via local socket. Per i comandi operativi, costruisce una stringa `exec\t<cwd>\t<arg1>\t<arg2>…` e la manda al daemon, che a sua volta spawna `speedy.exe`.

### Flag globali

| Flag                          | Default          | Descrizione                                                                                       |
|-------------------------------|------------------|---------------------------------------------------------------------------------------------------|
| `-p, --path <PATH>`           | cwd              | Project root. Se settato, fa `set_current_dir(<PATH>)` prima di mandare comandi al daemon.       |
| `--daemon-socket <NAME>`      | `speedy-daemon`  | Nome del socket del daemon. (Override anche via `SPEEDY_DEFAULT_SOCKET`.)                         |
| `--json`                      | off              | Globale. Per i comandi operativi, propaga `--json` al worker tramite la stringa `exec`.          |
| `-V, --version` / `-h, --help`| —                | Standard clap.                                                                                    |

### Subcomandi (operativi)

Tutti questi mandano `exec\t<cwd>[\t--json]\t<cmd>\t<args...>` al daemon, che esegue `speedy.exe` e ritorna lo stdout.

| Subcomando                           | Risultato lato CLI                                          |
|--------------------------------------|-------------------------------------------------------------|
| `index [<SUBDIR>]` (default `.`)     | `exec ... index <subdir>` → stampa la risposta del worker. |
| `query <QUERY> [-k <N>]` (default 5) | `exec ... query <q> -k <N>`.                                |
| `context`                            | `exec ... context`.                                         |
| `sync`                               | `exec ... sync`.                                            |
| `force [-p <PATH>]` (default cwd)    | Non passa per `exec`: manda direttamente `sync <PATH>` al daemon (daemon-driven incremental sync). |

### Subcomandi: `daemon`

| Sotto-comando | Cosa fa lato cli                                                                                                |
|---------------|-----------------------------------------------------------------------------------------------------------------|
| `status`      | `client.status()` → stampa PID/uptime/workspaces/watchers/version. Onora `--json` (pretty).                    |
| `list`        | `client.get_all_workspaces()` → stampa `[active] <path>` per ognuno.                                            |
| `stop`        | `client.stop()` → stampa `Daemon stopped.`                                                                       |
| `ping`        | `client.ping()` → stampa `pong`.                                                                                 |

Questi non scatenano `ensure_daemon` (skip): se il daemon è morto, falliscono pulitamente.

### Subcomandi: `workspace`

| Sotto-comando        | Cosa fa lato cli                                                                                          |
|----------------------|-----------------------------------------------------------------------------------------------------------|
| `list`               | Legge **direttamente** `workspaces.json` (non passa per il daemon).                                       |
| `add <PATH>`         | `client.add_workspace(<path>)` — il daemon registra + spawna watcher.                                     |
| `remove <PATH>`      | `client.remove_workspace(<path>)` — il daemon ferma watcher + deregistra.                                 |

> **Differenza importante rispetto a `speedy.exe`**: in `speedy-cli` `add`/`remove` prendono il path come argomento posizionale, non con `-p`.

### Senza argomenti

Se chiamato senza sottocomando, `speedy-cli` esce con `No command specified. Use --help for usage.`

---

## `speedy-mcp.exe` (MCP server)

Server [MCP](https://modelcontextprotocol.io) per AI agent (Claude Code, Cursor, opencode, Windsurf, …). Espone tre tool. Comunica su **stdio** via JSON-RPC, quindi **non ha flag CLI utente** — viene avviato dal client MCP secondo la sua config.

### Tool esposti

| Tool             | Argomenti                                  | Cosa fa                                                                                  |
|------------------|--------------------------------------------|------------------------------------------------------------------------------------------|
| `speedy_query`   | `{ query: string, top_k?: number }`        | Esegue `<SPEEDY_BIN> query <q> -k <top_k> --json`. Default top_k: `SPEEDY_MCP_TOP_K` env o `5`. |
| `speedy_index`   | `{ path?: string }`                        | Esegue `<SPEEDY_BIN> index <path>`. Senza `path`: cwd.                                   |
| `speedy_context` | `{}`                                       | Esegue `<SPEEDY_BIN> context --json`.                                                    |

### Variabili d'ambiente

| Env                | Default          | Descrizione                                                                                                                              |
|--------------------|------------------|------------------------------------------------------------------------------------------------------------------------------------------|
| `SPEEDY_BIN`       | `speedy-cli`     | Binary da invocare per i tool. Default è `speedy-cli` (passa per il daemon); puoi puntarlo a `speedy.exe` per bypass diretto.            |
| `SPEEDY_MCP_TOP_K` | `5`              | Top-K di default per `speedy_query` quando il client non specifica `top_k`.                                                              |
| `RUST_LOG`         | —                | Filtro log `tracing_subscriber`.                                                                                                          |

### Esempio di configurazione client MCP

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-mcp",
      "args": [],
      "env": { "SPEEDY_BIN": "speedy-cli", "SPEEDY_MCP_TOP_K": "10" }
    }
  }
}
```

---

## Variabili d'ambiente comuni

Lette da uno o più binari Speedy. Le env hanno **priorità sopra** `speedy.toml` / `.speedy/config.toml`.

### Comportamento daemon / IPC

| Env                       | Letta da              | Effetto                                                                                                                             |
|---------------------------|-----------------------|-------------------------------------------------------------------------------------------------------------------------------------|
| `SPEEDY_NO_DAEMON`        | `speedy.exe`          | Se settata (qualunque valore), il worker **salta** `ensure_daemon`. Impostata automaticamente dal daemon quando spawna il worker via `exec`. Evita il fork-bomb. |
| `SPEEDY_DEFAULT_SOCKET`   | `speedy-core`         | Sovrascrive il nome di default del socket (`speedy-daemon`).                                                                       |
| `SPEEDY_DAEMON_DIR`       | `speedy-daemon`, `speedy-core::daemon_util`, `workspace` | Override directory per `daemon.pid` e `workspaces.json`. Impostata anche da `--daemon-dir` del daemon.       |
| `SPEEDY_WATCH_LOG`        | `speedy-daemon`       | Test hook: scrive i path osservati invece di spawnare `speedy.exe`. Solo per test E2E.                                              |
| `SPEEDY_BIN`              | `speedy-mcp`          | Binary che l'MCP invoca per implementare i tool. Default `speedy-cli`.                                                              |
| `SPEEDY_MCP_TOP_K`        | `speedy-mcp`          | Top-K di default per `speedy_query`. Default 5.                                                                                     |

### Embedding / config indexer (lette da `speedy.exe`)

| Env                       | Default                       | Descrizione                                                       |
|---------------------------|-------------------------------|-------------------------------------------------------------------|
| `SPEEDY_MODEL`            | `all-minilm:l6-v2`            | Modello di embedding Ollama.                                      |
| `SPEEDY_OLLAMA_URL`       | `http://localhost:11434`      | URL del server Ollama.                                            |
| `SPEEDY_PROVIDER`         | `ollama`                      | Provider embedding (`ollama` o `agent`).                          |
| `SPEEDY_AGENT_COMMAND`    | *(vuoto)*                     | Comando per il provider `agent` (riceve testo, ritorna JSON di float). |
| `SPEEDY_TOP_K`            | `5`                           | Top-K di default per `query` (worker).                            |

### Logging

| Env         | Letta da | Descrizione                                  |
|-------------|----------|----------------------------------------------|
| `RUST_LOG`  | tutti    | Filtro `tracing_subscriber::EnvFilter`.      |

---

## Protocollo IPC del daemon

Riferimento sintetico — dettagli completi in `docs/ipc-protocol.md`.

- **Trasporto**: local socket via `interprocess` (Named Pipe Windows / UDS Unix), namespace generico.
- **Nome default**: `speedy-daemon`. Override con `--daemon-socket`.
- **Wire**: una richiesta per connessione, line-based UTF-8. `\n` chiude la riga.
- **Versionamento**: la risposta `status` include `protocol_version` (intero). Client che non riconoscono la versione devono fallire pulito.

### Comandi accettati

| Richiesta                              | Risposta                                                                                       |
|----------------------------------------|------------------------------------------------------------------------------------------------|
| `ping`                                 | `pong`                                                                                          |
| `status`                               | JSON `{pid, uptime_secs, workspace_count, watcher_count, version, protocol_version}`           |
| `list`                                 | JSON array `["/path/1", "/path/2", ...]`                                                       |
| `watch-count`                          | `N`                                                                                            |
| `daemon-pid`                           | `N`                                                                                            |
| `is-workspace <path>`                  | `true` / `false`                                                                               |
| `add <path>`                           | `ok` / `error: ...` (registra + spawna watcher)                                                 |
| `remove <path>`                        | `ok` / `error: ...` (ferma watcher + deregistra)                                                |
| `sync <path>`                          | `ok` / `error: ...` (spawna `speedy.exe -p <path> sync`)                                        |
| `reload`                               | `ok: N workspaces reloaded` (rilegge `workspaces.json` e sincronizza i watcher)                 |
| `metrics`                              | JSON `{queries, indexes, watcher_events, syncs, exec_calls}` (contatori cumulativi)             |
| `exec <args>` o `exec\t<cwd>\t<args>`  | stdout di `speedy.exe <args>` (env `SPEEDY_NO_DAEMON=1`)                                       |
| `stop`                                 | `ok` poi shutdown graceful                                                                      |
| qualsiasi altro                        | `error: unknown command: <cmd>`                                                                |

### Encoding `exec` con CWD e spazi

Per preservare path con spazi, `exec` accetta la forma tab-separata:

```
exec\t<cwd>\t<arg1>\t<arg2>...
```

- `<cwd>` può essere vuoto (`exec\t\tindex\t.`) → niente `chdir` lato child.
- La forma legacy `exec index .` (whitespace-separated) è ancora accettata per chiamanti che non hanno bisogno di cwd.
- Tutti gli `exec` lanciano `speedy.exe` con `SPEEDY_NO_DAEMON=1` per impedire ricorsione.

---

## Cheat sheet — equivalenze cli ↔ worker

Stesso effetto, due interfacce:

| Operazione                  | Via `speedy-cli.exe`                                | Via `speedy.exe` (standalone)               |
|-----------------------------|------------------------------------------------------|----------------------------------------------|
| Index dir                   | `speedy-cli index src/`                              | `speedy index src/`                          |
| Query                       | `speedy-cli query "auth" -k 10`                      | `speedy query "auth" -k 10`                  |
| Project context             | `speedy-cli context`                                 | `speedy context`                             |
| Sync incrementale           | `speedy-cli sync`                                    | `speedy sync`                                |
| Force daemon-driven sync    | `speedy-cli force -p C:\proj`                        | (non disponibile, usa `speedy sync` con `-p`) |
| Stato del daemon            | `speedy-cli daemon status`                           | (non esposto da speedy.exe)                  |
| Ping del daemon             | `speedy-cli daemon ping`                             | (non esposto)                                |
| Stop del daemon             | `speedy-cli daemon stop`                             | (non esposto)                                |
| Lista workspace (registry)  | `speedy-cli workspace list`                          | `speedy workspace list` o `speedy -w`        |
| Lista workspace (daemon)    | `speedy-cli daemon list`                             | `speedy --daemons` o `speedy -d`             |
| Aggiungi workspace          | `speedy-cli workspace add C:\proj`                   | (non esposto, usa `speedy-cli`)              |
| Rimuovi workspace           | `speedy-cli workspace remove C:\proj`                | (non esposto, usa `speedy-cli`)              |
| Avvia daemon centrale       | (parte automaticamente al primo `ensure_daemon`)     | `speedy daemon`                              |
