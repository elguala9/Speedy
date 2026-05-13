# Speedy

Local Semantic File System — bridges local filesystem with AI models.

Speedy indexes your codebase into a vector database, watches for file changes, and enables semantic search over your code. Designed as a plugin for [Opencode](https://opencode.ai).

## Quick Start

Minimal commands to get Speedy running:

```bash
# 1. Install Rust (if you don't have it): https://rustup.rs/

# 2. Install and start Ollama with an embedding model
curl -fsSL https://ollama.ai/install.sh | sh
ollama pull all-minilm:l6-v2

# 3. Build Speedy
cargo build --release

# 4. Index a project
speedy index /path/to/your/project

# 5. Search semantically
speedy query "how does authentication work?"
```

The binary is at `target/release/speedy` (`speedy.exe` on Windows).

## Installation Advanced

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2021)
- [Ollama](https://ollama.ai/) with an embedding model (default: `all-minilm:l6-v2`)

```bash
ollama pull all-minilm:l6-v2
```

### Build

```bash
cargo build --release
```

The `speedy` binary will be at `target/release/speedy.exe` (Windows) or `target/release/speedy` (Linux).

### Optional: config file

Create a `speedy.toml` or `.speedy/config.toml` in your project root:

```toml
model = "nomic-embed-text"
top_k = 10
```

All options are documented in the [Configuration](#configuration) section.

## Features

- **Semantic Indexing** — splits source files into chunks, generates embeddings, stores in SQLite
- **File Watching** — watches directories for changes and incrementally updates the index
- **Semantic Search** — query your codebase by meaning, not just keywords
- **Daemon Management** — create, stop, restart, delete background watcher daemons
- **Workspace Management** — manage multiple indexed workspaces
- **Embedding Cache** — avoids recomputing embeddings for unchanged content
- **Config File** — supports `speedy.toml` / `.speedy/config.toml` in addition to env vars

## CLI Reference

### Global flags

| Flag | Description |
|------|-------------|
| `--json` | Output in JSON format |
| `-p, --path <path>` | Project root (default: current directory) |
| `-V, --version` | Print version information |
| `-h, --help` | Print help |

### Top-level flags

These flags can be used in place of a subcommand:

| Flag | Short | Description |
|------|-------|-------------|
| `--read <prompt>` | `-r` | Query the workspace with a natural language prompt |
| `--modify <prompt>` | `-m` | Modify the workspace (write content to a file) |
| `--file <path>` | — | Target file for `--modify` |
| `--daemons` | `-d` | List all running daemons |
| `--daemon-stop <path>` | — | Stop a daemon |
| `--daemon-restart <path>` | — | Restart a daemon |
| `--daemon-delete <path>` | — | Delete a daemon permanently |
| `--daemon-create <path>` | — | Create a daemon for a workspace |
| `--daemon-status <path>` | — | Show daemon status |
| `--force <path>` | `-f` | Force reindex of a workspace |
| `--workspaces` | `-w` | List all registered workspaces |
| `--workspace-create <path>` | — | Create a workspace and its daemon |
| `--workspace-delete <path>` | — | Delete a workspace and its daemon |

### Subcommands

| Command | Description |
|---------|-------------|
| `index [<subdir>]` | Index a directory into the vector database |
| `query <query>` | Query the index with semantic search |
| `watch [<subdir>]` | Watch a directory for changes and auto-index |
| `context` | Show project context summary |
| `sync` | Sync filesystem changes to the database incrementally |
| `daemon <action>` | Manage the background watcher daemon |
| `workspace <action>` | Manage workspaces |
| `force [-p <path>]` | Force reindex of a workspace |

### `speedy --read` / `-r`

```
speedy --read "how does authentication work?"
speedy -r "project structure overview" -p /path/to/project
```

| Flag | Description |
|------|-------------|
| `-r, --read <prompt>` | Natural language prompt describing what to find |
| `-p, --path <path>` | Project root (default: current dir) |
| `--json` | Output in JSON format |

Queries the workspace index with semantic search. Returns ranked results matching the prompt's meaning.

### `speedy --modify` / `-m`

```
speedy --modify "console.log('hello');" --file src/index.js
speedy -m "fn main() {}" --file src/main.rs
```

| Flag | Description |
|------|-------------|
| `-m, --modify <content>` | Content to write |
| `--file <path>` | Target file path (required) |
| `-p, --path <path>` | Project root (default: current dir) |
| `--json` | Output in JSON format |

Writes content to a file and updates the index.

### `speedy --daemons` / `-d`

```
speedy --daemons
speedy -d
```

Lists all running daemons.

### `speedy --daemon-stop`

```
speedy --daemon-stop /path/to/project
```

### `speedy --daemon-restart`

```
speedy --daemon-restart /path/to/project
```

### `speedy --daemon-delete`

```
speedy --daemon-delete /path/to/project
```

### `speedy --daemon-create`

```
speedy --daemon-create /path/to/project
```

### `speedy --daemon-status`

```
speedy --daemon-status /path/to/project
```

### `speedy --force` / `-f`

```
speedy --force /path/to/project
speedy -f /path/to/project
```

Forces a full re-scan of the workspace.

### `speedy --workspaces` / `-w`

```
speedy --workspaces
speedy -w
```

Lists all registered workspaces.

### `speedy --workspace-create`

```
speedy --workspace-create /path/to/project
```

Creates a workspace and its daemon.

### `speedy --workspace-delete`

```
speedy --workspace-delete /path/to/project
```

Deletes a workspace and its daemon.

### `speedy index`

```
speedy index [<subdir>]
speedy index src/
```

| Argument | Default | Description |
|----------|---------|-------------|
| `subdir` | `.` (current dir) | Subdirectory to index |

### `speedy query`

```
speedy query <query>
speedy query --top-k 10 "error handling patterns"
```

| Argument / Flag | Default | Description |
|-----------------|---------|-------------|
| `query` | _(required)_ | Search query string |
| `-k, --top-k <n>` | `5` | Number of results to return |

### `speedy watch`

```
speedy watch [<subdir>]
speedy watch --detach
```

| Argument / Flag | Default | Description |
|-----------------|---------|-------------|
| `subdir` | `.` (current dir) | Subdirectory to watch |
| `--detach` | `false` | Run watcher in background (daemon mode) |

### `speedy daemon`

| Subcommand | Description |
|------------|-------------|
| `install [<path>]` | Register daemon to start at boot (requires workspace) |
| `uninstall` | Unregister daemon and stop it |
| `status` | Show daemon status |
| `list` | List all daemons |
| `create -p <path>` | Create a daemon for a workspace |
| `stop -p <path>` | Stop a daemon |
| `restart -p <path>` | Restart a daemon |
| `delete -p <path>` | Delete a daemon permanently |

> A daemon cannot exist without a workspace. Use `workspace create` first or `--workspace-create`.

### `speedy workspace`

| Subcommand | Description |
|------------|-------------|
| `list` | List all workspaces |
| `create -p <path>` | Create a workspace and its daemon |
| `delete -p <path>` | Delete a workspace and its daemon |

### `speedy force`

```
speedy force
speedy force -p /path/to/project
```

| Flag | Description |
|------|-------------|
| `-p` | Workspace path (default: current dir) |

### `speedy context`

```
speedy context
speedy context --json
```

No arguments. Shows a project context summary.

### `speedy sync`

```
speedy sync
```

No arguments. Incrementally syncs filesystem changes to the database.

## Examples

```bash
# Top-level action flags
speedy --read "how does the file watcher work?"
speedy -r "error handling patterns" --json
speedy --modify "console.log('hello world');" --file src/index.js
speedy -m "fn main() { println!(\"hello\"); }" --file main.rs
speedy --daemons
speedy -d
speedy --workspaces
speedy -w
speedy --daemon-status /path/to/project
speedy --daemon-stop /path/to/project
speedy --force /path/to/project
speedy -f /path/to/project

# Index current directory
speedy index

# Index a specific subdirectory
speedy index src/

# Semantic search with default top-K
speedy query "how does the file watcher work?"

# Semantic search with custom top-K
speedy query --top-k 10 "error handling patterns"

# JSON output
speedy query --json "authentication flow"

# Watch in foreground
speedy watch

# Watch in background daemon
speedy watch --detach

# Show context summary
speedy context

# Incremental sync
speedy sync

# Force reindex
speedy force
speedy force -p /path/to/project

# Workspace management
speedy workspace list
speedy workspace create -p /path/to/project
speedy workspace delete -p /path/to/project

# Top-level workspace management
speedy --workspace-create /path/to/project
speedy --workspace-delete /path/to/project

# Daemon management
speedy daemon list
speedy daemon status
speedy daemon create -p /path/to/project
speedy daemon install
speedy daemon uninstall
speedy daemon stop -p /path/to/project
speedy daemon restart -p /path/to/project
speedy daemon delete -p /path/to/project

# With --path (works with all commands)
speedy --path /path/to/project index
speedy -p /path/to/project --read "project overview"
speedy -p /path/to/project --modify "new content" --file file.txt
```

## Configuration

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `SPEEDY_MODEL` | `all-minilm:l6-v2` | Ollama embedding model |
| `SPEEDY_OLLAMA_URL` | `http://localhost:11434` | Ollama server URL |
| `SPEEDY_PROVIDER` | `ollama` | Embedding provider (`ollama` or `agent`) |
| `SPEEDY_AGENT_COMMAND` | *(empty)* | External command for `agent` provider |
| `SPEEDY_TOP_K` | `5` | Default top-K results |

### Config file

Speedy also reads configuration from `speedy.toml` or `.speedy/config.toml` (in the project root). Environment variables take precedence over the config file.

Example `speedy.toml`:

```toml
model = "nomic-embed-text"
ollama_url = "http://localhost:11434"
provider_type = "ollama"
top_k = 10
max_chunk_size = 500
```

| Key | Default | Description |
|-----|---------|-------------|
| `model` | `all-minilm:l6-v2` | Embedding model |
| `ollama_url` | `http://localhost:11434` | Ollama server URL |
| `provider_type` | `ollama` | Provider (`ollama` or `agent`) |
| `agent_command` | `""` | External command for `agent` provider |
| `top_k` | `5` | Default top-K results |
| `max_chunk_size` | `1000` | Max characters per chunk |
| `chunk_overlap` | `200` | Overlap between chunks |
| `watch_delay_ms` | `500` | Debounce delay for file watcher |
| `ignore_patterns` | `["target/", ".git/", ...]` | Glob patterns to ignore |

### Provider: `ollama` (default)

Uses Ollama's `/api/embeddings` endpoint. Requires Ollama running locally.

### Provider: `agent`

Delegates embedding generation to an external command. The command receives the text as its first argument and must output a JSON array of floats on stdout.

```bash
SPEEDY_PROVIDER=agent SPEEDY_AGENT_COMMAND=my-embedder speedy query "find relevant code"
```

## Ignoring files

Speedy uses the `ignore` crate to respect `.gitignore` patterns automatically. Additionally, you can create a `.speedyignore` file in your project root for Speedy-specific ignore patterns. It follows the same format as `.gitignore`:

```gitignore
# .speedyignore
build/
dist/
*.log
```

Files matching these patterns will be skipped during indexing.

## JSON output

Add `--json` for structured output:

```bash
speedy index --json
speedy query "auth flow" --json
speedy context --json
```

## Project structure

```
speedy-core/src/
├── bin/
│   └── speedy.rs        # main CLI binary
├── cli.rs               # clap argument definitions
├── config.rs            # configuration (env vars + toml file)
├── daemon.rs            # daemon lifecycle management
├── db.rs                # SQLite vector store
├── document.rs          # text chunking
├── embed.rs             # embedding providers (Ollama, Agent)
├── embedding.rs         # embedding data types
├── file.rs              # file type detection
├── hash.rs              # SHA-256 file hashing
├── ignore.rs            # gitignore-aware file filtering
├── indexer.rs           # indexing orchestration
├── lib.rs               # module declarations
├── text.rs              # text preprocessing utilities
├── watcher.rs           # filesystem watcher
└── workspace.rs         # workspace persistence
```

## Development

```bash
cargo test        # run all tests
cargo build       # compile debug
cargo clippy      # lint
```

## License

MIT
