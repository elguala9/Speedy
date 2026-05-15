# Flow di Speedy — come deve girare

> Documento di verifica della comprensione del progetto. Descrive, passo passo, **cosa fa cosa** e **chi chiama chi** nei vari scenari. Se qualcosa qui è sbagliato, è un punto in cui ho frainteso il progetto.

## Principio cardine

**Un solo daemon. Globale. Per tutto.**

- C'è **un solo** `speedy-daemon.exe` in esecuzione per utente, mai uno per
  workspace. Tutti i workspace dell'utente sono gestiti da quell'unico
  processo, ciascuno con il proprio task watcher interno.
- Il daemon ha una **memoria persistente fissa** su disco
  (`~/.config/speedy/workspaces.json` su Linux/macOS,
  `%APPDATA%\speedy\workspaces.json` su Windows) dove tiene la lista dei
  workspace registrati: path canonico + eventuali metadati per ognuno.
- All'avvio (anche dopo un riavvio del PC) il daemon **rilegge questa
  memoria** e ricostruisce in RAM lo stato: per ogni path ancora esistente
  riavvia un watcher; gli orfani vengono purgati.
- `workspaces.json` è la **fonte di verità**. Lo stato in RAM del daemon è
  uno specchio. CLI / MCP / script esterni non scrivono mai direttamente
  in `workspaces.json` — passano sempre per il daemon (`add` / `remove`),
  che è l'unico autorizzato a mutarlo.

---

## 1. Gli attori (5 .exe + 1 lib)

```
speedy-core (lib)        ← libreria leggera condivisa
                           DaemonClient, workspace registry, config,
                           local-socket helpers, types serde condivisi
                           (DaemonStatus, Metrics, WorkspaceStatus,
                            ScanResult, LogLine), embedding type

speedy.exe (worker)      ← TUTTA la logica pesante inline
                           indexer, query, embedding, SQLite, chunking,
                           hashing, ignore, file filter, watcher reale
                           può girare standalone, oppure essere spawnato
                           come subprocess dal daemon

speedy-daemon.exe        ← UN SOLO processo, globale per l'utente
                           gestisce TUTTI i workspace insieme
                           (mai un daemon-per-workspace)
                           IPC server su local socket "speedy-daemon"
                           N file-watcher (uno per workspace) DENTRO
                           lo stesso processo, come task tokio
                           NON fa embedding/indexing: delega a speedy.exe
                           via subprocess
                           target di deploy: cartella Startup di Windows
                           (parte al login utente)

speedy-cli.exe           ← thin client (solo tokio + serde + clap +
                           interprocess), zero dipendenze pesanti
                           parla con il daemon via local socket
                           se daemon morto → lo spawna (speedy-daemon.exe)

speedy-mcp.exe           ← server MCP (JSON-RPC su stdio) per AI agent
                           usa SPEEDY_BIN (default: speedy-cli) per
                           eseguire i tool → daemon → speedy.exe

speedy-gui.exe           ← desktop GUI (egui + eframe) per gestione manuale
                           usa DaemonClient di speedy-core direttamente
                           via tokio runtime in background, NON passa
                           per speedy-cli. 4 tab: Dashboard / Workspaces /
                           Scan / Logs. Tray icon di sistema.
```

### Dipendenze fra crate

| Binary           | Dipende da                                       |
|------------------|--------------------------------------------------|
| `speedy`         | `speedy-core` + tutta la logica pesante          |
| `speedy-daemon`  | `speedy-core` + tutta la logica pesante          |
| `speedy-cli`     | solo `speedy-core` (DaemonClient + local_sock)   |
| `speedy-mcp`     | solo `speedy-core` (chiama `SPEEDY_BIN`)         |
| `speedy-gui`     | solo `speedy-core` (DaemonClient + types) + egui |

---

## 2. IPC — protocollo

- **Trasporto**: local socket via crate `interprocess`.
  - Windows → Named Pipe `\\.\pipe\speedy-daemon`
  - Unix    → Unix Domain Socket `speedy-daemon` (namespace generico)
- **Nome default**: `speedy-daemon`. Override con `--daemon-socket`.
- **Wire**: una richiesta per connessione, line-based UTF-8.
  - request:  `<cmd>[ args...]\n`
  - response: `<line>\n`
  - il server chiude la connessione dopo la risposta.
