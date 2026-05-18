# TODO: Nuovi tool MCP

## Obiettivo
Aggiungere tool per gestione workspace e reindex in entrambi i server MCP.

---

## 1. `speedy-mcp` (`packages/speedy-mcp/src/main.rs`)

Questo server delega al binario `speedy-cli` via `run_speedy(args)`.
Da aggiungere in `tools/list` e `tools/call`:

| Tool | Args | CLI sottostante |
|------|------|-----------------|
| `speedy_workspace_add` | `path` (req) | `speedy-cli workspace add <path>` |
| `speedy_workspace_remove` | `path` (req) | `speedy-cli workspace remove <path>` |
| `speedy_workspace_list` | — | `speedy-cli workspace list --json` |
| `speedy_force_reindex` | `path` (opt, default `.`) | `speedy-cli force -p <path>` |

**File da modificare:** `packages/speedy-mcp/src/main.rs`
- Aggiungere 4 entry in `tools/list` (vettore `tools`)
- Aggiungere 4 arm in `match name` dentro `tools/call`
- Aggiornare il test `test_tools_list_has_three_tools` → 7 tool
- Aggiornare il test `test_tools_list_names` → lista completa
- Aggiungere test per ogni nuovo tool (success + error path)

---

## 2. `speedy-language-context` (`packages/speedy-language-context/src/mcp.rs`)

Questo server ha accesso diretto a `GraphStore`, `Indexer` e `Memory`.
Non gestisce workspace multipli (è per-workspace), quindi:

| Tool | Args | Implementazione |
|------|------|-----------------|
| `force_reindex` | — | `indexer.full_index().await` |

> Nota: `workspace_add/remove/list` non hanno senso qui — questo server
> viene avviato su un singolo workspace root passato come argomento CLI.
> Se si vuole comunque esporli, andrebbero delegati a `speedy-cli` via
> `std::process::Command` (stessa cosa che fa speedy-mcp).

**File da modificare:** `packages/speedy-language-context/src/mcp.rs`
- Aggiungere `force_reindex` in `handle_tools_list`
- Aggiungere arm `"force_reindex"` in `handle_tools_call`
- La funzione chiama `indexer.full_index().await` e restituisce un summary
- Aggiornare il test `handle_tools_list_includes_all_tools`
- Aggiungere test `tool_force_reindex_runs_indexer`

---

## Checklist

- [x] `speedy-mcp`: aggiungere `speedy_workspace_add`
- [x] `speedy-mcp`: aggiungere `speedy_workspace_remove`
- [x] `speedy-mcp`: aggiungere `speedy_workspace_list`
- [x] `speedy-mcp`: aggiungere `speedy_force_reindex`
- [x] `speedy-mcp`: aggiornare test esistenti (count tool)
- [x] `speedy-mcp`: aggiungere test nuovi tool
- [x] `speedy-language-context`: aggiungere `force_reindex`
- [x] `speedy-language-context`: aggiornare test esistenti
- [x] `speedy-language-context`: aggiungere test `force_reindex`
- [x] `cargo test` su entrambi i package — tutto verde
