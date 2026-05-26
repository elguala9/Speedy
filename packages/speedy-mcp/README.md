# speedy-ai-context-mcp

MCP server that exposes [Speedy](https://github.com/elguala9/Speedy) semantic search as tools for AI coding agents.

Compatible with any MCP client: opencode, Claude Code, Cursor, Windsurf, and more.

## Tools

| Tool | Parameters | Description |
|---|---|---|
| `speedy_query` | `query` (req), `top_k` (opt, default 5), `mode` (opt, `full`\|`files`, default `full`) | Semantic/conceptual search using natural language. Requires an embedding model (Ollama or configured provider). Use for "where is X logic?" questions when you don't know the exact symbol name. Use `mode: "files"` for token-efficient location queries |
| `speedy_grep` | `pattern` (req), `top_k` (opt, default 20) | Keyword/phrase search using SQLite FTS5. **No embedding model required** — works even without Ollama. Use for exact symbol names, function signatures, string literals. Supports FTS5 syntax: `fn*` (prefix), `"phrase search"`, `fn OR struct` |
| `speedy_index` | `path` (opt, default `.`) | Index a directory into the vector database |
| `speedy_context` | — | Show project context summary |
| `speedy_workspace_add` | `path` (req) | Add a directory to the workspace registry |
| `speedy_workspace_remove` | `path` (req) | Remove a directory from the workspace registry |
| `speedy_workspace_list` | — | List all registered workspaces |
| `speedy_force_reindex` | `path` (opt, default `.`) | Force a full reindex of a workspace |

## Install

```bash
cargo install speedy-ai-context-mcp
```

## Usage

Add to your MCP client config:

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-ai-context-mcp",
      "args": []
    }
  }
}
```

Set `SPEEDY_BIN` env var to point to the speedy binary if not in PATH.

### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "speedy": {
      "command": "C:\\Program Files\\Speedy\\speedy-ai-context-mcp.exe",
      "args": [],
      "env": {
        "SPEEDY_BIN": "speedy-cli",
        "SPEEDY_DEFAULT_SOCKET": "speedy-daemon",
        "SPEEDY_MCP_TOP_K": "10"
      }
    }
  }
}
```

> Full per-binary flag reference: [`commands.md`](../../commands.md).  
> Environment variables and config file options: [`README.md`](../../README.md#configuration).
