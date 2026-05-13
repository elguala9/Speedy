# Piano di Completamento — Speedy

## Stato al 2026-05-14

> Baseline: 201 test (182 inline + 19 integration), 3 binary in `dist/`, TODO con ~20 item di cui ~8 già implementati ma non aggiornati.

---

## 0. Obsoleto da rimuovere dal TODO

| Item | Stato reale |
|------|-------------|
| Porta occupata → errore | `speedy-daemon/src/main.rs:929` — test già scritto |
| Port fallback | `speedy-daemon/src/main.rs:968` — test già scritto |
| Config reload (`reload` command) | `speedy-daemon/src/main.rs:372` — già implementato |
| Logging strutturato (`eprintln!` → `tracing`) | Già fatto: tutti e 3 i binary hanno `tracing_subscriber::fmt()`, unico residuo `testexe/src/main.rs:134` |
| `ensure_daemon()` daemon dead | Test già scritto (`test_ensure_daemon_daemon_dead_spawn_fails`) ma solo failure path |

---

## 1. Test facili (priorità alta)

### 1A. `speedy` standalone (no daemon)

**File:** `packages/speedy-cli/tests/e2e_test.rs`

**Logica:** I test e2e esistenti passano sempre dal daemon (`DaemonGuard` → `run_cli`). Manca la verifica che `speedy.exe` funzioni anche **senza** daemon, in modalità autonoma.

**Test da aggiungere:**

```
test_standalone_index          — speedy.exe index . in temp dir
test_standalone_query          — speedy.exe query "test" dopo index
test_standalone_context        — speedy.exe context
test_standalone_sync           — speedy.exe sync
```

**Pattern:**
```rust
fn test_standalone() {
    let dir = tempdir();
    create_test_project(&dir);
    let speedy = build_binary("speedy", "speedy");
    
    // Index
    let out = Command::new(&speedy).args(["index", "."]).current_dir(&dir).output();
    assert!(out.status.success());
    
    // Query
    let out = Command::new(&speedy).args(["query", "test"]).current_dir(&dir).output();
    assert!(out.status.success());
}
```

### 1B. Index con file non esistenti

**File:** `packages/speedy-cli/tests/e2e_test.rs`

**Logica:** Verificare che `speedy.exe index /path/inesistente` restituisca un errore gestito (non crash).

**Test:**
```rust
fn test_index_nonexistent_path() {
    let speedy = build_binary("speedy", "speedy");
    let out = Command::new(&speedy)
        .args(["index", "C:\\questa_dir_nON_esiste_xyz789"])
        .output();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error") || stderr.contains("Error") || stderr.contains("No such"));
}
```

### 1C. Watch `--detach`

**File:** `packages/speedy-cli/tests/e2e_test.rs`

**Logica:** `speedy.exe watch --detach` dovrebbe spawnare un watcher in background e tornare subito. Il test verifica che il comando torni OK e che il processo si stacchi.

**Test:**
```rust
fn test_watch_detach() {
    let dir = tempdir();
    create_test_project(&dir);
    let speedy = build_binary("speedy", "speedy");
    
    let out = Command::new(&speedy)
        .args(["watch", "--detach"])
        .current_dir(&dir)
        .output();
    assert!(out.status.success());
    
    // Give it a moment, then verify no crash
    std::thread::sleep(Duration::from_secs(1));
}
```

### 1D. `ensure_daemon()` daemon morto → spawn reale

**File:** `packages/speedy/src/main.rs` (test esistente `test_ensure_daemon_daemon_dead_spawn_fails`)

