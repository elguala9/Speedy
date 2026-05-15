# speedy-language-context

Local code-intelligence engine for AI assistants. Builds a symbol graph of your workspace using tree-sitter and exposes it via an MCP (Model Context Protocol) server.

## Features

- Symbol extraction for Rust, TypeScript, JavaScript, Python, Go (JSX/TSX included)
- Call-site edge detection (same-file)
- BM25 symbol search
- File skeleton rendering at three detail levels
- Impact analysis (reverse BFS from a symbol)
- FTS5-backed observation memory
- MCP server over stdio (JSON-RPC 2.0)

## Installation

Binary is distributed with the Speedy release bundle. Place `speedy-language-context` (or `.exe` on Windows) in your `PATH` or alongside the `speedy-daemon` binary.

## CLI usage

```
# Index the workspace (run once, then the daemon keeps it fresh)
speedy-language-context --path /path/to/workspace index

# Incremental update after a git hook or manual change
speedy-language-context --path /path/to/workspace update src/main.rs src/lib.rs

# Print counts and last indexed timestamp
speedy-language-context --path /path/to/workspace status

# Search for a symbol
speedy-language-context --path /path/to/workspace search "handle_request" --top-k 5

# Render a file skeleton
speedy-language-context --path /path/to/workspace skeleton src/lib.rs --detail standard

# Start the MCP server (stdio, one JSON-RPC message per line)
speedy-language-context --path /path/to/workspace serve
```

All commands accept `--json` to emit machine-readable output.

## MCP server

### Configuring in Claude Code

Add to `.claude/settings.json` (project) or `~/.claude/settings.json` (global):

```json
{
  "mcpServers": {
    "speedy-language-context": {
      "command": "speedy-language-context",
      "args": ["--path", "/absolute/path/to/workspace", "serve"]
    }
  }
}
```

Or in `claude_desktop_config.json` for Claude Desktop:

```json
{
  "mcpServers": {
    "speedy-language-context": {
      "command": "/usr/local/bin/speedy-language-context",
      "args": ["--path", "/absolute/path/to/workspace", "serve"]
    }
  }
}
```

### Available tools

| Tool | Description |
|------|-------------|
| `index_status` | Returns file/symbol/edge counts and `last_indexed` timestamp |
| `get_skeleton` | Renders file skeletons. Args: `files[]`, `detail` (`minimal`\|`standard`\|`detailed`) |
| `run_pipeline` | Search + impact analysis for a free-form task. Args: `task`, `preset`, `top_k` |
| `save_observation` | Persist a note about the codebase to the FTS memory store |
| `search_observations` | Full-text search over saved observations. Args: `query`, `limit` |

### Presets for `run_pipeline`

| Preset | Impact depth | Use for |
|--------|-------------|---------|
| `auto` | 1 | General exploration |
| `explore` | 1 | Read-only investigation |
| `modify` | 3 | Planning a change |
| `refactor` | 3 | Large structural changes |
| `debug` | 2 | Tracing a bug |

## Feature toggles

The daemon controls whether SLC indexing is active per workspace:

```
speedy enable slc      # enable in current directory
speedy disable slc     # disable in current directory
speedy features        # show current status
```

Settings are written to `<workspace>/.speedy/config.toml` under `[features]`.

## Data directory

All data lives in `<workspace>/.speedy/slc.db` (SQLite, WAL mode). The file is safe to delete; the next `index` run recreates it.

## Supported languages

Rust · TypeScript · JavaScript · JSX · TSX · Python · Go
