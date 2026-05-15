# speedy-mcp

MCP server that exposes [Speedy](https://github.com/elguala9/Speedy) semantic search as tools for AI coding agents.

Compatible with any MCP client: opencode, Claude Code, Cursor, Windsurf, and more.

## Tools

| Tool | Description |
|---|---|
| `speedy_query` | Semantic search over the codebase using natural language |
| `speedy_index` | Index a directory into the vector database |
| `speedy_context` | Show project context summary |

## Install

```bash
cargo install speedy-mcp
```

## Usage

Add to your MCP client config:

```json
{
  "mcpServers": {
    "speedy": {
      "command": "speedy-mcp",
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
      "command": "C:\\Program Files\\Speedy\\speedy-mcp.exe",
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
