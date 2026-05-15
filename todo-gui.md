# Speedy GUI — TODO

GUI desktop per amministrare il `speedy-daemon`: vedere workspace, indicizzare,
aggiungere/rimuovere, scansionare il disco per `.speedy/` orfani, riavviare il
daemon, monitorare stato e log in tempo reale.

**Stato (2026-05-15, sessione 3):** stack = egui Rust. **Tutto** implementato
e test-coperto inclusi i rinvii precedenti: tray icon con menu Show/Restart/Quit,
autostart sistema (HKCU\Run / plist / .desktop), notifiche di sistema su
errori, export log filtrato, viewer storico log da file, mock test backend
GUI (5 nuovi test). Aggiunto anche **alias cargo `build-all`** per buildare
tutti i 5 binari in un colpo solo. Vedi `docs/gui-progress.md` per il
dettaglio completo dell'avanzamento.

---

## 0. Scelta dello stack

**Decisione presa (2026-05-15): egui + eframe (Rust nativo).**

Motivo: l'utente ha scelto stack nativo senza JS. egui riusa `speedy-core`
direttamente (named pipe, tipi IPC), single binary cross-platform, stile
"tool tecnico" coerente con un pannello daemon.

Tauri/Svelte e Flutter scartati. Vedi `docs/gui-progress.md` per i dettagli.

---

## 1. Lavori lato daemon (prerequisiti — vanno fatti PRIMA della GUI)

Lo stato attuale del daemon manca di due cose chiave per supportare bene la
GUI. Vanno chiuse prima.

### 1.1 Logging strutturato su file + streaming IPC

Stato attuale (`packages/speedy-daemon/src/main.rs:981-984`): `tracing_subscriber::fmt()`
scrive solo su stderr, niente file. Quando il daemon è detached i log si perdono.

- [x] Aggiungere sink su file rotante: `tracing-appender` con `RollingFileAppender`
      (rotazione giornaliera) in `<daemon_dir>/logs/daemon.log.YYYY-MM-DD`.
- [x] Formato JSON (una riga per evento) → parsabile dalla GUI.
      `tracing_subscriber::fmt::layer().json()` come secondo layer.
- [x] Mantenere il layer testuale su stderr per debug interattivo.
- [x] Aggiungere span/event per OGNI azione:
  - [x] IPC: connection accepted, response sent (livello debug).
  - [x] Watcher: ogni evento debounced (workspace + path).
  - [x] Sync: start, end, durata in ms, exit code.
  - [x] Index: spawn `speedy index`, durata, exit code (`handle_reindex`).
  - [~] workspaces.json: read/write già coperti dai log esistenti
        (`Auto-reload triggered…`, prune messages); nessun nuovo evento.
  - [x] Heartbeat watcher: già esiste (`Health: N watcher(s) active`).
  - [x] Errori: già coperti, includono il path.
- [x] Nuovo comando IPC **`tail-log [n]`** → ritorna ultime N righe (default 200)
      del file di log corrente come JSON array di `LogLine`.
- [x] Nuovo comando IPC **`subscribe-log`** → connessione long-lived. Daemon
      risponde `ok\n` e poi una riga JSON per evento. Implementato via
      `tokio::sync::broadcast<LogLine>` + custom `BroadcastLayer`.

### 1.2 Comandi IPC aggiuntivi

Lo schema attuale (`docs/ipc-protocol.md`) copre già quasi tutto. Da aggiungere:

- [x] **`scan <root> [--max-depth N]`** → cammina il filesystem da `<root>`,
      restituisce JSON `[{path, registered, last_modified, db_size_bytes}]`
      per ogni cartella che contiene `.speedy/index.sqlite`. `walkdir` con skip
      su `target`, `.git`, `node_modules`, `dist`, ecc. Default `max-depth 8`.
      Wire: `scan\t<root>[\t<max_depth>]`.
- [x] **`reindex <path>`** → spawna `speedy index .` con cwd=path. Incrementa
      `metrics.indexes` e logga durata.
- [x] **`workspace-status <path>`** → JSON `WorkspaceStatus` con `watcher_alive`,
      `last_event_at`, `last_sync_at`, `index_size_bytes`. `chunk_count` rinviato
      (rimane `None` — richiederebbe aprire il DB dal daemon).
- [x] **`restart`** → implementato lato GUI: stop IPC + polling `is_alive`
      (backoff 200ms, max 10s) + `spawn_daemon_process`. Nessuna modifica al
      daemon (preferibile come da TODO).

### 1.3 Versionamento protocollo

- [x] `protocol_version: 2` già emesso da `status` (era stato bumpato il
      2026-05-14 per `query-all`). La GUI confronta con
      `SUPPORTED_PROTOCOL_VERSION` esposto da `speedy-core`.

---

## 2. Struttura del progetto GUI

**Realizzato (egui, no JS):**