**Logica:** Il test esistente verifica solo il **failure path** (daemon non spawnabile perché manca l'exe). Va aggiunto un test del **success path**: buildare `speedy-daemon`, killare il daemon (o pulire il PID), chiamare `ensure_daemon()` e verificare che spawni un nuovo daemon.

**Pattern:**
```rust
async fn test_ensure_daemon_daemon_dead_spawn_success() {
    // 1. Build speedy-daemon e posizionarlo accanto al test binary
    // 2. Mockare/Mettere in PATH il daemon
    // 3. Far finta che un daemon sia morto (PID inesistente in daemon.pid)
    // 4. Chiamare ensure_daemon()
    // 5. Verificare che un nuovo daemon sia partito (TCP connect + ping)
}
```

---

## 2. Test watcher reali (complesso)

### 2A. Notify event → `speedy.exe index`

**File:** `packages/speedy-daemon/src/main.rs` (nuovo test inline)

**Logica:** Avviare un `CentralDaemon` reale in un thread, aggiungere un workspace via `DaemonClient`, creare/modificare un file nel workspace, attendere il debouncer (500ms + margine), e verificare che `speedy.exe index <file>` venga invocato.

**Problema:** Il watcher spawna `speedy.exe index <path>` in un thread separato. Per verificare, si può:
- Monitorare i PID attivi (`active_pids`) 
- Oppure wrappare la chiamata a `speedy.exe` con un binary di test (`testexe`) che scrive su un file di log
- Oppure usare `tempfile` per creare file nuovi e verificare che il DB venga aggiornato

**Approccio consigliato:** Usare una variabile atom/share tra test e watcher per contare le call a `speedy.exe`.

```rust
async fn test_watcher_file_change_triggers_index() {
    let guard = start_daemon("speedy_d_test_watcher_trigger");
    let ws_path = guard.dir.to_string_lossy().to_string();
    
    guard.client.add_workspace(&ws_path).await.unwrap();
    
    // Create a new file in the workspace
    let new_file = guard.dir.join("nuovo-file.rs");
    std::fs::write(&new_file, "fn new() {}").unwrap();
    
    // Wait for debouncer (500ms) + margin
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Verify via query that the file was indexed
    // (this requires standalone speedy to query the same DB)
    // Alternative: check active_pids was populated and cleared
    drop(guard);
}
```

---

## 3. Miglioramenti

### 3A. Concorrenza workspace (file lock)

**File:** `packages/speedy-core/Cargo.toml` + `packages/speedy-core/src/workspace.rs`

**Logica:** Il `Mutex` in-process non protegge da accessi concorrenti di **processi diversi** (es. due worker o worker + daemon). Serve un **file lock** su `workspaces.json`.

**Aggiunte:**
- Dipendenza: `fd-lock = "4"` in `speedy-core/Cargo.toml`
- Sostituire `std::sync::Mutex` con `fd_lock::RwLock` sul file `workspaces.json`

**Pattern:**
```rust
fn workspace_file_lock() -> Result<fd_lock::RwLock<std::fs::File>> {
    let path = workspaces_path()?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)?;
    Ok(fd_lock::RwLock::new(file))
}
```

### 3B. Health check periodico migliorato

**File:** `packages/speedy-daemon/src/main.rs`

**Logica:** Il daemon ha già un `health_ticker` ogni 30s che logga il numero di watcher. Va esteso per:
- Verificare che i thread watcher siano effettivamente vivi
- Tentare restart automatico di watcher morti
- Loggare warning se un watcher non risponde

**Modifiche:**
```rust
_ = health_ticker.tick() => {
    let ws = watchers_clone.lock().await;
    for (path, handle) in ws.iter() {
        // Verificare che il watcher thread sia vivo
        // Se morto: restart
    }
    info!("Health: {} watcher(s) active", ws.len());
}
```

### 3C. Documentazione API IPC

**File:** Nuovo `docs/ipc-protocol.md`

**Logica:** La documentazione del protocollo IPC esiste già come commento in testa a `speedy-daemon/src/main.rs` (linee 1-22). Va estratta in un file dedicato e arricchita con esempi.

### 3D. Pulizia `testexe`

**File:** `packages/testexe/src/main.rs`

**Logica:** Unico residuo di `eprintln!` nel progetto. Sostituire con `tracing::error!` (crate già presente).

---

## Riepilogo modifiche ai file

| File | Cosa | Fase |
|------|------|------|
| `TODO.md` | Cleanup item già fatti | 0 |
| `speedy-cli/tests/e2e_test.rs` | +4 test: standalone, nonexistent, watch-detach | 1 |
| `speedy/src/main.rs` | +1 test: ensure_daemon spawn success | 1D |
| `speedy-daemon/src/main.rs` | +1 test: watcher file change trigger | 2 |
| `speedy-core/Cargo.toml` | +`fd-lock` dep | 3A |
| `speedy-core/src/workspace.rs` | File lock su workspaces.json | 3A |
| `speedy-daemon/src/main.rs` | Health check migliorato | 3B |
| `docs/ipc-protocol.md` | Nuovo: documentazione API IPC | 3C |
| `testexe/src/main.rs` | eprintln! → tracing::error! | 3D |
| `docs/COMPLETION-PLAN.md` | Questo file | - |

---

## Metriche target (post-completamento)

| Package | Test attuali | Target |
|---------|-------------|--------|
| `speedy-core` | 24 | 24 |
| `speedy` | 111 | 115 (+4) |
| `speedy-daemon` | 22 | 24 (+2) |
| `speedy-cli` | 33 | 37 (+4) |
| `speedy-mcp` | 33 | 33 |
| **Totale** | **223** | **233** (+10 test) |
