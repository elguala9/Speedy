# GUI Project — Log di avanzamento

Working log per `todo-gui.md`. Una sezione per macro-attività completata.

---

## 2026-05-15 — §1 daemon prerequisites + §4 tipi serde

Tutti i prerequisiti del daemon descritti in `todo-gui.md` §1.1–§1.3 e §4 sono
implementati e compilano. I 57 test del daemon e i 57 del core restano verdi.

### Logging strutturato (§1.1)

- File rotanti giornalieri in `<daemon_dir>/logs/daemon.log.YYYY-MM-DD` via
  `tracing-appender::rolling::daily` con writer non-blocking. Il guard è
  intenzionalmente leakato (`Box::leak`) così il thread di scrittura resta
  attivo per tutta la durata del processo, niente perdite a shutdown.
- Layer JSON sul file (`tracing_subscriber::fmt::layer().json()`), layer
  testuale su stderr per debug interattivo.
- `BroadcastLayer` custom (`tokio::sync::broadcast<LogLine>`, capacità 1024)
  che alimenta tutti i `subscribe-log` attivi. Un `FieldVisitor` impl
  `tracing::field::Visit` estrae il `message` separato dai field extra.
- Aggiunte tracce `target: "watcher"`, `target: "sync"`, `target: "index"`,
  `target: "ipc"` con campi strutturati (`workspace`, `ms`, ecc.). I `error!`
  esistenti già contenevano il path.

### Nuovi comandi IPC (§1.2)

- `tail-log [n]` → JSON array di `LogLine` (default 200). Trova il file di
  log più recente in `<daemon_dir>/logs/` e parsa le ultime N righe.
- `subscribe-log` → long-lived. Il daemon risponde `ok\n` e poi una riga
  JSON per evento finché il client non chiude. `handle_connection` ora
  riconosce esplicitamente questo comando come l'unico non-one-shot.
- `scan\t<root>[\t<max_depth>]` → `walkdir` con skip su `target`, `.git`,
  `node_modules`, `dist`, ecc. Per ogni dir che contiene
  `.speedy/index.sqlite` ritorna `ScanResult`.
- `reindex <path>` → spawna `speedy index .` con `cwd=<path>` e
  `SPEEDY_NO_DAEMON=1`. Incrementa `metrics.indexes`.
- `workspace-status <path>` → `WorkspaceStatus` con `watcher_alive`,
  `last_event_at`, `last_sync_at`, `index_size_bytes`. `chunk_count` è
  `None` per ora (richiederebbe aprire il DB da qui — rimandato).

### Tipi serde condivisi (§4)

Nuovo modulo `speedy-core/src/types.rs` con `DaemonStatus`, `Metrics`,
`WorkspaceStatus`, `ScanResult`, `LogLine`. `daemon_client::DaemonStatus`
è ora `pub use` dei tipi in `types`, niente duplicazione di shape.

### Protocol version (§1.3)

Era già a 2 (bumpato per `query-all` il 2026-05-14). Nessun ulteriore lavoro.

### DaemonClient

Nuovi metodi: `reload`, `scan`, `reindex`, `workspace_status`, `tail_log`,
`subscribe_log`. Quest'ultimo ritorna `(UnboundedReceiver<LogLine>, JoinHandle<()>)`
— droppare il receiver chiude il task background.

### File toccati

- `packages/speedy-core/Cargo.toml` — aggiunto `walkdir`.
- `packages/speedy-core/src/lib.rs` — espone `types`.
- `packages/speedy-core/src/types.rs` — **nuovo**.
- `packages/speedy-core/src/daemon_client.rs` — usa types, +6 metodi.
- `packages/speedy-daemon/Cargo.toml` — `tracing-subscriber[json]`,
  `tracing-appender`, `walkdir`, `chrono`.
- `packages/speedy-daemon/src/main.rs` — `BroadcastLayer`, `FieldVisitor`,
  `WatcherHandle` arricchito con `last_event_at`/`last_sync_at`, nuovi
  comandi in `dispatch_command`, `handle_connection` con branch
  `subscribe-log`, `main()` reinizializza tracing.