```
packages/speedy-gui/
├── Cargo.toml            # dipende da speedy-core, eframe, egui, tokio, rfd
└── src/
    ├── main.rs           # bootstrap eframe + windows_subsystem=windows
    ├── app.rs            # SpeedyApp impl eframe::App, persistenza via Storage
    ├── daemon.rs         # DaemonBridge: tokio rt + Arc<Mutex<DaemonState>>
    ├── log_stream.rs     # LogStreamHandle: ring buffer + auto-reconnect
    └── views/
        ├── mod.rs        # Tab enum (Dashboard, Workspaces, Scan, Logs)
        ├── dashboard.rs  # status + metrics + restart/reload/stop
        ├── workspaces.rs # list + file picker + sync/index + conferma rimozione
        ├── scan.rs       # form + tabella + register-batch
        └── logs.rs       # live tail con filtri (livello, substring, target, workspace)
```

- [x] Crate `packages/speedy-gui` aggiunto al workspace `Cargo.toml` root.

---

## 3. Feature GUI — checklist per fase

### Fase A — MVP (parlare col daemon e mostrare lo stato)

- [x] **Connessione al daemon**
  - [x] All'avvio: ping. Se fallisce, banner "Daemon non in esecuzione" +
        bottone "Avvia daemon" (`spawn_daemon_process`).
  - [x] Auto-reconnect ogni 2s — `refresh_all()` ribadisce `is_alive` ogni 2s.
  - [x] Indicatore di stato in **tray icon** — icona 16x16 RGBA che cambia
        colore (verde = alive, rosso = down). `tray::TrayHandle::set_alive`
        viene chiamato ogni frame da `App::update`.
- [x] **Dashboard**
  - [x] PID, uptime, version, protocol_version, workspace_count, watcher_count.
  - [x] Metrics live (queries, indexes, syncs, watcher_events, exec_calls).
  - [x] Path config dir cliccabile → apre nel file manager nativo
        (Explorer/`open`/`xdg-open`).
- [x] **Lista workspace**
  - [x] Tabella scroll con path, "Open folder", "Rimuovi".
  - [x] Per ogni riga, badge watcher + DB size + "event N ago" + "sync N ago"
        da `workspace-status`.
  - [x] Bottone "Aggiungi workspace" → file picker `rfd` nativo → `add <path>`.
  - [x] Bottone "Rimuovi" con conferma → `remove <path>` (modal dedicata).

### Fase B — Operazioni

- [x] **Indicizzazione manuale**
  - [x] Per ogni workspace, bottoni "Index" (= reindex) e "Sync". Un solo
        bottone "Index" copre sia index che reindex pulito.
  - [x] Spinner globale in topbar mentre IPC è in volo; toast verde/rosso
        a termine con il messaggio.
  - [~] "Pannello operazioni attive" — coperto dal contatore busy in topbar
        (numero di IPC in volo), non da una lista dedicata.