- **`exec` con path che contengono spazi** → forma tab-separata:
  ```
  exec\t<cwd>\t<arg1>\t<arg2>...
  ```
  `<cwd>` può essere vuoto. Forma whitespace `exec <args>` ancora accettata per legacy.

### Comandi

| Comando                  | Risposta                                                   | Dispatch lato daemon                            |
|--------------------------|------------------------------------------------------------|-------------------------------------------------|
| `ping`                   | `pong`                                                     | inline                                          |
| `status`                 | JSON `{pid, uptime_secs, workspace_count, watcher_count, version}` | inline                                  |
| `list`                   | JSON `["/path/1", "/path/2"]`                              | inline (dalla mappa watcher)                    |
| `watch-count`            | `N`                                                        | inline                                          |
| `daemon-pid`             | `N`                                                        | inline                                          |
| `is-workspace <path>`    | `true` / `false`                                           | inline                                          |
| `add <path>`             | `ok` / `error: ...`                                        | registra in `workspaces.json` + spawna watcher  |
| `remove <path>`          | `ok` / `error: ...`                                        | abort watcher + deregistra                      |
| `sync <path>`            | `ok` / `error: ...`                                        | spawna `speedy.exe -p <path> sync` (incrementale) |
| `reload`                 | `ok: N workspaces reloaded`                                | rilegge workspaces.json + sync watcher          |
| `exec <args>`            | stdout di `speedy.exe`                                     | spawna `speedy.exe <args>` con `SPEEDY_NO_DAEMON=1` |
| `stop`                   | `ok` (poi shutdown graceful)                               | abort tutti i watcher, esce dal loop accept     |
| qualsiasi altro          | `error: unknown command: <cmd>`                            | —                                               |

Note operative del daemon:
- `accept()` ha timeout 1s → può controllare il flag `running` e uscire pulito entro un tick dopo `stop`.
- `exec` setta `SPEEDY_NO_DAEMON=1` nell'env del child → il worker non rientra mai nel daemon (no fork-bomb).
- All'avvio, le entry in `workspaces.json` con path inesistente vengono purgate.

> Comandi aggiuntivi a supporto della GUI (`metrics`, `scan`, `reindex`,
> `workspace-status`, `tail-log`, `subscribe-log`, `query-all`) sono
> documentati nel dettaglio in [`docs/ipc-protocol.md`](./docs/ipc-protocol.md).
> Tutti one-shot tranne `subscribe-log`, che è long-lived: il daemon manda
> `ok\n` come handshake e poi una `LogLine` JSON per ogni evento finché il
> client non chiude la connessione.

---

## 3. Flusso "primo comando dopo boot del PC"

```
PC riparte
  ~/.config/speedy/workspaces.json  → integro su disco
  ~/.config/speedy/daemon.pid       → stale
  named pipe "speedy-daemon"        → non esiste

$ speedy-cli query "auth flow"
  │
  ├─ DaemonClient::is_alive()
  │    ├─ LocalStream::connect("speedy-daemon")
  │    ├─ write "ping\n" + shutdown
  │    ├─ read_line con timeout 2s
  │    └─ accetta solo se risposta == "pong"   ← evita pipe half-open
  │
  │   → connect fail → false
  │
  ├─ ensure_daemon()
  │    ├─ kill_existing_daemon()  ← rimuove daemon.pid stale,
  │    │                            taskkill PID stale se serve
  │    ├─ spawn speedy-daemon.exe  (CREATE_NO_WINDOW su Windows,
  │    │                            stdout/stderr verso null)
  │    └─ attende che is_alive() diventi true (poll con timeout)
  │
  ├─ daemon.start()
  │    ├─ scrive daemon.pid
  │    ├─ legge workspaces.json
  │    ├─ per ogni ws esistente → spawna watcher (tokio task)
  │    └─ Listener::bind("speedy-daemon"), loop accept()
  │
  ├─ DaemonClient::is_workspace(CWD)?  → false
  ├─ DaemonClient::add_workspace(CWD)
  │    ├─ daemon riceve "add <canonical>"
  │    ├─ workspace::add() su workspaces.json
  │    ├─ spawna watcher
  │    └─ (opzionale) sync_all iniziale via speedy.exe sync
  │
  └─ DaemonClient::cmd("exec\t<CWD>\tquery\tauth flow")
       ├─ daemon spawna: speedy.exe -p <CWD> query "auth flow"
       │                 con SPEEDY_NO_DAEMON=1
       ├─ speedy.exe esegue la query sul DB SQLite
       ├─ stdout torna al daemon
       └─ daemon lo gira al cli → cli lo stampa
```

