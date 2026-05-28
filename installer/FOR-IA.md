# FOR-IA.md — AI Agent Guide: Configure Speedy MCP

This file is written for AI coding agents (Claude Code, Cursor, Windsurf, opencode, etc.)
to detect, configure, and use Speedy's MCP servers on a machine where Speedy is already
installed.

---

## What Speedy is

Speedy is a local semantic search engine for codebases. It runs a background daemon that
watches registered workspaces, indexes files into a local vector database, and exposes
search through a CLI and two MCP servers.

---

## Step 0 — Prerequisite: Ollama

Speedy uses **Ollama** as the default embedding provider.
If Ollama is not already installed and the embedding model not pulled, indexing will fail.

```bash
# Verify Ollama is running
ollama --version

# Pull the default embedding model (once only)
ollama pull all-minilm
```

Ollama runs an HTTP server on `http://localhost:11434` by default. No configuration is needed if it is running on the default port.

---

## Step 1 — Verify Speedy is installed

### Windows (installer or manual)

```powershell
# Check if speedy-cli is on PATH
where.exe speedy-cli

# Default install path (installer)
Test-Path "$env:LOCALAPPDATA\Programs\Speedy\speedy-cli.exe"

# Check daemon status
speedy-cli daemon status
# Expected output: {"status":"running",...}

# If speedy-cli is not on PATH, use the full path:
& "$env:LOCALAPPDATA\Programs\Speedy\speedy-cli.exe" daemon status
```

### Linux / macOS

```bash
which speedy-cli
speedy-cli daemon status
```

### Start the daemon if it is not running

```powershell
# Windows
Start-Process "$env:LOCALAPPDATA\Programs\Speedy\speedy-daemon.exe" -WindowStyle Hidden

# Linux / macOS
speedy-daemon &
```

---

## Step 2 — Register and index the current workspace

```bash
# Register the workspace (run once per project)
speedy-cli workspace add /absolute/path/to/project

# Index the project (run from the project root)
cd /absolute/path/to/project
speedy-cli index
```

On Windows:

```powershell
speedy-cli workspace add C:\path\to\project
cd C:\path\to\project
speedy-cli index
```

Verify the workspace is registered:

```bash
speedy-cli workspace list
```

---

## Step 3 — Add Speedy MCP servers to your AI agent

Speedy ships three independent MCP servers. Add any or all.

| Server binary              | Purpose                                          |
|----------------------------|--------------------------------------------------|
| `speedy-ai-context-mcp`    | Semantic search (natural language queries)       |
| `speedy-language-context-mcp` | Code graph: symbol skeletons, impact analysis |
| `speedy-text-context-mcp`  | Text-symbol search over docs/config files        |

### Claude Code — `.claude/settings.json` or `claude.json`

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-ai-context-mcp",
      "args": []
    },
    "speedy-lc": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "C:\\path\\to\\your\\project"]
    },
    "speedy-text": {
      "command": "speedy-text-context-mcp",
      "args": ["--workspace", "C:\\path\\to\\your\\project"]
    }
  }
}
```

If the binaries are not on PATH, use the full path:

```json
{
  "mcpServers": {
    "speedy": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Programs\\Speedy\\speedy-ai-context-mcp.exe",
      "args": [],
      "env": {
        "SPEEDY_BIN": "C:\\Users\\<you>\\AppData\\Local\\Programs\\Speedy\\speedy-cli.exe"
      }
    },
    "speedy-lc": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Programs\\Speedy\\speedy-language-context-mcp.exe",
      "args": ["--workspace", "C:\\path\\to\\your\\project"]
    },
    "speedy-text": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Programs\\Speedy\\speedy-text-context-mcp.exe",
      "args": ["--workspace", "C:\\path\\to\\your\\project"]
    }
  }
}
```

### Cursor / Windsurf / opencode — `mcp.json`

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-ai-context-mcp",
      "args": [],
      "env": {
        "SPEEDY_MCP_TOP_K": "10"
      }
    },
    "speedy-lc": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "/path/to/your/project"]
    },
    "speedy-text": {
      "command": "speedy-text-context-mcp",
      "args": ["--workspace", "/path/to/your/project"]
    }
  }
}
```

---

## MCP tools available

### speedy-ai-context-mcp (semantic search)

