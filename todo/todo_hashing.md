# TODO: Centralized Hash Registry

## Obiettivo

Centralizzare il controllo degli hash dei file in un unico registro condiviso (in `speedy-core`),
mantenendo ogni context **indipendente**: ognuno scrive e legge solo le proprie righe.
Il daemon non dovrà più delegare il controllo delle modifiche ai subprocess — potrà farlo prima di
spawnarli, evitando di avviare processi inutili se il file non è cambiato.

---

## Stato attuale

| Package | Hash | Storage | Mtime? | Change detection |
|---|---|---|---|---|
| `speedy-ai-context` | SHA256 | `sac.sqlite` tabella `chunks` (col. `hash`) | Sì (`last_modified`) | mtime → fallback hash |
| `speedy-language-context` | **blake3** | `slc.sqlite` tabella `slc_files` (col. `content_hash`) | Sì (`mtime` unix sec) | hash only |
| `speedy-text` | SHA256 | `stext.sqlite` tabella `indexed_files` (col. `hash`) | No | hash only |

Problemi:
- Il daemon spawna sempre il subprocess anche se il file non è cambiato — il check avviene *dentro* il processo figlio.
- Tre algoritmi/storage diversi: SHA256 (sac, stext) vs blake3 (slc).
- `speedy-text` non ha mtime → sempre lettura da disco.
- Nessuna visione trasversale: non si sa "quali contesti hanno già indicizzato questo file".

---

## Architettura proposta

### 1. Shared Hash Registry in `speedy-core`

Nuovo modulo: `speedy-core/src/hash_registry.rs`

Database: `.speedy/hashes.sqlite` (uno per workspace)

Schema:
```sql
CREATE TABLE file_hashes (
    file_path   TEXT    NOT NULL,   -- path relativo al workspace root
    context     TEXT    NOT NULL,   -- "ai-context" | "language-context" | "text"
    hash        TEXT    NOT NULL,   -- SHA256 hex
    mtime_secs  INTEGER NOT NULL,   -- unix timestamp secondi
    updated_at  INTEGER NOT NULL,   -- unix timestamp secondi
    PRIMARY KEY (file_path, context)
);
CREATE INDEX idx_file ON file_hashes(file_path);
```

API pubblica (`HashRegistry` struct):
```rust
pub fn open(workspace: &Path) -> Result<Self>
pub fn get(file: &str, context: &str) -> Option<FileHashEntry>
pub fn set(file: &str, context: &str, hash: &str, mtime_secs: u64) -> Result<()>
pub fn delete_file(file: &str) -> Result<()>                    // file rimosso
pub fn delete_context(context: &str) -> Result<()>             // re-index completo
pub fn get_all_for_context(context: &str) -> HashMap<String, FileHashEntry>
pub fn needs_reindex(file: &Path, context: &str) -> Result<bool>  // mtime + hash check
```

`needs_reindex` implementa la logica a due livelli:
1. Legge mtime dal disco → se uguale a `mtime_secs` → `false` (fast path)
2. Legge contenuto → SHA256 → se uguale a `hash` → `false`, aggiorna mtime
3. Altrimenti → `true`

### 2. Standardizzare su SHA256

`speedy-language-context` usa blake3. Migrare a SHA256 per:
- Condividere la stessa implementazione da `speedy-core`
- Evitare dipendenze diverse per lo stesso scopo
- Nota: blake3 è più veloce, ma la differenza è trascurabile su file piccoli — e la coerenza vale di più

Azione: rimuovere dipendenza `blake3` da `speedy-language-context/Cargo.toml`,
usare `speedy_core::hash_registry::hash_file()`.

### 3. Daemon: pre-check prima di spawning

In `speedy-daemon/src/main.rs` nella funzione `start_workspace_watcher()` (linee 413-487):

**Prima** di spawning subprocess, controllare il registro condiviso:

```rust
// Pseudo-code
let registry = HashRegistry::open(&workspace_path)?;
let changed_for_sac = registry.needs_reindex(&file, "ai-context")?;
let changed_for_slc = registry.needs_reindex(&file, "language-context")?;
let changed_for_text = registry.needs_reindex(&file, "text")?;

if features.speedy_indexer && changed_for_sac {
    spawn_sac(&file, ...);
}
if features.language_context && changed_for_slc {
    spawn_slc(&file, ...);
}
if features.speedy_text && changed_for_text {
    spawn_text(&file, ...);
}
```