---

## 4. Flusso "file salvato dall'editor"

```
Utente salva src/lib.rs
  │
  ├─ notify (nel watcher del workspace) genera evento
  │
  ├─ daemon: debounce + filtro ignore (.gitignore + .speedyignore)
  │
  ├─ daemon calcola hash SHA-256 del file
  │    ├─ hash uguale al precedente?  → skip
  │    └─ hash diverso?               → continua
  │
  ├─ PID-check anti-loop:
  │    ├─ il file è stato toccato da un PID presente in active_pids?
  │    └─ (cioè: una nostra scrittura via speedy.exe?)  → skip
  │
  └─ daemon spawna: speedy.exe -p <ws> index ./src/lib.rs
       (SPEEDY_NO_DAEMON=1)
       │
       ├─ inserisce il PID in active_pids
       ├─ aspetta che il child termini (in task tokio)
       └─ rimuove il PID da active_pids
```

### Safety: self-write

```
speedy.exe scrive sul DB (.speedy/index.sqlite)
  → notify nota le modifiche al file DB
  → ma le ignore-rules contengono ".speedy/"  → skip

speedy.exe non scrive nei sorgenti dell'utente → nessun loop possibile
```

Il PID-check serve come secondo livello difensivo, in caso un giorno il worker dovesse riscrivere qualche file.

---

## 5. Flusso "AI Agent via MCP"

```
Claude / altro agent
  │  (stdio JSON-RPC)
  ▼
speedy-mcp.exe
  │  per ogni tool call invoca: SPEEDY_BIN <args>
  │  (default SPEEDY_BIN = speedy-cli.exe)
  ▼
speedy-cli.exe
  │  ensure_daemon() → local socket
  ▼
speedy-daemon.exe
  │  exec <args>  → subprocess
  ▼
speedy.exe
  │  query / index / context / sync su SQLite + Ollama
  ▼
stdout risale fino all'agent come result MCP
```

`SPEEDY_BIN` permette di puntare a `speedy.exe` direttamente (bypass daemon) per scenari batch / test.

---

## 5b. Flusso "GUI desktop (`speedy-gui.exe`)"

A differenza di MCP — che è una pipeline `agent → mcp → cli → daemon → speedy` — la GUI **salta lo scalino `speedy-cli.exe`** e parla al daemon direttamente con `speedy-core::DaemonClient`.

```
Utente lancia speedy-gui.exe
  │
  ├─ main thread: TrayHandle::try_new() (Windows/macOS lo vogliono qui)
  │   └─ eframe::run_native → SpeedyApp::new
  │        ├─ DaemonBridge::new
  │        │   ├─ tokio::runtime::Runtime (multi-thread, 2 worker)
  │        │   └─ Arc<Mutex<DaemonState>>  ← snapshot condivisa
  │        └─ Carica settings da eframe::Storage (tab, tema, socket)
  │
  ├─ A ogni frame (≤500ms, ctx.request_repaint_after):
  │   ├─ App::update clona DaemonState (Vec/HashMap moderati: cheap)
  │   ├─ Le view (Dashboard / Workspaces / Scan / Logs) leggono dalla snapshot
  │   └─ Nessun Mutex held durante il disegno
  │
  └─ Utente clicca "Aggiungi workspace":
       │
       ├─ rfd::FileDialog::pick_folder (file picker nativo)
       │
       ├─ DaemonBridge::add_workspace(path)
       │   ├─ inc_busy()  (mostra spinner in topbar)
       │   ├─ runtime.spawn:
       │   │    ├─ DaemonClient::add_workspace(path)  →  IPC "add <canonical>"
       │   │    └─ scrive il risultato in DaemonState.last_op_result
       │   └─ ritorna SUBITO (UI non blocca)
       │
       └─ Il frame successivo legge la snapshot:
            ├─ se ok → toast verde + refresh lista workspace
            └─ se err → toast rosso con il messaggio del daemon
```

