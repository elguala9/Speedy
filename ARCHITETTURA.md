# Architettura Speedy — 3 Eseguibili

## I Tre Attori

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│   AI Agent / Utente / MCP                                        │
│         │                                                        │
│         ▼                                                        │
│   ┌────────────┐    TCP :42137                                   │
│   │ speedy-cli  │────────────────┐                               │
│   │ .exe        │                │                               │
│   └────────────┘                │                               │
│         │                       │                               │
│         │ (oppure chiama        │                               │
│         │  direttamente)        │                               │
│         ▼                       ▼                               │
│   ┌──────────────────────────────────────┐                      │
│   │         speedy-daemon.exe            │                      │
│   │                                      │                      │
│   │  ● IPC server (TCP :42137)          │                      │
│   │  ● Monitora file system (notify)    │                      │
│   │  ● Gestisce N workspace             │                      │
│   │  ● NON fa embedding/indexing        │                      │
│   │  ● Chiama speedy.exe per il lavoro  │                      │
│   └──────────┬───────────────────────────┘                      │
│              │                                                  │
│              │ subprocess: speedy.exe index ./src/file.rs       │
│              ▼                                                  │
│   ┌──────────────────────────────────────┐                      │
│   │          speedy.exe                  │                      │
│   │                                      │                      │
│   │  ● Indexing (embedding + SQLite)    │                      │
│   │  ● Query (semantic search)          │                      │
│   │  ● Sync, Context, Force             │                      │
│   │  ● Chunking, hashing, file filter   │                      │
│   │  ● Può essere usato standalone      │                      │
│   └──────────────────────────────────────┘                      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Ruoli

### `speedy.exe` — Il Lavoratore
- **Contiene TUTTA la logica**: indexer, query, embedding, SQLite, chunking, hashing, file filtering, ignore patterns
- Può essere chiamato standalone: `speedy index .`, `speedy query "cose"`
- Viene spawnato come **subprocess** dal daemon quando serve lavoro
- Non ha logica di monitoring, non resta in esecuzione
- Dipende da: `speedy-core` + `speedy-daemon-core`
- **Size**: ~5-8MB (Ollama, SQLite, notify, tutto)

### `speedy-daemon.exe` — Il Manager
- **Unico processo long-running**
- All'avvio:
  1. Legge `~/.config/speedy/workspaces.json`
  2. Per ogni workspace: avvia un **file watcher** (notify)
  3. Espone IPC server su `TCP 127.0.0.1:42137`
- Quando un file cambia:
  1. Il watcher rileva la modifica
  2. Calcola l'hash del file
  3. Se l'hash è diverso → spawna `speedy.exe -p <workspace> index <path>`
  4. Se l'hash è uguale (es. nostra scrittura) → skip (grazie al PID check)
- **Non fa direttamente embedding/indexing** — delega a `speedy.exe`
- Espone API via TCP per query/status/etc:
  - Se la richiesta è una **query** → spawna `speedy.exe query ...` e ritorna risultato
  - Se la richiesta è **reindex** → spawna `speedy.exe index ...`
  - Se la richiesta è **status** → risponde direttamente (non serve speedy.exe)
- **PID tracking**: prima di spawnare `speedy.exe`, salva il PID in una coda `active_pids`. Quando arrivano eventi di file, controlla se il PID del processo che ha scritto è nella coda → skip. Dopo che `speedy.exe` termina, rimuove il PID.
- Dipende da: `speedy-core` + `speedy-daemon-core`

### `speedy-cli.exe` — Il Thin Client
- **Leggerissimo**: solo tokio + serde + clap
- Si connette al daemon via TCP `127.0.0.1:42137`
- Se il daemon non è attivo → lo spawna (chiama `speedy-daemon.exe`)
- Proxy semplice: inoltra comandi al daemon, ritorna risposte
- **Questo è ciò che chiamano AI Agent e MCP server**
- Non ha dipendenze pesanti (no SQLite, no Ollama, no notify)
- Dipende solo da: `speedy-core`

## Flusso Completo

### Query (da AI Agent)
```
AI Agent → speedy-cli query "trova auth"
  → TCP connect :42137
  → send("query trovare auth")
  → daemon spawna: speedy.exe query "trova auth"
  → speedy.exe fa query sul DB, ritorna risultato
  → daemon ritorna risultato a speedy-cli
  → AI Agent riceve risposta
```