| Tool                    | Parameters (key ones)                                              | Description                                         |
|-------------------------|--------------------------------------------------------------------|-----------------------------------------------------|
| `speedy_query`          | `query` (req), `top_k` (opt, def 5), `mode` (`full`\|`files`, def `full`) | Semantic/conceptual search. **Requires embedding model (Ollama).** Use for "where is X logic?" questions. Use `mode:"files"` to get only unique paths — cheaper in tokens |
| `speedy_grep`           | `pattern` (req), `top_k` (opt, def 20)                            | Keyword/phrase search via SQLite FTS5. **No Ollama required.** Use for exact symbol names, function signatures, string literals. Supports `fn*` prefix, `"phrase"`, `fn OR struct` |
| `speedy_index`          | —                                                                  | Index a directory                                   |
| `speedy_context`        | —                                                                  | Project context summary                             |
| `speedy_workspace_add`  | `path` (req)                                                       | Register a workspace                                |
| `speedy_workspace_remove` | `path` (req)                                                     | Unregister a workspace                              |
| `speedy_workspace_list` | —                                                                  | List all registered workspaces                      |
| `speedy_force_reindex`  | `path` (opt)                                                       | Force a full reindex                                |

### speedy-language-context-mcp (code graph)

| Tool                  | Description                                           |
|-----------------------|-------------------------------------------------------|
| `run_pipeline`        | Semantic search + impact analysis for a task          |
| `get_skeleton`        | File/symbol structure at configurable detail levels   |
| `index_status`        | Current index stats                                   |
| `force_reindex`       | Force a full reindex                                  |
| `workspace_add`       | Register a workspace                                  |
| `workspace_remove`    | Unregister a workspace                                |
| `workspace_list`      | List all workspaces                                   |
| `save_observation`    | Save a note about the codebase                        |
| `search_observations` | Search saved notes                                    |

### speedy-text-context-mcp (text-symbol search)

| Tool                  | Parameters (key ones)                                  | Description                                          |
|-----------------------|--------------------------------------------------------|------------------------------------------------------|
| `text_query`          | `symbol` (req), `type` (opt), `ext` (opt), `ignore_case` (opt) | Find occurrences of a symbol in docs/config files, with line/column positions |
| `text_status`         | —                                                      | Index stats: file, occurrence and unique-symbol counts |
| `text_force_reindex`  | —                                                      | Drop the text index and re-index the workspace        |

---

## Environment variables

| Variable               | Default         | Effect                                          |
|------------------------|-----------------|-------------------------------------------------|
| `SPEEDY_BIN`           | `speedy-cli`    | Full path to speedy-cli (used by MCP servers)   |
| `SPEEDY_MCP_TOP_K`     | `5`             | Default result count for `speedy_query`         |
| `SPEEDY_DEFAULT_SOCKET`| `speedy-daemon` | Daemon socket name                              |
| `SPEEDY_MODEL`         | built-in        | Embedding model name                            |

---

## Data locations

| Platform | Path                                              | Contents                        |
|----------|---------------------------------------------------|---------------------------------|
| Windows  | `%APPDATA%\speedy\`                               | workspaces.json, daemon logs    |
| Windows  | `%USERPROFILE%\.speedy\`                          | Global user config              |
| All      | `<project>\.speedy\`                              | Per-project SQLite + embeddings |
| Linux    | `~/.config/speedy/`                               | Config                          |

---

## Troubleshooting

**Daemon not running**

```bash
speedy-cli daemon status   # check
speedy-daemon &            # start (Linux/macOS)
```

```powershell
Start-Process "$env:LOCALAPPDATA\Programs\Speedy\speedy-daemon.exe" -WindowStyle Hidden
```

**speedy-cli not on PATH (Windows)**

Add the install directory to PATH or use the full path. The installer adds
`%LOCALAPPDATA%\Programs\Speedy` to the user PATH — restart your terminal after install.

**MCP server exits immediately**

Set `SPEEDY_BIN` to the full path of `speedy-cli.exe` in the MCP server's `env` block.

**Empty search results**

The workspace may not be indexed. Run:

```bash
speedy-cli workspace add /path/to/project
speedy-cli index
```

---

## Quick reference: useful CLI commands

```bash
speedy-cli daemon status              # daemon status
speedy-cli workspace list             # list registered workspaces
speedy-cli workspace add <path>       # register a workspace
speedy-cli index                      # index current directory
speedy-cli query "authentication"     # semantic search (requires Ollama)
speedy-cli query "database pool" -k 10  # top 10 results
speedy-cli grep "fn authenticate"     # keyword search (no Ollama required)
speedy-cli grep "\"error handling\""  # phrase search
speedy-cli context                    # project context summary
speedy-cli force                      # force full reindex
```
