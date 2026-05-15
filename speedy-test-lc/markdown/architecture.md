# Architecture

## Overview

Speedy is a local semantic-search service composed of four binaries and a shared core library.

```
speedy          CLI / worker process
speedy-daemon   Long-running watcher + IPC server
speedy-mcp      Model-Context-Protocol adapter
speedy-gui      Desktop tray app (optional)
speedy-core     Shared types, IPC client, config, DB primitives
```

## Data flow

```
User edits a file
      │
      ▼
speedy-daemon (filesystem watcher, debounce 500 ms)
      │  spawns
      ▼
speedy index <file>           ← sets SPEEDY_NO_DAEMON=1 to prevent recursion
      │
      ├─ chunk the file (1000 chars, 200-char overlap)
      ├─ compute embeddings  (Ollama or custom provider)
      └─ upsert into SQLite vector store (.speedy/index.sqlite)
```

## IPC protocol

The daemon exposes a Unix domain socket (Windows named pipe). Each connection is a single newline-terminated request/response pair:

| Command | Description |
|---|---|
| `ping` | Returns `pong` |
| `query-all\t<k>\t<q>` | Fan-out semantic search across all workspaces |
| `sync <path>` | Incremental re-sync of a workspace |
| `reindex <path>` | Drop + rebuild the entire index |
| `add <path>` | Register a new workspace |
| `subscribe-log` | Long-lived streaming log channel |

## Storage

Each workspace gets a `.speedy/` directory:

```
.speedy/
  index.sqlite   ← vector store (chunks, embeddings, file hashes)
  config.toml    ← per-workspace overrides
```

The daemon persists its own state in `$CONFIG_DIR/speedy/`:

```
$CONFIG_DIR/speedy/
  daemon.pid
  daemon.lock
  workspaces.json
  logs/daemon.log.*
```

## Embedding providers

| Provider | Config key | Notes |
|---|---|---|
| Ollama | `ollama` | Default; requires local Ollama instance |
| Agent | `agent` | Shells out to `$SPEEDY_AGENT_COMMAND` |