### Log streaming (tab "Logs")

```
LogStreamHandle::start
  ├─ tokio task: DaemonClient::subscribe_log
  │    ├─ apre la pipe, manda "subscribe-log\n", legge "ok\n"
  │    └─ poi legge una LogLine JSON per riga → mpsc::UnboundedSender
  │
  ├─ ring buffer cap 5000 nel main thread (drain del receiver in update())
  │
  └─ Se la pipe muore (daemon riavviato) → riconnessione automatica ogni 2s
```

Filtri (livelli, substring, target, workspace) operano sul buffer in memoria, niente nuovo IPC.

### Differenze chiave vs MCP

- **Niente subprocess**: la GUI non spawna `speedy-cli`/`speedy.exe`. Tutto passa via `DaemonClient` in-process (più veloce, no overhead di fork per ogni click).
- **State condiviso**: la GUI vede metriche + status + workspace status aggregati in una `DaemonState`, e li aggiorna in modo asincrono.
- **Autostart**: gestito a livello OS (cartella Startup su Windows, equivalenti su macOS/Linux). La GUI non scrive nel registro né in LaunchAgents — l'utente posiziona `speedy-daemon.exe` (o un suo shortcut) nella cartella Startup.
- **Tray + notifiche**: `tray-icon` per quick-actions (Open / Restart / Quit), `notify-rust` per popup di sistema sui livelli `error` del log stream (toggle opt-in).

### Quando il daemon è giù

La GUI rileva il fallimento di `ping` e mostra un banner "Avvia daemon"; il click chiama `spawn_daemon_process` (stessa logica di `ensure_daemon` lato cli: cerca `speedy-daemon{EXE_SUFFIX}` accanto al binario GUI, spawn detached, polling `is_alive` con backoff fino a 10s).

---

## 6. Flusso "speedy.exe standalone, no daemon"

```
$ speedy index .
  │
  ├─ should_skip_daemon_check()?  → sì
  │    (subcomandi puntuali tipo index/query/context/sync da CLI diretta,
  │     o env SPEEDY_NO_DAEMON=1, o flag --no-daemon)
  │
  └─ esegue tutto in-process
       ├─ carica Config (env + speedy.toml / .speedy/config.toml)
       ├─ apre SQLite in .speedy/index.sqlite
       ├─ EmbeddingProvider (Ollama o agent)
       ├─ scansione + ignore + chunking + embedding + insert
       └─ termina
```

`speedy.exe` è completamente autosufficiente. Il daemon serve **solo** per:
1. Monitoring continuo (auto-reindex on save)
2. Pre-flight check (indice sempre aggiornato prima di una query)
3. API server per AI / MCP

---

## 7. Comandi CLI — chi li gestisce

| Comando                         | `speedy.exe`           | `speedy-cli.exe`                   |
|---------------------------------|------------------------|------------------------------------|
| `index [<subdir>]`              | esegue inline          | exec → daemon → `speedy.exe index` |
| `query <q>`                     | esegue inline          | exec → daemon → `speedy.exe query` |
| `context`                       | esegue inline          | exec → daemon → `speedy.exe context` |
| `sync`                          | esegue inline          | exec → daemon → `speedy.exe sync`  |
| `reembed`                       | esegue inline          | exec → daemon → `speedy.exe reembed` |
| `force [-p <path>]`             | n/a (rimosso)          | sync → daemon                      |
| `daemon status/ping/stop/list`  | n/a                    | risposta diretta dal daemon        |
| `daemon` (no action)            | avvia il daemon centrale | n/a                              |
| `workspace list`                | n/a (worker: solo `list`) | `add`/`remove`/`list` su daemon |

---

## 8. File su disco — la "memoria fissa" del daemon

```
~/.config/speedy/                      (Windows: %APPDATA%\speedy)
├── workspaces.json     ← MEMORIA PERSISTENTE del daemon globale:
│                         lista di TUTTI i workspace dell'utente
│                         [{ "path": "C:/a/proj1", ... },
│                          { "path": "C:/b/proj2", ... }, ...]
└── daemon.pid          ← PID del daemon corrente (uno solo)

<workspace>/
├── .speedy/
│   ├── index.sqlite    ← vector store di QUESTO workspace
│   └── config.toml     ← opzionale, override config per-workspace
└── .speedyignore       ← opzionale, formato gitignore
```

