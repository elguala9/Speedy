## Text symbol search & rename — use the `speedy-text` MCP

For tokens in docs/config files (Markdown, JSON, YAML, `.env`, `.toml`, …):

- **Find every occurrence of a token** → `mcp__speedy-text__text_query` with
  `symbol`. Returns file + line + column.
- **Rename a token across the whole project** → `mcp__speedy-text__text_replace`.
  Rewrites every occurrence and re-indexes in one call. **Always pass
  `dry_run: true` first**, check the count, then run for real.
- Use `whole_token_only: true` to skip substrings inside larger tokens
  (rename `Dummy` without touching `FooDummyBar`).
- Narrow scope with `ext` (e.g. `"md"`) or `files: [...]`.

Prefer this over hand-editing many files for a rename. If `text_query` finds
nothing for a visible symbol, the workspace isn't indexed → `text_force_reindex`.

### Tools

| Tool | Params | Use |
|------|--------|-----|
| `text_query` | `symbol` (req), `type`, `ext`, `ignore_case` | Find occurrences with positions |
| `text_replace` | `symbol` (req), `replacement` (req), `type`, `ext`, `ignore_case`, `whole_token_only`, `force`, `files`, `dry_run` | Replace + re-index |
| `text_status` / `text_force_reindex` | — | Index stats / rebuild index |

`type` = `cased` (default, case-sensitive) \| `isolated_special` \| `isolated`.
`ignore_case` affects search only; `replacement` is written literally.
`force: true` bypasses the 500-occurrence safety limit.
