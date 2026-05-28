# SETTINGS_MCP.md — AI Agent Task: Configure Speedy MCP

> **Read this file end-to-end before doing anything.**
> Your job is to register Speedy's MCP servers in the user's AI coding tool
> (Claude Code, Cursor, Windsurf, opencode, Claude Desktop, …) by editing the
> tool's settings/config JSON. Speedy is **already installed** — you only edit
> client config. Do **not** install Speedy, do **not** start the daemon, do
> **not** index workspaces. That belongs to other guides.

---

## Step 1 — Resolve the binary paths

Speedy ships two MCP server binaries. You must use **absolute paths** in the
config (clients do not always inherit the user PATH).

### Windows (default installer location)

```
C:\Users\<USERNAME>\AppData\Local\Programs\Speedy\speedy-ai-context-mcp.exe
C:\Users\<USERNAME>\AppData\Local\Programs\Speedy\speedy-language-context-mcp.exe
C:\Users\<USERNAME>\AppData\Local\Programs\Speedy\speedy-cli.exe
```

Resolve `<USERNAME>` to the actual value of `$env:USERNAME` or
`%USERPROFILE%`. Verify each binary exists with `Test-Path` before writing
config — abort with a clear error if any is missing.

### Linux / macOS

The binaries are typically on PATH after a manual install
(`/usr/local/bin/` or `~/.local/bin/`). Resolve with:

```bash
which speedy-ai-context-mcp
which speedy-language-context-mcp
which speedy-cli
```

If `which` fails, ask the user where Speedy is installed. Do not guess.

### Custom install paths

Some users install Speedy elsewhere. If the default locations above don't
exist, ask the user for the install directory before writing anything.

---

## Step 2 — Choose the right config file

Pick the file for the client you are configuring. **Do not invent paths.**