- `docs/ipc-protocol.md` — documentati i 6 comandi nuovi.

### Cosa NON è stato fatto in questa tornata

- Comando `restart`: il TODO suggerisce di lasciarlo alla GUI (stop + spawn).
  Concordo, non lo aggiungo lato daemon.
- `chunk_count` in `WorkspaceStatus` resta `None`.
- Test integrazione per i nuovi comandi (§5) — passano cargo check, ma non
  ci sono asserzioni dedicate ancora.

### Prossimo step

§2 scaffolding crate `speedy-gui` con `egui` + `eframe`, e poi Fase A MVP.

---

## 2026-05-15 (parte 2) — §2 GUI + Fase A/B/C/D minima + §5 test

Crate `speedy-gui` creato e builda in debug e release. La GUI copre tutte e
quattro le fasi del TODO con un set di feature funzionali; lo "showtime"
visivo (animazioni, layout raffinato) è da rifinire ma il funzionale c'è.

### Scaffolding (§2)

Nuovo crate `packages/speedy-gui/` aggiunto al workspace. Dipendenze:
`eframe` + `egui` 0.28, `tokio` (multi-thread runtime in background),
`rfd` per il file picker nativo, `tracing`, `dirs`, `chrono`, `serde`.

Layout finale:

```
packages/speedy-gui/
├── Cargo.toml
└── src/
    ├── main.rs           # bootstrap eframe, windows_subsystem=windows in release
    ├── app.rs            # SpeedyApp impl eframe::App, persistenza via Storage
    ├── daemon.rs         # DaemonBridge: tokio rt + Arc<Mutex<DaemonState>>
    ├── log_stream.rs     # LogStreamHandle con ring buffer (5000 righe)
    └── views/
        ├── mod.rs        # Tab enum
        ├── dashboard.rs  # status + metrics + restart/reload/stop
        ├── workspaces.rs # list + add (file picker) + sync/index + conferma rimozione
        ├── scan.rs       # form + tabella + register-batch
        └── logs.rs       # live tail con filtri (livello, substring, target, workspace)
```

### Architettura sync ⇄ async

egui è immediate-mode quindi il main loop non può bloccarsi su IPC. Soluzione:

- `DaemonBridge` possiede un `tokio::runtime::Runtime` multi-thread (2 worker)
  e un `Arc<Mutex<DaemonState>>` condiviso.
- Ogni metodo pubblico (`refresh_all`, `add_workspace`, `sync_workspace`,
  `scan`, …) è sync, fa `inc_busy()`, spawna una task sul runtime, e
  scrive il risultato nello state quando la task completa.
- `App::update()` clona lo state ad ogni frame (è un `Clone` cheap di
  Vec/HashMap moderati), poi le view leggono dalla snapshot — niente
  Mutex held mentre si disegna.
- `ctx.request_repaint_after(500ms)` garantisce che la UI rifletta gli
  aggiornamenti background anche quando il mouse non si muove.

### Fase A (MVP)

- **Topbar** con nome app + tabs + indicatore daemon (verde/rosso/probing) +
  spinner quando ci sono IPC in volo + toggle tema chiaro/scuro.
- **Banner "Avvia daemon"** quando il ping fallisce (richiama
  `spawn_daemon_process`).
- **Dashboard**: PID, uptime, version, protocol_version, workspace_count,
  watcher_count, metrics cumulativi, link cliccabile alla config dir
  (apre Explorer su Windows, `open` su macOS, `xdg-open` su Linux).
- **Workspaces**: tabella scroll, badge stato watcher, DB size, "event N
  ago" e "sync N ago" da `workspace-status`, bottoni Index/Sync/Open
  folder/Rimuovi (con conferma e nota che il DB on-disk non viene toccato).
- **File picker** nativo via `rfd::FileDialog` per "Aggiungi workspace".

### Fase B (Operazioni)

