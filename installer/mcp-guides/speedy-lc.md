## Structure & impact — use the `speedy-lc` MCP before editing

- **Understand a file/symbol without reading it whole** →
  `mcp__speedy-lc__get_skeleton` with `files: [...]`. Start at `detail: "minimal"`
  (signatures only), step up to `"standard"` / `"detailed"` only when needed.
- **Plan a change / find what it breaks** → `mcp__speedy-lc__run_pipeline` with a
  `task` describing the change and a `preset`:
  - `explore` — just understand (shallow impact)
  - `modify` / `refactor` — deep impact, find all affected callers
  - `debug` — medium depth, focused on the failure path
- Run `run_pipeline` with `modify`/`refactor` **before** touching a widely-used
  symbol, then review the `impact` list of affected symbols.

Use `get_skeleton` instead of Read when you only need structure — far cheaper.
If `impact` is empty for a symbol you know is used, the index is stale →
`force_reindex`.

### Tools

| Tool | Params | Use |
|------|--------|-----|
| `get_skeleton` | `files` (req, string[]), `detail` (`minimal`\|`standard`\|`detailed`) | File/symbol structure |
| `run_pipeline` | `task` (req), `preset` (`auto`\|`explore`\|`modify`\|`debug`\|`refactor`), `top_k` (def 10) | Search + impact analysis |
| `index_status` / `force_reindex` | — | Index stats / full reindex |
| `workspace_add` / `_remove` / `_list` | `path` | Manage workspaces |
| `save_observation` / `search_observations` | note / query | Persist & recall codebase notes |
