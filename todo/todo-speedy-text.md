# speedy-text — TODO (MVP exe)

Strumento CLI per indicizzare occorrenze di simboli testuali in una repo.
Language-agnostic: funziona su qualunque file di testo (.md, .txt, .json, ecc.).

Scope attuale: **solo exe**, niente MCP.

---

## Tipi di ricerca

| Tipo | Descrizione | `$Dummy$` | `foo Dummy bar` | `FooDummyBar` |
|------|-------------|-----------|-----------------|---------------|
| `cased` | Sottostringa esatta, case-sensitive | ✓ | ✓ | ✓ |
| `isolated_special` | Delimitata da non-alfanumerici o inizio/fine riga | ✓ | ✓ | ✗ |
| `isolated` | Delimitata da spazi o inizio/fine riga | ✗ | ✓ | ✗ |

Tutti i tipi sono case-sensitive di default (flag `--ignore-case` disponibile). La ricerca è sempre su un simbolo alla volta.

---

## CLI MVP

```
speedy-text index  <path>                        Cancella l'indice e reindicizza tutto da zero
speedy-text sync   <path>                        Aggiornamento incrementale: aggiunge/aggiorna i file
                                                  modificati, rimuove i file cancellati — non tocca i file
                                                  non modificati (confronto via hash SHA-256)
speedy-text query  <path> <symbol>               Cerca (tutti i tipi, tutte le ext)
             [--type cased|isolated|isolated_special]
             [--ext  <EXT>]                       es. --ext md
             [--ignore-case]                      ricerca case-insensitive
speedy-text status <path>                        Stato indice
```

Output `query` — JSON su stdout:

```json
{
  "symbol": "Dummy",
  "type": "isolated_special",
  "ext": "md",
  "count": 3,
  "results": [
    { "file": "docs/guide.md", "line": 42, "col_start": 3, "col_end": 8 },
    ...
  ]
}
```

---

## Schema DB

File: `<root>/.speedy-text/index.db`

```sql
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Una riga per occorrenza, search_type è il tipo "più stretto":
--   isolated → implica anche isolated_special e cased
--   isolated_special → implica anche cased
--   cased → solo sottostringa
CREATE TABLE occurrences (
    symbol      TEXT    NOT NULL,
    search_type TEXT    NOT NULL,
    file_path   TEXT    NOT NULL,
    file_ext    TEXT    NOT NULL,
    line_no     INTEGER NOT NULL,
    col_start   INTEGER NOT NULL,
    col_end     INTEGER NOT NULL
);
CREATE INDEX idx_sym_type     ON occurrences(symbol, search_type);
CREATE INDEX idx_sym_type_ext ON occurrences(symbol, search_type, file_ext);

-- Re-index incrementale via hash
CREATE TABLE indexed_files (
    file_path  TEXT PRIMARY KEY,
    file_ext   TEXT NOT NULL,
    hash       TEXT NOT NULL,
    indexed_at INTEGER NOT NULL
);
```

Al query, la clausola `search_type` si espande:
- richiesto `cased` → `IN ('cased','isolated_special','isolated')`
- richiesto `isolated_special` → `IN ('isolated_special','isolated')`
- richiesto `isolated` → `= 'isolated'`

Con `--ignore-case` il confronto si fa su `LOWER(symbol)` vs `LOWER(?)`.

---

## Struttura crate

```
packages/speedy-text/
├── Cargo.toml
└── src/
    ├── main.rs        CLI entry point, dispatch
    ├── config.rs      trova root, percorso DB
    ├── ignore.rs      .speedyignore / .gitignore
    ├── walk.rs        walk filesystem → Vec<(PathBuf, String)>
    ├── tokenize.rs    estrae Token per i tre tipi
    ├── db.rs          open, migrations, insert, query
    ├── indexer.rs     pipeline walk→tokenize→insert
    └── query.rs       SELECT + serializzazione JSON
```

---

## Task

- [ ] **1. Crate + Cargo.toml**
  - Dipendenze: `rusqlite` (bundled), `walkdir`, `ignore`, `sha2`, `clap` (derive), `serde`, `serde_json`, `anyhow`
  - Aggiungere al workspace `Cargo.toml`