- Index/Sync per workspace — con toast verde/rosso al termine.
- **Scan**: form con root path + max depth (`DragValue` 1..=20), bottone
  Scansiona → tabella risultati con colonna "Registrato" colorata, checkbox
  selezione (disabilitata per quelli già registrati), "Registra selezionati"
  in batch. NESSUNA opzione di cancellare `.speedy/` sul disco — solo
  unregister, come deciso.
- **Restart**: stop IPC → polling `is_alive` con backoff 200ms (max 10s)
  → spawn detached del binario daemon. Tutto in background, UI responsiva.
- **Reload** e **Stop daemon** con conferma colorata.

### Fase C (Log viewer)

- `LogStreamHandle` connesso a `subscribe-log` IPC. Ring buffer cap 5000
  per non far esplodere la RAM. Riconnessione automatica ogni 2s se la
  pipe muore (es. daemon riavviato).
- Filtri: livelli (5 checkbox), substring case-insensitive, target
  (anche substring), workspace (legge il field `workspace` del LogLine).
- Follow tail toggle (egui ScrollArea::stick_to_bottom).
- Colorazione per livello (error rosso, warn arancio, info azzurro,
  debug/trace grigi).
- Bottoni "Pulisci buffer" e "Restart stream".

### Fase D (Polish minima)

- Toggle tema chiaro/scuro in topbar.
- Settings persistenti via `eframe::Storage`: tab selezionato, dark mode,
  socket name.
- Statusbar in basso con toast (6s di vita) e socket name corrente.

**Rinviati:** tray icon (richiede integrazione con `tray-icon` crate + un
event loop separato; non banale con winit/eframe — vale un round dedicato),
autostart sistema (HKCU\Run su Windows), notifiche di sistema su error.

### Test (§5)

8 nuovi test in `speedy-daemon/src/main.rs` (modulo `tests`):

- `test_workspace_status_unknown_path_errors` — canonicalize fail → `error:`
- `test_workspace_status_known_path_reports_no_watcher` — JSON con
  `watcher_alive=false` e `index_size_bytes=0`
- `test_scan_finds_directory_with_index_sqlite` — crea
  `<root>/proj-a/.speedy/index.sqlite`, verifica che `scan` lo trovi
- `test_scan_missing_root_returns_empty_array` — `walkdir` su path
  inesistente → `[]`
- `test_reindex_missing_path_errors` — canonicalize fail su path mancante
- `test_tail_log_returns_empty_when_no_logs` — directory `logs/` vuota
- `test_tail_log_parses_json_lines` — mix di righe JSON e junk; le junk
  sono saltate, le JSON parsate correttamente
- `test_stream_log_handshake_and_forward` — usa `tokio::io::duplex` come
  fake socket, verifica `ok\n` handshake + serializzazione JSON LogLine

Tutti 65 test del daemon passano (57 esistenti + 8 nuovi). I 57 test
di `speedy-core` restano verdi. Workspace `cargo check --all-targets`
verde, build release verde.

### File toccati in questa tornata

- `Cargo.toml` (root) — aggiunto `packages/speedy-gui` ai membri.
- `packages/speedy-gui/Cargo.toml` — **nuovo**.
- `packages/speedy-gui/src/{main.rs, app.rs, daemon.rs, log_stream.rs,
  views/{mod.rs, dashboard.rs, workspaces.rs, scan.rs, logs.rs}}` — **nuovi**.
- `packages/speedy-daemon/src/main.rs` — 8 nuovi `#[tokio::test]` in `tests`.

### Cosa NON è stato fatto

- Tray icon di sistema (`tray-icon` crate).
- Auto-start del daemon al login utente (HKCU\Run su Windows, plist su
  macOS, `.desktop` autostart su Linux).
- Notifiche di sistema su evento `error` (configurabile, off default).
- Export selezione log a file `.log`/`.json` (basta avere il buffer; il
  bottone è da aggiungere — 5 righe di `rfd::FileDialog::save_file` +
  `serde_json::to_writer`).
- Storico log: drop-down con i file `daemon.log.*` in `<daemon_dir>/logs/`
  e una view read-only sul file selezionato.
- Test backend GUI con mock `DaemonClient` (la struct `DaemonBridge` non
  ha unit test dedicati; copertura è indiretta via test daemon).