| Client                | Config file (Windows)                                         | Config file (Linux/macOS)                                |
|-----------------------|---------------------------------------------------------------|----------------------------------------------------------|
| Claude Code (user)    | `%USERPROFILE%\.claude\settings.json`                         | `~/.claude/settings.json`                                |
| Claude Code (project) | `<project>\.claude\settings.local.json`                       | `<project>/.claude/settings.local.json`                  |
| Claude Desktop        | `%APPDATA%\Claude\claude_desktop_config.json`                 | `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) |
| Cursor                | `%USERPROFILE%\.cursor\mcp.json`                              | `~/.cursor/mcp.json`                                     |
| Windsurf              | `%USERPROFILE%\.codeium\windsurf\mcp_config.json`             | `~/.codeium/windsurf/mcp_config.json`                    |
| opencode              | `%USERPROFILE%\.config\opencode\opencode.json`                | `~/.config/opencode/opencode.json`                       |

If the file does not exist:

1. Create its **parent directory** first (e.g. `~/.claude/`, `%APPDATA%\Claude\`,
   `~/.cursor/`). Most clients do NOT auto-create their config dir until the
   first launch — so the directory might be missing entirely.
   - PowerShell: `New-Item -ItemType Directory -Force -Path <parent-dir>`
   - bash: `mkdir -p <parent-dir>`
2. Create the file with `{}` as initial content.
3. Then proceed to merge as below.

If the file exists, **read it first**, merge keys preserving existing entries,
and write back valid JSON.

---

## Step 3 — Apply the config

Add the following `mcpServers` block. Merge with any existing
`mcpServers` — **do not overwrite** other servers the user has configured.

### Recommended block (semantic search + code graph)

Replace `<ABS_PATH_TO_SPEEDY_DIR>` with the directory containing the binaries.
The default installer location on Windows is
`C:\Users\<USERNAME>\AppData\Local\Programs\Speedy` — verify it with
`Test-Path` first. In JSON on Windows, every `\` becomes `\\`.

**Windows example** (fully escaped, default installer path, username = `luca`):

```json
{
  "mcpServers": {
    "speedy": {
      "command": "C:\\Users\\luca\\AppData\\Local\\Programs\\Speedy\\speedy-ai-context-mcp.exe",
      "args": [],
      "env": {
        "SPEEDY_BIN": "C:\\Users\\luca\\AppData\\Local\\Programs\\Speedy\\speedy-cli.exe",
        "SPEEDY_MCP_TOP_K": "10"
      }
    },
    "speedy-lc": {
      "command": "C:\\Users\\luca\\AppData\\Local\\Programs\\Speedy\\speedy-language-context-mcp.exe",
      "args": []
    },
    "speedy-text": {
      "command": "C:\\Users\\luca\\AppData\\Local\\Programs\\Speedy\\speedy-text-context-mcp.exe",
      "args": []
    }
  }
}
```

Alternative on Windows: use forward slashes (`/`) — the Windows binaries
accept them and JSON does not require escaping. This avoids the `\\` trap
entirely:

```json
"command": "C:/Users/luca/AppData/Local/Programs/Speedy/speedy-ai-context-mcp.exe"
```

On Linux/macOS drop the `.exe` suffix:

```json
{
  "mcpServers": {
    "speedy": {
      "command": "/usr/local/bin/speedy-ai-context-mcp",
      "args": [],
      "env": {
        "SPEEDY_BIN": "/usr/local/bin/speedy-cli",
        "SPEEDY_MCP_TOP_K": "10"
      }
    },
    "speedy-lc": {
      "command": "/usr/local/bin/speedy-language-context-mcp",
      "args": []
    },
    "speedy-text": {
      "command": "/usr/local/bin/speedy-text-context-mcp",
      "args": []
    }
  }
}
```

### Minimal block (semantic search only)

If the user explicitly says they only want semantic search, omit `speedy-lc`.
Do not omit it on your own initiative — both servers together are the
default and they are independent.

---

## Step 4 — Decision rules

These are the rules you must follow when editing the config file:

1. **Read existing content first.** Never blind-overwrite. Parse the JSON,
   merge under `mcpServers`, preserve every other top-level key
   (`permissions`, `hooks`, `env`, anything custom).
2. **Idempotency.** If `speedy` and/or `speedy-lc` already exist under
   `mcpServers`, update only their `command`/`args`/`env` to the values
   above. Do not duplicate.
3. **Naming.** Use exactly `speedy` and `speedy-lc` as the server keys.
   Tools will appear in the host as `mcp__speedy__speedy_query`,
   `mcp__speedy-lc__run_pipeline`, etc.
4. **Path escaping.** In JSON on Windows, every `\` becomes `\\`. Use
   forward slashes (`/`) as an alternative — Windows binaries accept them.
5. **No PATH assumption.** Always emit absolute paths in `command` and
   `SPEEDY_BIN`. Several clients spawn the MCP server with an empty PATH.
6. **JSON validity.** After writing, parse the file again to confirm it is
   still valid JSON. If parsing fails, restore the backup (step 5 below).
7. **Backup.** Before overwriting an existing file, copy it to
   `<file>.bak-speedy-<timestamp>`. Keep the backup; do not delete it.

---

## Step 5 — Per-client quirks

- **Claude Code:** after editing `settings.json`, the user must restart
  Claude Code. Tell them.
- **Cursor:** `mcp.json` is read on startup; restart Cursor.
- **Windsurf:** uses `mcp_config.json`; restart Windsurf.
- **opencode:** the MCP block lives inside `opencode.json` under
  `"mcp": { "speedy": { ... } }` — schema differs slightly. If the user
  asks for opencode, verify their version's schema before writing.
- **Claude Desktop:** restart the desktop app, not just close the window.

---

## Step 6 — Verify

After editing, **do not run the MCP server yourself**. Instead, instruct the
user to:

1. Restart the client (Claude Code / Cursor / …).
2. Open a new chat / session in the client.
3. Ask the client to call `speedy_context` — if it returns a project
   summary, configuration is correct.

If you want a non-interactive smoke test of the binary itself, you may run:

```powershell
# Windows
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | & "<ABS_PATH_TO_SPEEDY_DIR>\speedy-ai-context-mcp.exe"
```

Expected: a one-line JSON response with `"serverInfo":{"name":"speedy-ai-context-mcp",...}`.
A non-zero exit or no output means the binary itself is broken — escalate to
the user, do not retry the config.

---

## What the tools look like once configured

| Server key (in config) | Tools exposed to the client                                              |
|------------------------|--------------------------------------------------------------------------|
| `speedy`               | `speedy_query` (semantic, needs Ollama), `speedy_grep` (FTS5 keyword, no Ollama), `speedy_index`, `speedy_context`, `speedy_workspace_add`, `speedy_workspace_remove`, `speedy_workspace_list`, `speedy_force_reindex`, `speedy_lc_status`, `speedy_lc_skeleton` |
| `speedy-lc`            | `run_pipeline`, `get_skeleton`, `index_status`, `force_reindex`, `workspace_add`, `workspace_remove`, `workspace_list`, `save_observation`, `search_observations` |

In the host (e.g. Claude Code) these appear with the prefix
`mcp__<server-key>__<tool-name>`, e.g. `mcp__speedy__speedy_query`.

---

## Optional — environment variables

| Variable               | Where           | Effect                                                |
|------------------------|-----------------|-------------------------------------------------------|
| `SPEEDY_BIN`           | `env` of `speedy` | Absolute path to `speedy-cli`. **Recommended.**     |
| `SPEEDY_MCP_TOP_K`     | `env` of `speedy` | Default `top_k` for `speedy_query` (default `5`)    |
| `SPEEDY_DEFAULT_SOCKET`| `env` of either | Daemon socket name override (rarely needed)           |

Set `SPEEDY_MCP_TOP_K=10` by default — it gives the AI richer recall without
much downside.

### Choosing the right search tool

| Question type | Tool to use | Why |
|---|---|---|
| "Where is the authentication logic?" | `speedy_query` | Conceptual, no exact symbol known |
| "Find all calls to `fn authenticate`" | `speedy_grep` | Exact symbol — faster, no Ollama needed |
| "In which file is X handled?" | `speedy_query` with `mode:"files"` | Semantic location, token-efficient |
| Ollama is unavailable | `speedy_grep` | FTS5 works without any embedding model |

### Token-efficient queries with `mode: "files"`

`speedy_query` accepts an optional `mode` parameter:

- `"full"` (default) — returns full chunks: `[{path, line, text, score}, ...]` — use when you need the actual code
- `"files"` — returns only unique file paths: `[{path, score}, ...]` — use for "in which file is X?" questions

Example tool call (in Claude Code / MCP JSON):
```json
{"query": "authentication middleware", "mode": "files", "top_k": 5}
```

`mode: "files"` reduces output from ~2000-5000 tokens to ~50-150 tokens for location queries.

### `speedy_grep` — keyword search without Ollama

`speedy_grep` searches the indexed text using SQLite FTS5. No embedding model required.

```json
{"pattern": "fn authenticate", "top_k": 20}
{"pattern": "\"error handling\"", "top_k": 10}
{"pattern": "fn*", "top_k": 30}
```

FTS5 syntax quick reference: bare words match anywhere in a chunk, quoted strings are phrase searches, `*` suffix is a prefix match, `OR`/`AND`/`NOT` are boolean operators.

---

## What this file is *not*

- Not an installer guide → see `INSTALLATION.md`.
- Not a Speedy setup/usage guide → see `FOR-IA.md`.
- Not a config reference for Speedy itself → see `CONFIG.md` (root of repo).

Only edit MCP client config. Nothing else.