Beneficio: se un file è salvato due volte senza modifiche (save ridondanti dell'editor),
**nessun subprocess** viene spawwato.

### 4. Ogni context rimuove il proprio hash check interno

Dopo l'adozione del registro, il check ridondante dentro ogni subprocess
può essere semplificato (ma non eliminato del tutto — il subprocess deve comunque
aggiornare il registro dopo aver completato l'indicizzazione).

Flusso per ogni context dopo la modifica:
1. Subprocess avviato dal daemon (già pre-filtrato)
2. Subprocess fa il proprio lavoro
3. Subprocess chiama `registry.set(file, context, hash, mtime)` al termine
4. Il check interno di hash può diventare un assert/sanity check opzionale

---

## Task

### speedy-core

- [x] Aggiungere dipendenza `rusqlite` e `sha2` in `speedy-core/Cargo.toml`
- [x] Creare `speedy-core/src/hash_registry.rs` con struct `HashRegistry` e API completa
- [x] Implementare `needs_reindex()` con logica mtime → SHA256, `mtime_unchanged()` per daemon
- [x] Implementare `hash_file()` e `hash_bytes()` come metodi statici pubblici
- [x] Aggiungere `hash_registry` al `pub mod` di `speedy-core/src/lib.rs`
- [x] Test unitari: file nuovo, invariato, modificato, delete_context, contesti indipendenti

### speedy-ai-context

- [x] `speedy-core` già presente come dipendenza
- [x] Aggiunto `mtime_secs` a `PreparedFile`, calcolato in `prepare_file()`
- [x] `reembed()` chiama `registry.delete_context("ai-context")` prima del wipe
- [x] `index_directory()` chiama `registry.set_indexed()` per ogni file dopo batch DB write
- [x] `index_file()` chiama `registry.set_indexed()` dopo DB write
- [x] `hash.rs` mantenuto per retrocompatibilità interna (stesso algoritmo SHA256)

### speedy-language-context

- [x] `speedy-core` già presente come dipendenza
- [x] Rimossa dipendenza `blake3` da `Cargo.toml`
- [x] `index_one_file()`: usa `HashRegistry::needs_reindex()` (aggiunge mtime fast-path)
- [x] Dopo upsert: chiama `registry.set_indexed()`
- [x] `full_index_blocking()` chiama `registry.delete_context("language-context")` prima del walk
- [x] `slc_files.content_hash` mantenuto per integrità FK interna (ora conterrà SHA256)

### speedy-text

- [x] Aggiunta dipendenza `speedy-core` in `Cargo.toml`
- [x] Rimossa dipendenza `sha2` (ora usa `HashRegistry::hash_bytes()`)
- [x] `sync()`: usa `mtime_unchanged()` fast-path + hash fallback via `HashRegistry`
- [x] `index()`: chiama `registry.delete_context("text")` + `registry.set_indexed()` per ogni file
- [x] Mtime tracking aggiunto nello stesso step (non richiede modifica schema stext.sqlite)

### speedy-daemon

- [x] Aggiunto `use speedy_core::hash_registry::HashRegistry`
- [x] Apre `HashRegistry` dentro ogni spawned event-handler thread (Connection è !Send)
- [x] Pre-check `mtime_unchanged()` per `"ai-context"` e `"language-context"` prima di ogni spawn
- [x] Log `debug` quando subprocess viene skippato per mtime invariato

---

## Note / Decisioni aperte

- **Concorrenza**: `hashes.sqlite` sarà scritto da più processi (daemon + subprocess). Usare WAL mode. I subprocess scrivono solo la propria riga `(file_path, context)` → no conflitti tra context diversi.
- **Hash pre-calcolato dal daemon**: il daemon potrebbe calcolare l'hash una volta e passarlo al subprocess (env `SPEEDY_FILE_HASH=<hex>`), evitando che ogni subprocess rilegga il file. Valutare se vale la complessità aggiuntiva.
- **Migration**: al primo avvio con il nuovo codice, `hashes.sqlite` è vuoto → tutti i context faranno un full re-index una tantum. Accettabile.
- **speedy-mcp / speedy-cli**: non indicizzano direttamente, nessuna modifica necessaria.
