# speedy-language-context-mcp

Standalone MCP server for Speedy code intelligence. Exposes symbol search, file skeletons, impact analysis, and observation memory to any MCP-compatible agent (Claude Code, Claude Desktop, Cursor, Windsurf, opencode, …).

Part of the [Speedy](https://github.com/elguala9/Speedy) release bundle.

## Quick start

```
speedy-language-context-mcp --workspace /path/to/project
```

The server reads JSON-RPC 2.0 messages from stdin and writes responses to stdout (one message per line). It opens (or creates) `.speedy/slc.sqlite` inside the workspace — no daemon, no IPC.

## Client configuration

### Claude Code (`.claude/settings.json`)

```json
{
  "mcpServers": {
    "speedy-lang": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "/absolute/path/to/project"]
    }
  }
}
```

### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "speedy-lang": {
      "command": "C:\\Program Files\\Speedy\\speedy-language-context-mcp.exe",
      "args": ["--workspace", "C:\\path\\to\\project"]
    }
  }
}
```

### VS Code (`.vscode/mcp.json`)

```json
{
  "servers": {
    "speedy-lang": {
      "type": "stdio",
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "${workspaceFolder}"]
    }
  }
}
```

### Running both Speedy MCP servers together

```json
{
  "mcpServers": {
    "speedy-ai": {
      "command": "speedy-ai-context-mcp",
      "args": []
    },
    "speedy-lang": {
      "command": "speedy-language-context-mcp",
      "args": ["--workspace", "/path/to/project"]
    }
  }
}
```

`speedy-ai-context-mcp` handles semantic vector search; `speedy-language-context-mcp` handles symbol graph, skeletons, and memory. They communicate only via stdio and share no sockets.

## Available tools

| Tool | Description |
|------|-------------|
| `index_status` | File/symbol/edge counts and `last_indexed` timestamp |
| `get_skeleton` | Compact file structure. Args: `files[]`, `detail` (`minimal`\|`standard`\|`detailed`) |
| `run_pipeline` | Search + impact analysis for a free-form task. Args: `task`, `preset`, `top_k` |
| `save_observation` | Persist a note to the FTS-backed memory store. Args: `content` |
| `search_observations` | Full-text search over saved notes. Args: `query`, `limit` |

## Presets for `run_pipeline`

| Preset | Impact depth | Use for |
|--------|--------------|---------|
| `auto` | 1 | General exploration |
| `explore` | 1 | Read-only investigation |
| `modify` | 3 | Planning a change |
| `refactor` | 3 | Large structural changes |
| `debug` | 2 | Tracing a bug |

## Protocol

- JSON-RPC 2.0 over stdio, one message per line
- MCP protocol version: `2025-03-26`
- Server name reported in `initialize`: `speedy-language-context`

## Data

All data lives in `<workspace>/.speedy/slc.sqlite` (SQLite, WAL mode). Safe to delete — the next index run recreates it.
