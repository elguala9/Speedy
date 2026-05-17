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

Use the dedicated `speedy-language-context-mcp` binary to register the server in your MCP client config. See **[README-mcp.md](README-mcp.md)** for full configuration examples (Claude Code, Claude Desktop, VS Code) and tool reference.

## Feature toggles

The daemon controls whether SLC indexing is active per workspace via `<workspace>/.speedy/config.toml`:

```toml
[features]
language_context = true   # SLC — default: true
speedy_indexer = true     # file indexer — default: true
```

Use the **speedy-gui** Workspaces tab to toggle these with checkboxes (changes are written to the same file immediately).

> CLI shorthand (`speedy enable slc`, `speedy disable slc`, `speedy features`) is planned but not yet implemented.

## Data directory

All data lives in `<workspace>/.speedy/slc.sqlite` (SQLite, WAL mode). The file is safe to delete; the next `index` run recreates it.

## Supported languages

Rust · TypeScript · JavaScript · JSX · TSX · Python · Go