- **`workspaces.json` è la memoria del daemon**: globale, condivisa fra
  tutti i workspace, sopravvive ai riavvii. Il daemon la legge all'avvio,
  la aggiorna a ogni `add`/`remove`, la usa per ricreare i watcher dopo
  un boot.
- **Un solo `workspaces.json`** per utente — non uno per progetto.
- **`daemon.pid`** serve solo per cleanup di un'istanza morta al boot
  successivo (il nuovo daemon `taskkill`a il PID stale se esiste).
- **`.speedy/index.sqlite`** vive invece **dentro** il singolo workspace:
  ogni progetto ha il suo DB vettoriale locale. Il daemon non centralizza
  i dati indicizzati — centralizza solo l'orchestrazione.
- **Concorrenza su `workspaces.json`** ancora **non** protetta da file-lock
  cross-process (TODO). Comunque solo il daemon ci scrive, quindi in
  pratica il problema si manifesta solo se due daemon partono insieme —
  e quello è già escluso da `kill_existing_daemon()` + check `is_alive()`.

---

## 9. Invarianti che il sistema deve rispettare

1. **Mai due daemon vivi contemporaneamente.** `kill_existing_daemon()` viene chiamato sia dal cli (prima di spawnare) sia dal daemon stesso all'avvio. Se la pipe esiste già con un listener vivo che risponde `pong`, lo spawn viene saltato.
2. **`speedy.exe` spawnato dal daemon ha sempre `SPEEDY_NO_DAEMON=1`** → niente ricorsione.
3. **Watcher e indexer non scrivono nei sorgenti dell'utente.** Solo in `.speedy/`, che è ignorato dal watcher tramite ignore-rules.
4. **`add` è idempotente.** Aggiungere lo stesso workspace due volte non crea due watcher.
5. **`remove` di un workspace inesistente non è un errore fatale**, risponde `ok` (o `error: ...` ma il cli lo tratta come no-op).
6. **Sul boot, le entry in `workspaces.json` con path inesistente vengono purgate** prima di avviare i watcher.
7. **`is_alive()` non si fida del solo connect** → manda `ping` e si aspetta `pong`. Un named pipe half-open non viene scambiato per un daemon vivo.
8. **Port-fallback non c'è più**: con local socket non serve, il nome è risolvibile univocamente per utente/sessione. (Il vecchio fallback TCP 42137→42138 è obsoleto.)

---

## 10. Cosa cambia rispetto a DAEMON-GUARD.md / ARCHITETTURA.md (storico)

- **Trasporto**: TCP `127.0.0.1:42137` → **local socket** (`interprocess`). I documenti vecchi parlano di TCP; il codice attuale (`daemon_client.rs`, `local_sock.rs`) usa local socket. L'API è la stessa, cambia solo il connettore.
- **Niente firewall prompt su Windows** (era il problema di `docs/windows-firewall-tcp.md`, ora rimosso).
- **Niente port fallback** per la stessa ragione.

---

## 11. Punti dove potrei aver capito male — da verificare

- **PID-tracking lato watcher**: il PID-set serve per `taskkill` allo shutdown (`packages/speedy-daemon/src/main.rs`, campo `CentralDaemon.active_pids`). **Decisione 2026-05-14**: si mantiene come *defense-in-depth*. La protezione principale contro self-write resta l'ignore di `.speedy/`, ma `active_pids` permette uno shutdown deterministico (zero indexer orfani) anche se domani il worker dovesse iniziare a scrivere file di stato fuori da `.speedy/`. Il costo è minimo (un `HashSet<u32>` per processo).
- **Sync iniziale su `add` — risolto 2026-05-15**: `handle_add` ora fa fire-and-forget di `handle_sync` solo per i workspace nuovi (esistenti già su disco ma non ancora gestiti). L'awaiter del client torna `ok` subito, lo spawn di `speedy.exe sync` corre in background con `SPEEDY_NO_DAEMON=1`. Override per test: `SPEEDY_SKIP_INITIAL_SYNC=1`.