- [ ] **2. `config.rs`**
  - `find_root(start: &Path) -> PathBuf` — usa il path dato direttamente come root
  - `db_path(root: &Path) -> PathBuf` — `<root>/.speedy-text/index.db`

- [ ] **3. `db.rs`**
  - `open(path: &Path) -> Connection` — crea la dir se non esiste, abilita WAL
  - `migrate(conn: &Connection)` — crea tabelle + indici se non esistono
  - `insert_occurrences(conn, file_path, file_ext, tokens: &[Token])`
  - `delete_file(conn, file_path)` — per re-index
  - `upsert_indexed_file(conn, file_path, file_ext, hash)`
  - `get_file_hash(conn, file_path) -> Option<String>`
  - `query(conn, symbol, search_type, ext_filter, ignore_case) -> Vec<Occurrence>`
  - `status(conn) -> Status` — conteggi per output

- [ ] **4. `tokenize.rs`**
  - Struct `Token { symbol: String, search_type: SearchType, line_no: u32, col_start: u32, col_end: u32 }`
  - `tokenize(content: &str) -> Vec<Token>`
    - `_` e `-` sono sia parte del token che separatori (come i non-alfanumerici normali)
    - Per ogni sequenza `[a-zA-Z0-9_\-]+` trovata nel testo:
      1. Indicizza il token intero con il tipo "più stretto" basato sui char esterni
      2. Se il token contiene `_` o `-`, indicizza anche ogni componente separata come `isolated_special`
         (es. `my_func` → `my_func` intero + `my` isolated_special + `func` isolated_special)
    - Classificazione del tipo per il token intero:
      - char precedente e successivo entrambi spazio/newline/inizio-fine → `isolated`
      - char precedente e successivo entrambi non-alfanumerici → `isolated_special`
      - altrimenti → `cased`
    - Emette una sola riga per occorrenza (il tipo più stretto)

- [ ] **5. `ignore.rs` + `walk.rs`**
  - Usa crate `ignore` (WalkBuilder) — rispetta `.gitignore` e `.speedyignore`
  - Skip default: `.speedy-text/`, `.git/`
  - `walk(root: &Path, ext_filter: Option<&str>) -> Vec<(PathBuf, String)>`
    - ritorna (path, ext_normalizzata_lowercase_senza_punto)

- [ ] **6. `indexer.rs`**
  - `index(conn: &Connection, root: &Path)`
    - Cancella tutte le tabelle (`DELETE FROM occurrences`, `DELETE FROM indexed_files`)
    - Walk → per ogni file: leggi UTF-8, tokenize, `insert_occurrences` + `upsert_indexed_file`
    - Stampa progresso ogni 100 file su stderr
  - `sync(conn: &Connection, root: &Path)`
    - Walk → per ogni file:
      1. Hash SHA-256 contenuto
      2. Se hash uguale a DB → skip
      3. Leggi UTF-8, skip non-UTF-8 con eprintln warning
      4. `delete_file` + tokenize + `insert_occurrences` + `upsert_indexed_file`
    - Dopo il walk, recupera tutti i `file_path` in `indexed_files` non più presenti su disco → `delete_file` per ognuno
    - Stampa progresso ogni 100 file su stderr

- [ ] **7. `query.rs`**
  - `run_query(conn, symbol, search_type, ext_filter, ignore_case) -> QueryResult`
  - Serializza in JSON e stampa su stdout

- [ ] **8. `main.rs`**
  - Subcomandi `index`, `sync`, `query`, `status` con clap
  - Errori con `anyhow`, uscita 1 su errore

- [ ] **9. Smoke test manuale**
  - `cargo build -p speedy-text`
  - `speedy-text index .`
  - `speedy-text query . Dummy --type isolated_special --ext md`
  - `speedy-text query . dummy --ignore-case` → deve trovare anche `Dummy`
  - `speedy-text status .`
  - Modifica un file, poi `speedy-text sync .` → verifica che solo il file modificato venga re-indicizzato
  - Cancella un file, poi `speedy-text sync .` → verifica che le occorrenze vengano rimosse

---

## Fuori scope MVP

