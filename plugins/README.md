# Speedy plugins

Two standalone, dependency-free code-intelligence MCP servers from the Speedy
project, packaged for **Claude Code** and **opencode**:

| Plugin | Binary | What it does |
|--------|--------|--------------|
| `speedy-lc` | `speedy-language-context-mcp` | Code graph: file/symbol skeletons + change-impact analysis |
| `speedy-text` | `speedy-text-context-mcp` | Text-symbol search + project-wide rename across docs/config |

Both are single static binaries — **no Ollama, no daemon, no system libraries**.
(This is the difference vs the full `speedy` server, which needs Ollama for
semantic search.)

## 1. Install the binaries

The plugins reference the binaries by name, so they must be on your `PATH`.
Pick one:

- **Download script (recommended):**
  - macOS / Linux: `bash plugins/install.sh`
  - Windows: `powershell -ExecutionPolicy Bypass -File plugins/install.ps1`
- **From source:** `cargo install --git https://github.com/elguala9/Speedy speedy-language-context speedy-text`
- **Windows installer:** the Speedy setup already puts both binaries on PATH.

Verify: `speedy-language-context-mcp --help` and `speedy-text-context-mcp --help`.

## 2a. Claude Code

```
/plugin marketplace add elguala9/Speedy
/plugin install speedy-lc@speedy
/plugin install speedy-text@speedy
```

Each plugin registers its MCP server (workspace = `${CLAUDE_PROJECT_DIR}`) and
ships a skill telling the agent when and how to use the tools.

## 2b. opencode

Merge `plugins/opencode/opencode.json` into your `opencode.json` (global
`~/.config/opencode/opencode.json` or per-project). opencode launches local MCP
servers in the project directory, so no workspace argument is needed.

## First run

The index is built lazily. If a query returns nothing, the workspace isn't
indexed yet — call `force_reindex` (speedy-lc) / `text_force_reindex`
(speedy-text). Index files live under the project root and honor
`.speedyignore` / `.gitignore`.

## Releasing new binaries

No separate plugin release is needed: the two MCP binaries already ship inside
the per-target `speedy-<target>.tar.gz` tarballs of every `v*` release
(built by `.github/workflows/release.yml`). The install scripts download the
tarball for the current platform and extract just the two `*-mcp` binaries.
Override the version with `TAG=v0.2.2` (`-Tag v0.2.2` on Windows).

Available targets: linux-x64, windows-x64, macos-x64. On Apple Silicon the
x86_64 macOS build runs under Rosetta.
