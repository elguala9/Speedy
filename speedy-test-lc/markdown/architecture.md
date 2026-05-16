# Architecture

## Overview

Speedy is a local semantic-search service composed of five binaries and a shared core library.

```
speedy-ai-context   CLI / worker process (indexing, query, embedding, SQLite)
speedy-daemon       Long-running watcher + IPC server (one per user, global)
speedy-cli          Thin client — routes commands to the daemon via IPC
speedy-mcp          Model-Context-Protocol adapter (stdio JSON-RPC)
speedy-gui          Desktop tray app (egui — Dashboard / Workspaces / Scan / Logs)
speedy-core         Shared types, IPC client, config, DB primitives (library)
```

## Data flow

```
User edits a file
      │
      ▼
speedy-daemon (filesystem watcher, debounce 500 ms)
      │  spawns subprocess
      ▼
speedy-ai-context index <file>    ← SPEEDY_NO_DAEMON=1 to prevent recursion
      │
      ├─ chunk the file (1000 chars, 200-char overlap)
      ├─ compute embeddings  (configured provider)
      └─ upsert into SQLite vector store (.speedy/index.sqlite)
```

## IPC protocol

The daemon exposes a Unix domain socket (Windows named pipe). Each connection is a single newline-terminated request/response pair:

| Command | Description |
|---|---|
| `ping` | Returns `pong` |
| `status` | JSON daemon status (pid, uptime, workspace_count, …) |
| `add <path>` / `remove <path>` | Register / unregister a workspace |
| `sync <path>` | Incremental re-sync of a workspace |
| `reindex <path>` | Drop + rebuild the entire index |
| `exec <args>` | Run `speedy-ai-context <args>` and return stdout |
| `query-all\t<k>\t<q>` | Fan-out semantic search across all workspaces |
| `subscribe-log` | Long-lived streaming log channel |
| `prune-missing` | Remove watchers/registry entries for deleted paths |
| `scan\t<root>` | Walk `<root>` for existing `.speedy/index.sqlite` directories |

Full reference: `docs/ipc-protocol.md`.

## Storage

Each workspace gets a `.speedy/` directory:

```
.speedy/
  index.sqlite       ← vector store (chunks, embeddings, file hashes)
  config.toml        ← per-workspace overrides + feature toggles
  config.speedy.json ← per-workspace provider config (JSON, takes precedence)
  slc.db             ← speedy-language-context symbol graph (if enabled)
```

The daemon persists its own state in `$CONFIG_DIR/speedy/`:

```
$CONFIG_DIR/speedy/       (Windows: %APPDATA%\speedy)
  daemon.pid
  workspaces.json
  logs/daemon.log.*
```

## Embedding providers

| Provider | Config key | Notes |
|---|---|---|
| Ollama | `ollama` | Default; requires local Ollama instance |
| OpenAI | `openai` | Native embedding API |
| Gemini | `gemini` | Native embedding API |
| Azure OpenAI | `azure-openai` | Set `base_url` to your deployment endpoint |
| OpenAI-compatible | `openai-compatible` | LM Studio, vLLM, etc. — requires `base_url` |
| Anthropic | `anthropic` | No native embedding — uses generative model as proxy |
| DeepSeek | `deepseek` | No native embedding — uses generative model as proxy |
| Agent | `agent` | Shells out to external process; reads JSON float array from stdout |

Full provider reference: `CONFIG.md`.

## Feature toggles

Each workspace can independently enable/disable the two subsystems via `<workspace>/.speedy/config.toml`:

```toml
[features]
speedy_indexer = true    # file indexer (speedy-ai-context)
language_context = true  # code intelligence (speedy-language-context)
```

The GUI (Workspaces tab) provides checkboxes that read/write this file directly.