---

## 12. Auto-reload e periodic prune (aggiunti 2026-05-15)

Il daemon mantiene la coerenza con la fonte di verità (`workspaces.json`) in due modi indipendenti:

1. **File watcher su `workspaces.json`**: `spawn_workspaces_json_watcher` osserva la `daemon_dir` con `notify_debouncer_mini` (debounce 1s). Qualsiasi modifica al file (anche da tool esterni che non passano dal daemon) triggera `reload_from_disk`, che riconcilia in-memory ↔ disk. Quando è il daemon stesso a scrivere `workspaces.json`, l'evento di notify lo fa rientrare nel reload — ma `reload_from_disk` è no-op se gli `HashSet<String>` di disk-paths e in-memory-paths sono uguali.
2. **Tick periodico (`PRUNE_EVERY_N_TICKS = 10`, ≈5 min)**: `prune_and_reconcile` rimuove i watcher i cui path non esistono più su disco e poi chiama `workspace::prune_missing` per allineare anche il file di registro. Cattura il caso "ho cancellato la cartella mentre il daemon era attivo".

---

## 13. Query cross-workspace (aggiunto 2026-05-15, protocol v2)

Comando IPC `query-all\t<top_k>\t<query>` → ritorna JSON-array di hit aggregati. Il daemon fa fan-out parallelo (`tokio::spawn` per ogni workspace registrato) eseguendo `speedy.exe -p <ws> query <q> -k <K> --json` con `SPEEDY_NO_DAEMON=1`, deserializza ciascuna risposta in `Vec<serde_json::Value>`, aggiunge il campo `workspace` a ogni hit, fonde tutto, ordina per score discendente e taglia a top_k.

CLI utente: `speedy-cli query --all <q>` (oppure direttamente via `DaemonClient::query_all`).

Note operative:
- Il fan-out non condivide il file lock di `workspaces.json` (per-workspace `vectors.db` sono indipendenti).
- Se un workspace fallisce (Ollama giù, DB corrotto), restituisce array vuoto e gli altri proseguono.
- `protocol_version` salito a 2; client più vecchi che vanno via `cmd("query-all …")` ricevono `error: unknown command`.

---

## 14. `prune-missing` esplicito (aggiunto 2026-05-15)

Oltre al prune periodico di §12, esiste ora un comando IPC esplicito
`prune-missing` (one-shot) che fa la stessa pulizia *su richiesta*:

- Lato daemon: ferma i watcher per i path non più esistenti, chiama
  `workspace::prune_missing` e ritorna `{"removed": N, "paths": [...]}`.
- Lato client: `DaemonClient::prune_missing() -> Result<Vec<String>>`.
- UI: pulsante "🧹 Pulisci orfani" nella tab Workspaces della GUI.
  Si differenzia dal `Remove` per-riga perché non richiede di sapere il
  path: pulisce tutto ciò che non esiste più senza confermare uno per uno.

`protocol_version` resta 2 — è un comando nuovo, non un'incompatibilità.

---

## 15. GUI: daemon-exe override (aggiunto 2026-05-15)

`spawn_daemon_process` nel `speedy-core` ora ha una variante
`spawn_daemon_process_with(exe, socket)` che accetta un path esplicito.
Esposto in `daemon_util::resolve_daemon_exe()` per UI/diagnostica.

Nella GUI, la Dashboard mostra:

- Path risolto correntemente (override custom o auto-detect).
- Campo testuale + `Sfoglia…` / `Applica` / `Ripristina automatico`.
- "Apri cartella" per saltare al folder che contiene il binario.

L'override è persistito in `eframe::Storage` (campo `daemon_exe_path`).
Quando settato, `bridge.spawn_daemon()` lo usa al posto dell'auto-detect.
Caso d'uso principale: GUI installata in una cartella separata dal daemon
(es. `~/.local/bin/` per GUI e `~/.local/libexec/` per il daemon).

Autostart al login: **rimosso dalla GUI** (commit `c642282`). La GUI non
scrive più nel registro Windows / LaunchAgents / `.desktop`. Per
avviare il daemon all'accesso utente, vedi README — la mossa
consigliata su Windows resta uno shortcut in `shell:startup`.