### File Change (da watcher)
```
Utente modifica src/lib.rs
  → notify triggera evento
  → daemon calcola hash
  → hash diverso → daemon spawna: speedy.exe -p /project index ./src/lib.rs
  → speedy.exe reindicizza il file
  → speedy.exe termina
  → daemon registra che il PID non è più attivo
```

### Riavvio PC
```
PC riparte
  → nessun daemon in esecuzione
  → prima chiamata a speedy-cli:
    1. TCP connect fallisce
    2. kill_existing_daemon() (pulisce PID stale)
    3. spawna speedy-daemon.exe
    4. daemon carica workspaces.json
    5. per ogni workspace: avvia watcher
    6. speedy-cli aggiunge workspace corrente
    7. procede con la richiesta originale
```

### Self-Modification Safety
```
Daemon spawna: speedy.exe -p /proj index ./src/lib.rs
  → speedy.exe scrive sul DB
  → il watcher NOTA la modifica... MA:
  → il PID di speedy.exe è nella coda active_pids
  → hash check: l'hash NON è cambiato (stesso contenuto)
  → skip ✓
```

## Struttura Crate

```
Cargo.toml (workspace)
├── packages/
│   ├── speedy-core/          (lib = speedy_core)
│   │   ├── daemon_client.rs   ← DaemonClient (per speedy-cli)
│   │   ├── daemon_util.rs     ← spawn/kill/dir utility
│   │   ├── workspace.rs       ← workspace registry
│   │   ├── config.rs          ← configurazione
│   │   └── embedding.rs       ← Embedding type
│   │
│   ├── speedy-daemon-core/   (lib = speedy_daemon_core)
│   │   ├── daemon_central.rs  ← CentralDaemon + IPC + watcher
│   │   ├── daemon.rs          ← legacy daemon
│   │   ├── indexer.rs         ← indexing engine
│   │   ├── watcher.rs         ← file watcher (notify)
│   │   ├── db.rs              ← SQLite vector store
│   │   ├── embed.rs           ← embedding providers
│   │   ├── hash.rs, ignore.rs, document.rs, file.rs, text.rs
│   │
│   ├── speedy/               (bin = speedy.exe)
│   │   ├── src/main.rs        ← worker entry point
│   │   └── src/cli.rs         ← argument parsing
│   │   Dipende da: speedy-core + speedy-daemon-core
│   │
│   ├── speedy-daemon/        (bin = speedy-daemon.exe)
│   │   └── src/main.rs        ← daemon entry point
│   │   Dipende da: speedy-core + speedy-daemon-core
│   │
│   └── speedy-cli/           (bin = speedy-cli.exe)
│       └── src/main.rs        ← thin client
│       Dipende da: speedy-core (solo DaemonClient)
│
│   (esistenti)
│   ├── speedy-mcp/
│   └── testexe/
```

## Speedy.exe può essere chiamato da chiunque

L'utente può usare `speedy.exe` direttamente:
```bash
speedy.exe index .                       # indicizza
speedy.exe query "funzione di login"     # cerca
speedy.exe -p /altro/progetto query "x"  # cerca su altro progetto
```

Non c'è bisogno del daemon per operazioni one-shot. Il daemon serve solo per:
1. **Monitoring continuo** — reagire a cambiamenti automaticamente
2. **Pre-flight check** — garantire che l'indice sia sempre aggiornato
3. **API server** — permettere a agent AI / MCP di fare query rapide

## Comandi CLI

| Comando | Su speedy.exe | Su speedy-cli.exe |
|---|---|---|
| `index` | ✅ indicizza direttamente | → TCP → daemon → speedy.exe |
| `query` | ✅ cerca direttamente | → TCP → daemon → speedy.exe |
| `context` | ✅ contesto diretto | → TCP → daemon (risponde diretto) |
| `sync` | ✅ sync diretto | → TCP → daemon → speedy.exe |
| `daemon` | ❌ (non pertinente) | → avvia speedy-daemon.exe (se non attivo) |
| `--help` | help completo | help leggero |
