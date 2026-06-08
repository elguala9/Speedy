## Code search — use the `speedy` MCP

Search the index before reading files or running `grep`/`rg`.

- **Conceptual question** ("where is the auth logic?", "how is the DB pool set up?")
  → `mcp__speedy__speedy_query` with a natural-language query (needs Ollama).
- **Exact symbol / string** ("find `fn authenticate`", a log message, a config key)
  → `mcp__speedy__speedy_grep`. Faster, works without Ollama.
- **"Which file is X in?"** → `speedy_query` with `mode: "files"` — returns only paths.
- **New session / project orientation** → `mcp__speedy__speedy_context`.

Only Read a file once search narrowed it to the relevant path(s). Use `top_k` 10
for broad questions, 3–5 when you roughly know where the answer is. If results
are empty the workspace isn't indexed — run `speedy_index`.

### Tools

| Tool | Params | Use |
|------|--------|-----|
| `speedy_query` | `query` (req), `top_k` (def 5), `mode` (`full`\|`files`) | Conceptual search; `mode:"files"` = paths only, cheap |
| `speedy_grep` | `pattern` (req), `top_k` (def 20) | Exact match; supports `fn*`, `"phrase"`, `a OR b`, `NOT` |
| `speedy_context` | — | Project summary |
| `speedy_index` / `speedy_force_reindex` | `path` (opt) | Index / full reindex |
| `speedy_workspace_add` / `_remove` / `_list` | `path` | Manage workspaces |
