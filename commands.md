# Comandi

## `speedy.exe`

### Subcomandi

| Comando                       | Cosa fa                                       |
|-------------------------------|-----------------------------------------------|
| `index [<SUBDIR>]`            | Indicizza una directory (default `.`)         |
| `query <QUERY> [-k <N>]`      | Ricerca semantica (default top-K 5)           |
| `context`                     | Riepilogo del workspace                       |
| `sync`                        | Sync incrementale FS → indice                 |
| `reembed`                     | Droppa tutti gli embedding e re-indicizza con il modello corrente |
| `daemon`                      | Spawna il daemon centrale                     |
| `workspace list`              | Lista workspace registrati                    |

### Flag

| Flag                          | Cosa fa                                       |
|-------------------------------|-----------------------------------------------|
| `-p, --path <PATH>`           | Project root (default cwd)                    |
| `--json`                      | Output JSON                                   |
| `--daemon-socket <NAME>`      | Nome socket (default `speedy-daemon`)         |
| `-r, --read <PROMPT>`         | Shortcut per `query`                          |
| `-m, --modify <CONTENT> --file <PATH>` | Scrive file e re-indicizza           |
| `-d, --daemons`               | Lista workspace tracciati dal daemon          |
| `-w, --workspaces`            | Lista workspace registrati                    |
| `-h, --help`                  | Help (anche per ogni subcomando)              |
| `-V, --version`               | Versione                                      |

---

## `speedy-cli.exe`

### Subcomandi

| Comando                       | Cosa fa                                       |
|-------------------------------|-----------------------------------------------|
| `index [<SUBDIR>]`            | Indicizza via daemon                          |
| `query <QUERY> [-k <N>] [--all]` | Ricerca semantica via daemon. `--all`: fan-out su tutti i workspace, aggrega top-K |
| `context`                     | Riepilogo workspace via daemon                |
| `sync`                        | Sync incrementale via daemon                  |
| `reembed`                     | Droppa embedding e re-indicizza con il modello corrente (via daemon) |
| `force [-p <PATH>]`           | Daemon-driven sync di un workspace            |
| `daemon status`               | Stato daemon (PID, uptime, ws/watchers)       |
| `daemon list`                 | Workspace attivi sul daemon                   |
| `daemon stop`                 | Ferma il daemon                               |
| `daemon ping`                 | Ping → pong                                   |
| `workspace list`              | Lista workspace registrati                    |
| `workspace add <PATH>`        | Aggiunge workspace al daemon                  |
| `workspace remove <PATH>`     | Rimuove workspace dal daemon                  |

### Flag

| Flag                          | Cosa fa                                       |
|-------------------------------|-----------------------------------------------|
| `-p, --path <PATH>`           | Project root (default cwd)                    |
| `--json`                      | Output JSON                                   |
| `--daemon-socket <NAME>`      | Nome socket (default `speedy-daemon`)         |
| `-h, --help`                  | Help (anche per ogni subcomando)              |
| `-V, --version`               | Versione                                      |

---

## `speedy-daemon.exe`

### Flag

| Flag                          | Cosa fa                                       |
|-------------------------------|-----------------------------------------------|
| `--daemon-socket <NAME>`      | Socket su cui ascoltare (default `speedy-daemon`) |
| `--daemon-dir <DIR>`          | Override directory `daemon.pid`/`workspaces.json` |
| `-h, --help`                  | Help                                          |
| `-V, --version`               | Versione                                      |

---

## `speedy-mcp.exe`

Nessun flag CLI. Avviato dal client MCP, comunica su stdio JSON-RPC.

### Tool esposti

| Tool             | Argomenti                              |
|------------------|----------------------------------------|
| `speedy_query`   | `{ query: string, top_k?: number }`    |
| `speedy_index`   | `{ path?: string }`                    |
| `speedy_context` | `{}`                                   |
