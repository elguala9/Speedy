# Plugins — distributing the MCP servers for Claude Code & opencode

This document is for **developers/maintainers**. It explains how the
`speedy-lc` and `speedy-text` plugins are packaged, what is checked into the
repo, how releasing works, and what a maintainer must do to ship a new version.
For end-user install steps see [`plugins/README.md`](../plugins/README.md).

## What ships as a plugin

Two of Speedy's MCP servers are distributed as standalone plugins because they
are **self-contained**: single static binaries, no Ollama, no daemon, no system
libraries.

| Plugin | Binary | Tools | Purpose |
|--------|--------|-------|---------|
| `speedy-lc` | `speedy-language-context-mcp` | `get_skeleton`, `run_pipeline`, `index_status`, `force_reindex`, `workspace_*`, `save_observation`, `search_observations` | Code graph: skeletons + impact analysis |
| `speedy-text` | `speedy-text-context-mcp` | `text_query`, `text_replace`, `text_status`, `text_force_reindex` | Text-symbol search + project-wide rename |

> The full `speedy` server (`speedy-ai-context-mcp`) is **not** shipped this way:
> it needs Ollama for semantic search, so it can't be a drop-in single binary.

Both servers speak MCP over stdio (JSON-RPC 2.0, one message per line) and take
the workspace root from `-w <path>` or, if omitted, the current directory.

## Repo layout

```
.claude-plugin/
  marketplace.json            # marketplace "speedy" → lists both plugins
plugins/
  README.md                   # end-user install guide
  install.sh / install.ps1    # download a release tarball, extract the 2 MCP binaries
  speedy-lc/
    .claude-plugin/plugin.json
    .mcp.json                 # command + "-w ${CLAUDE_PROJECT_DIR}"
    skills/speedy-lc/SKILL.md # when/how the agent should use the tools
  speedy-text/
    .claude-plugin/plugin.json
    .mcp.json
    skills/speedy-text/SKILL.md
  opencode/
    opencode.json             # mcp block (type:"local") for both servers
```

The marketplace manifest lives at the **repo root** (`.claude-plugin/marketplace.json`)
so `/plugin marketplace add elguala9/Speedy` resolves it from the default branch.

## How distribution works

Distribution is split in two independent planes:

1. **Plugin metadata** (manifests, skills, opencode config, install scripts) —
   lives in the repo and becomes available the moment it is on the **default
   branch** (`main`). This is what makes the plugins *discoverable/installable*.
2. **The binaries** — are **not** in the plugin and **not** in git. They already
   ship inside the per-target `speedy-<target>.tar.gz` assets of every `v*`
   GitHub Release (produced by `.github/workflows/release.yml`). The install
   scripts download that tarball and extract just the two `*-mcp` binaries onto
   the user's `PATH`.

A Claude Code plugin's `.mcp.json` references the binary **by name on PATH** — it
does not bundle it. That's why the binary install is a separate, one-time step.

## Releasing a new version (maintainer checklist)

There is **no separate plugin release**. The plugin binaries ride along with the
normal Speedy release.

1. Bump the workspace version in `Cargo.toml` and tag `vX.Y.Z`.
2. Push the tag → `release.yml` builds and attaches `speedy-<target>.tar.gz`
   (containing both `*-mcp` binaries) for linux-x64, windows-x64, macos-x64.
3. Keep the plugin `version` fields in
   `plugins/speedy-lc/.claude-plugin/plugin.json` and
   `plugins/speedy-text/.claude-plugin/plugin.json` in sync with the release
   (cosmetic — shown in `/plugin`, not used to fetch binaries).
4. Merge the plugin changes to `main` so the marketplace/scripts reflect them.

The install scripts default to `releases/latest`; users can pin a version with
`TAG=vX.Y.Z` (`-Tag vX.Y.Z` on Windows).

### "Is merging to main enough?"

For the **plugin metadata**: yes — once on `main`, the marketplace is discoverable
and the install scripts are current. The binaries are already published in the
existing `v0.2.2` release, so nothing else is required to ship.

You only need a **new `vX.Y.Z` release** when you want users to get *updated
binaries*; metadata-only changes (skills, docs, manifests) just need `main`.

## End-user install (summary)

```text
# 1. binaries onto PATH (one-time)
bash plugins/install.sh                              # macOS/Linux
powershell -ExecutionPolicy Bypass -File plugins/install.ps1   # Windows
# (or: cargo install --git https://github.com/elguala9/Speedy speedy-language-context speedy-text)

# 2a. Claude Code
/plugin marketplace add elguala9/Speedy
/plugin install speedy-lc@speedy
/plugin install speedy-text@speedy

# 2b. opencode: merge plugins/opencode/opencode.json into your opencode.json
```

## Platform notes & known gaps

- Published targets: **linux-x64, windows-x64, macos-x64**. On Apple Silicon the
  x86_64 macOS binary runs under Rosetta. Add `aarch64-apple-darwin` to
  `release.yml` if a native arm64 build is wanted.
- The MCP `command` is resolved from `PATH`; spawning resolves the `.exe`
  extension on Windows, so the same `.mcp.json` works cross-platform.
- The index is built lazily under the project root and honors
  `.speedyignore` / `.gitignore`. Empty results usually mean "not indexed yet"
  → call `force_reindex` / `text_force_reindex`.
- Not yet smoke-tested end-to-end inside a live Claude Code / opencode install
  (tool registration); the binary download + execution path is verified.