- [x] **Scan workspace orfani**
  - [x] Form: root (default = `dirs::home_dir`), max depth (`DragValue` 1..=20).
  - [x] `scan` → tabella con colonna "Registrato" colorata + checkbox.
  - [x] "Registra selezionati" → `add` in batch; "Seleziona tutti i non
        registrati" come scorciatoia.
  - [x] **Rimuovi `.speedy/`**: NON implementato — per design solo unregister
        (decisione esplicita 2026-05-15, l'utente cancella a mano via Explorer).
- [x] **Controllo daemon**
  - [x] "Restart daemon" — `stop` → polling `is_alive` con backoff →
        `spawn_daemon_process`, tutto sul runtime tokio in background.
  - [x] "Reload workspaces" → `reload`.
  - [x] "Stop daemon" (con etichetta rossa di avviso, niente modal — è già
        ovvio dall'azione).

### Fase C — Log viewer

- [x] **Live log**
  - [x] `subscribe-log` apre la connessione al primo render della view; ring
        buffer cap 5000 righe per non far esplodere la RAM. Auto-reconnect
        ogni 2s se la pipe cade (utile se l'utente riavvia il daemon).
  - [~] "Virtual list" non vera; uso `egui::ScrollArea` standard. 5000 righe
        sono gestite comodamente; per 100k+ si dovrebbe passare a una vera
        virtual list (rinviato — non urgente con il cap).
  - [x] Colorazione per livello (5 colori distinti).
  - [x] Filtri: livello (5 checkbox), substring, target (substring),
        workspace (substring sul field `workspace`).
  - [x] Toggle "follow tail" (egui `ScrollArea::stick_to_bottom`).
  - [x] Bottone "Esporta selezione (N)" — salva i log filtrati come JSON
        (array) o JSONL via `rfd::FileDialog::save_file`.
- [x] **Storico**
  - [x] Drop-down con la lista dei file in `<daemon_dir>/logs/` (ordine
        decrescente, più recente in alto). Bottone "Rilegge elenco".
  - [x] Apertura di un file storico in modalità read-only: la view riusa
        gli stessi filtri (livello/substring/target/workspace) sui log
        caricati dal file. Parser tollera sia lo shape `LogLine` (IPC)
        che lo shape `tracing_subscriber::fmt::layer().json()`.

### Fase D — Polish

- [x] Tray icon con menu: Open / Status / Restart daemon / Quit. Implementato
      con `tray-icon = "0.19"`. Voce "Daemon: ● alive/down" disabilitata
      che funge da status read-only. Le azioni vengono drenate da
      `App::update` via `MenuEvent::receiver().try_recv()`.
- [x] Notifiche di sistema su eventi `error`. Toggle in Dashboard
      ("Notifiche di sistema su errore"), persistito. Quando attivo, ogni
      nuova riga di livello `error` arrivata sullo stream live produce
      una notifica via `notify-rust`.
- [x] Tema chiaro/scuro (toggle nel topbar, default dark, persistito).
- [x] Auto-start del daemon al login (HKCU\Run / plist / `.desktop`).
      Modulo `autostart.rs` cross-platform; checkbox in Dashboard
      ("Avvia daemon al login utente") con feedback toast.
- [x] Settings persistenti via `eframe::Storage`: socket name, dark mode,
      tab selezionato. (Livello log default e daemon dir non ancora esposti
      come setting UI — vivono in env var `SPEEDY_DAEMON_DIR` /
      `SPEEDY_DEFAULT_SOCKET`.)

---

## 4. Modello dati condiviso

In `speedy-core` aggiungere (o esporre se già presenti) i tipi serde:

- [x] `DaemonStatus { pid, uptime_secs, workspace_count, watcher_count,
                       version, protocol_version }`
- [x] `Metrics { queries, indexes, syncs, watcher_events, exec_calls }`
- [x] `WorkspaceStatus { path, watcher_alive, last_event_at, last_sync_at,
                          index_size_bytes, chunk_count }`
- [x] `ScanResult { path, registered, last_modified, db_size_bytes }`
- [x] `LogLine { ts, level, target, message, fields: Map<String, Value> }`

Tutti in `packages/speedy-core/src/types.rs`, re-export da `lib.rs`. Il
`DaemonStatus` esistente in `daemon_client.rs` è ora `pub use` di quello in
`types`, niente duplicazione.

---

## 5. Test

- [x] Test integrazione daemon per i nuovi comandi IPC: 8 nuovi
      `#[tokio::test]` in `speedy-daemon/src/main.rs#tests` —
      `workspace-status` (path noto + sconosciuto), `scan` (hit + miss),
      `reindex` (path mancante), `tail-log` (vuoto + parsing JSON+junk),
      `subscribe-log` (`stream_log` handshake + forward via duplex pipe).
      Tutti verdi (65/65 daemon).
- [x] Test backend GUI con mock `DaemonClient`. 5 `#[test]` in
      `speedy-gui/src/daemon.rs#tests`:
      - `daemon_state_toast_helper_round_trips`
      - `bridge_against_dead_socket_marks_probed_not_alive`
      - `bridge_against_mock_marks_alive_and_loads_status_and_metrics`
        (fake listener su runtime separato risponde a ping/status/metrics/list)
      - `busy_counter_settles_after_multiple_overlapping_calls`
      - `workspace_status_error_on_dead_socket_surfaces_in_last_error`
- [ ] Smoke E2E manuale. La GUI builda in release; il lancio interattivo
      è da fare a mano (`cargo run --release -p speedy-gui`).

---

## 6. Domande aperte / da decidere

- [x] **Frontend framework**: né Svelte né React — **egui** (Rust nativo).
- [x] **Rimozione `.speedy/` dalla GUI**: **solo unregister**. La cancellazione
      del DB resta manuale via Explorer.
- [x] **Log JSON vs testo**: **solo JSON** su file. Stderr testuale per debug
      interattivo. Niente file di testo parallelo.
- [x] **Auth/multi-utente**: nessun cambiamento. Documentato nel
      `docs/gui-progress.md` (sez. 2026-05-15 parte 3): la GUI è single-user
      per design — il daemon gira sotto l'utente loggato, gli auto-start
      sono per-utente (HKCU / LaunchAgents user / `.config/autostart`).

---

## 7. Comando build unificato

- [x] **Alias cargo `build-all`** in `.cargo/config.toml`:
      ```
      cargo build-all
      ```
      Builda i 5 binari release (`speedy`, `speedy-daemon`, `speedy-cli`,
      `speedy-mcp`, `speedy-gui`). Gli script `scripts/build-release.{ps1,sh}`
      restano per chi vuole anche il copy in `dist/`.

---

## Riferimenti veloci nel codice

- IPC handler: `packages/speedy-daemon/src/main.rs` (cercare `handle_command`).
- Registry workspace: `packages/speedy-core/src/workspace.rs`.
- DaemonClient: `packages/speedy-core/src/` (cercare `DaemonClient`).
- Path config dir: `packages/speedy-core/src/daemon_util.rs:9-19`.
- Protocollo: `docs/ipc-protocol.md`.