- sqlite-vec / vettori (aggiunto in fase successiva)
- MCP server
- Test automatici (dopo smoke test manuale)
- `reindex` e `clear` subcomandi

---

## Dubbi aperti (da chiudere prima dello sviluppo)

### A. File binari / dimensione

Il progetto esistente ha `packages/speedy-core/assets/default_ignores.txt` con estensioni binarie
note (`*.exe`, `*.pdb`, `*.png`, ecc.) e dir da skippare (`target/`, `node_modules/`).

- Riutilizzo `speedy-core::default_ignores` da `speedy-text`, oppure copia/duplica la lista? Riutilizza default_ignores
- Detect runtime: se un file passa il filtro ext ma contiene NUL byte → skip (binary)? Si, skippalo
- Limite dimensione file? Es. skip se >5 MB → evita di tokenizzare file di log enormi. Da parametrizzare, valore di default 10MB

### B. Integrazione workspace

- Aggiungere `packages/speedy-text` ai `members` di `Cargo.toml` root. SI
- Nome binario: `speedy-text` (via `[[bin]]` in `packages/speedy-text/Cargo.toml`).  speedy-text-context.exe
- Conferma: dipende da `speedy-core` (per `default_ignores`) o resta totalmente standalone? DIpende in che senso, le librerie posono essre condivise, ma sarà un eseguibile a parte

### C. Path normalization

Decidere format dei `file_path` salvati in DB:
- Assoluti o relativi alla root? Assoluti
- Su Windows: backslash `\` o forward slash `/`? Quello che va anche su linux
- Proposta: **relativi alla root, forward slash sempre** — output JSON consistente cross-platform. SI

### D. Indexing convention `line_no` / `col_start` / `col_end`

- `line_no`: 1-indexed (convenzione editor) o 0-indexed? 0
- `col_start`/`col_end`: byte offset, char offset, o UTF-16 code unit? Quello più robusto e compatibile con altri os
- Proposta: **`line_no` 1-indexed, col in byte offset 0-indexed, `col_end` esclusivo** (range half-open). Va bene

### E. Performance batch

Inserire un INSERT per token su file grandi è lento. Serve:
- Transazione per file (`BEGIN`/`COMMIT`)
- Prepared statement riutilizzato
- `PRAGMA synchronous=NORMAL` durante index/sync (sicuro con WAL)

Va bene tutto

Da esplicitare nel task `db.rs` / `indexer.rs`.

### F. Tabella `meta`

Schema definito ma non usato. Cosa ci va dentro?
- Proposta: `schema_version` (per migration future), `last_index_at`, `last_sync_at`. ok, accetato
- Senza un uso definito, rimuovere dallo schema per ora.

### G. Output `status`

Task `db.rs` dice "Status — conteggi per output" ma non specifica:
- Formato JSON o testo? 
- Quali campi? Proposta: `{ files, occurrences, unique_symbols, db_size_bytes, last_index_at }`

Per ora salta

### H. Default di `--type` in `query`

Se l'utente non passa `--type`, cosa cerca?
- Proposta: default `cased` (include `isolated_special` e `isolated` via espansione) → "trova ovunque". Si va bene

### I. UTF-16 BOM su Windows

File salvati con encoding UTF-16 LE + BOM sono comuni su Windows (PowerShell default).
- Skip con warning come per altri encoding non-UTF-8?
- Oppure detect BOM → converti a UTF-8 e indicizza?
- Proposta MVP: **skip con warning**, BOM detection in fase successiva. Ok va bene

---

## Dubbi IA (risolti)

1. **Definizione di "parola" in `tokenize`**: `_` e `-` sono sia parte del token che separatori.
   `my_func` → indicizzata intera + `my` (isolated_special) + `func` (isolated_special).

2. **`isolated` con punteggiatura a fine riga**: `Dummy.` — `isolated_special` lo trova, `isolated` no. Corretto.

3. **`--ignore-case`**: sì, da implementare nel MVP. Flag su `query`.

4. **Filtro estensioni all'index**: si usa `.speedyignore` per escludere file/cartelle dall'indicizzazione.
   Il flag `--ext` resta solo su `query`.

5. **Output quando nessun risultato**: JSON con `count: 0` e `results: []`, exit 0.