- Smoke E2E manuale: il binario builda ma non è stato lanciato (sessione
  non-interattiva).

### Come provarla

```powershell
cargo run --release -p speedy-gui
```

Se il daemon non sta girando vedrai il banner con "Avvia daemon".

### Stato finale task TODO

- [x] §1.1 logging strutturato + streaming IPC
- [x] §1.2 nuovi comandi (scan, reindex, workspace-status, tail-log, subscribe-log)
- [x] §1.3 protocol_version=2 (era già OK)
- [x] §2 scaffolding crate speedy-gui (egui)
- [x] §4 tipi serde condivisi in speedy-core
- [x] Fase A MVP (connessione/dashboard/workspace)
- [x] Fase B operazioni (index/sync/scan/restart)
- [x] Fase C log viewer
- [~] Fase D polish — tema + persistenza fatti; tray/autostart/notifiche rinviati
- [~] §5 test — 8 integrazione daemon OK; mock GUI rinviato

---

## 2026-05-15 (parte 3) — chiusura rinvii + cargo build-all

L'utente ha chiesto di completare tutti i rinvii ancora aperti e di aggiungere
un comando unificato per buildare tutti i binari. Tutto fatto in una sessione.

### Cargo alias `build-all`

Nuovo file `.cargo/config.toml`:
```
[alias]
build-all = "build --release -p speedy -p speedy-daemon -p speedy-cli -p speedy-mcp -p speedy-gui"
```

Verifica: `cargo build-all` → finisce in ~45s e produce i 5 .exe in
`target/release/`. Gli script `scripts/build-release.{ps1,sh}` restano la
soluzione "tutto compreso" (build + copy in `dist/`).

### Tray icon (`tray.rs`, nuovo)

`tray-icon = "0.19"` con icona 16x16 RGBA generata in codice (disco verde
quando il daemon è alive, rosso quando è down — niente file binari embedded).
Menu: voce di status read-only ("Daemon: ● alive/down"), separator, "Open
Speedy", "Restart daemon", separator, "Quit".

Le azioni vengono drenate ad ogni frame da `App::update` via
`MenuEvent::receiver().try_recv()` (non bloccante). `TrayHandle::set_alive`
aggiorna icona + label solo quando lo stato cambia (atomico).

`TrayHandle::try_new` torna `None` se la piattaforma non supporta il tray
(tipicamente Linux senza AppIndicator). L'app continua a funzionare senza.
La handle vive in `Arc<TrayHandle>` ed è creata sul main thread *prima* di
`eframe::run_native`, requisito Windows/macOS.

### Notifiche di sistema su error (`notify-rust = "4.11"`)

Toggle "Notifiche di sistema su errore" nella Dashboard, persistito via
`eframe::Storage`. Quando attivo, `App::notify_new_errors` scorre solo le
righe nuove dello stream live (delta vs `last_notified_log_count`,
clampato all'effettivo ring buffer) e per ogni livello `error` chiama
`notify_rust::Notification::new().summary(...).body(...).show()`.

### Auto-start daemon al login (`autostart.rs`, nuovo)

Modulo cross-platform con tre cfg-branch:
- **Windows**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` via
  `winreg = "0.52"`. Valore = path al `speedy-daemon.exe` *quoted*
  (so spaces in Program Files non rompono).
- **macOS**: `~/Library/LaunchAgents/com.speedy.daemon.plist` con
  `RunAtLoad=true, KeepAlive=false`.
- **Linux**: `~/.config/autostart/speedy-daemon.desktop` con
  `X-GNOME-Autostart-enabled=true`.

API: `is_enabled() -> Result<bool>`, `enable()`, `disable()`. Trovata
dell'eseguibile cerca `speedy-daemon{EXE_SUFFIX}` accanto al binario GUI.

UI: checkbox in Dashboard ("Avvia daemon al login utente"). Toggle del
checkbox chiama enable/disable e mostra toast (verde su success, rosso
su error).

### Export log + storico log (`views/logs.rs`, rewritten)

Aggiunta una selezione di **sorgente** in cima alla view:
- "Live (stream)" — comportamento precedente, `subscribe-log` IPC.
- ComboBox con i file `daemon.log.*` trovati in `<daemon_dir>/logs/`
  (ordinati per nome decrescente, più recente primo).

Quando l'utente seleziona un file diverso, viene caricato e parsato
una volta sola (cache `history_loaded_path`). Il parser è tollerante:
prova prima lo shape `LogLine` (IPC) e poi proietta lo shape del
`tracing_subscriber::fmt::layer().json()` (top-level `timestamp`, `level`,
`target`, `fields.message`, altri `fields.*`).

Bottone "Esporta selezione (N)" sempre disponibile: usa
`rfd::FileDialog::save_file()` con filtri JSON/JSONL. Estensione `.jsonl`
→ una riga per record (newline-delimited); altrimenti array JSON
pretty-printed. Mostra toast con il path su success.

### Test backend GUI (§5, completato)

5 nuovi `#[test]` in `speedy-gui/src/daemon.rs#tests`:

- `daemon_state_toast_helper_round_trips` — `set_toast` mette message/ok
  correttamente in `state.toast`.
- `bridge_against_dead_socket_marks_probed_not_alive` — refresh contro
  socket inesistente, polling fino a `busy==0`; verifica
  `alive=false, probed=true, status=None`.
- `bridge_against_mock_marks_alive_and_loads_status_and_metrics` —
  fake listener su runtime separato risponde a ping/status/metrics/list;
  bridge dopo `refresh_all` ha `alive=true, status.pid==42,
  status.protocol_version==2, metrics=Some(...)`.
- `busy_counter_settles_after_multiple_overlapping_calls` — 5 refresh
  consecutivi contro socket dead, busy deve tornare a 0 (no underflow).
- `workspace_status_error_on_dead_socket_surfaces_in_last_error` —
  `refresh_workspace_status` contro socket dead → `state.last_error`
  contiene "workspace-status".

Pattern del mock: come in `daemon_client::tests`, ma DaemonBridge ha già
il suo runtime tokio, quindi il mock vive in un *runtime separato* (
`tokio::runtime::Runtime::new()` locale al test). Dopo l'asserzione, drop
del runtime libera il socket OS.

### Dipendenze nuove (speedy-gui)

```toml
tray-icon = "0.19"
notify-rust = "4.11"

[target.'cfg(windows)'.dependencies]
winreg = "0.52"
```

### File toccati / nuovi

- `.cargo/config.toml` — **nuovo** (alias build-all).
- `packages/speedy-gui/Cargo.toml` — +3 deps (tray, notify, winreg).
- `packages/speedy-gui/src/main.rs` — crea `TrayHandle::try_new()` prima
  di `eframe::run_native`, passa `Option<Arc<TrayHandle>>` a `SpeedyApp::new`.
- `packages/speedy-gui/src/app.rs` — `notify_on_error` persistito,
  `handle_tray_actions`, `notify_new_errors`, signature di
  `views::dashboard::render` arricchita.
- `packages/speedy-gui/src/tray.rs` — **nuovo**.
- `packages/speedy-gui/src/autostart.rs` — **nuovo**.
- `packages/speedy-gui/src/views/logs.rs` — riscritto: source switch
  (Live / File), export button, history viewer.
- `packages/speedy-gui/src/views/dashboard.rs` — checkbox notifiche +
  checkbox autostart (con feedback toast).
- `packages/speedy-gui/src/daemon.rs` — +5 test.
- `todo-gui.md` — tutti i task chiusi.

### Risultato

`cargo check --workspace --all-targets` → verde.
`cargo build-all` → 5 binari release in 45s.
`cargo test -p speedy-core --lib` → 57/57.
`cargo test -p speedy-daemon --bin speedy-daemon` → 65/65.
`cargo test -p speedy-gui` → 5/5.

Resta solo lo smoke E2E manuale (`cargo run --release -p speedy-gui` e
verifica visiva di tray icon + notifiche + autostart), non eseguibile in
sessione non-interattiva.

