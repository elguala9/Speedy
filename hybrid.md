# Speedy — Hybrid Plan: Daemon + Git Hooks

> **⚠️ Superseded — see [`diagram-flow.md`](./diagram-flow.md) for the shipped flow.**
> The embedded scripts below (with the "daemon up? → IPC / daemon down? →
> standalone" branch, and one block per worker) are **historical**. As shipped,
> the templates in `scripts/git-hooks/*.tpl`:
> - **route through `speedy-cli`** (one orchestration point) with
>   `SPEEDY_NO_DAEMON=1`, never the individual workers;
> - fan out to **all enabled contexts** (ai-context, language-context,
>   text-context) — contexts are opt-in, so each is a no-op when off;
> - `post-commit` runs `speedy-cli update <changed files>` (per-file);
>   `post-checkout` / `post-merge` / `post-rewrite` run `speedy-cli sync`
>   (incremental, mtime+hash skip, prunes deletions);
> - are **installed automatically by `speedy-cli workspace add`** and removed by
>   `workspace remove` (still also available via `speedy install-hooks` /
>   `uninstall-hooks`).
>
> This document remains as a reference for the original hybrid design.

**Goal (historical)**: the git hooks cover the "daemon off" case and speed up post-commit synchronization even when the daemon is active, without duplicating work.

---

## Architecture

```
 git commit / checkout / merge
          │
          ▼
    hook script (sh/ps1)
          │
          ├─── daemon UP? ──YES──► speedy-ai-context daemon exec index <file> (IPC)
          │                         (the daemon already watches the FS, but the
          │                          hook forces the immediate index without
          │                          waiting for the 500 ms debounce)
          │
          └─── daemon DOWN? ────► speedy-ai-context index <file>  (direct process)
                                   (fallback without IPC, no daemon required)
```

The daemon remains the main path for uncommitted changes (continuous saves, live refactors). The hooks are the accelerator and the safety net.

---

## Files to create

### 1. Hook scripts — `scripts/git-hooks/`

**`post-commit`** (sh template — `{{SPEEDY_WORKER_EXE}}` and `{{SPEEDY_CLI_EXE}}` replaced by `install-hooks`)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
# SPEEDY_WORKER = speedy-ai-context (standalone fallback, no daemon needed)
# SPEEDY_CLI    = speedy-cli        (thin client, routes through daemon)
SPEEDY_WORKER="{{SPEEDY_WORKER_EXE}}"
SPEEDY_CLI="{{SPEEDY_CLI_EXE}}"

CHANGED=$(git diff-tree --no-commit-id -r --name-only HEAD 2>/dev/null)
[ -z "$CHANGED" ] && exit 0

ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY_CLI" daemon ping 2>/dev/null | grep -q "pong"; then
    # Daemon up: index via daemon (speedy-cli routes automatically)
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && "$SPEEDY_CLI" -p "$ROOT" index "$f"
    done
else
    # Daemon down: index directly with the worker
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && SPEEDY_NO_DAEMON=1 "$SPEEDY_WORKER" -p "$ROOT" index "$f"
    done
fi
exit 0
```

**`post-checkout`** (sh template)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY_WORKER="{{SPEEDY_WORKER_EXE}}"
SPEEDY_CLI="{{SPEEDY_CLI_EXE}}"
# $3 = 1 if branch switch, 0 if file checkout
[ "$3" = "0" ] && exit 0

ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY_CLI" daemon ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY_CLI" -p "$ROOT" sync
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY_WORKER" -p "$ROOT" sync
fi
exit 0
```

**`post-merge`** (sh template)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY_WORKER="{{SPEEDY_WORKER_EXE}}"
SPEEDY_CLI="{{SPEEDY_CLI_EXE}}"
ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY_CLI" daemon ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY_CLI" -p "$ROOT" sync
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY_WORKER" -p "$ROOT" sync
fi
exit 0
```

**`post-rewrite`** (sh template — covers rebase and amend)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY_WORKER="{{SPEEDY_WORKER_EXE}}"
SPEEDY_CLI="{{SPEEDY_CLI_EXE}}"
# $1 = "rebase" or "amend"
ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY_CLI" daemon ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY_CLI" -p "$ROOT" index .
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY_WORKER" -p "$ROOT" index .
fi
exit 0
```

**`post-commit.ps1`** (PowerShell template — native Windows alternative)
```powershell
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
$SPEEDY_WORKER = "{{SPEEDY_WORKER_EXE}}"   # speedy-ai-context (standalone fallback)
$SPEEDY_CLI    = "{{SPEEDY_CLI_EXE}}"      # speedy-cli (daemon-mediated)

$changed = git diff-tree --no-commit-id -r --name-only HEAD 2>$null
if (-not $changed) { exit 0 }

$root = git rev-parse --show-toplevel

$daemonUp = (& $SPEEDY_CLI daemon ping 2>$null) -eq "pong"

foreach ($f in $changed) {
    $full = Join-Path $root $f
    if (Test-Path $full) {
        if ($daemonUp) {
            & $SPEEDY_CLI -p $root index $f
        } else {
            $env:SPEEDY_NO_DAEMON = "1"
            & $SPEEDY_WORKER -p $root index $f
        }
    }
}
exit 0
```

---

### 2. New CLI command — `speedy-ai-context install-hooks` / `speedy-ai-context uninstall-hooks`

**`packages/speedy-ai-context/src/hooks.rs`** (new file)

Responsibilities:
- Resolves the `.git/hooks/` folder of the current repo (or via `git rev-parse --git-path hooks`)
- Gets the absolute path of its own executable via `std::env::current_exe()` and **interpolates it into the hook templates** — the scripts do not call bare `speedy-ai-context` but the exact path of the binary that ran `install-hooks`
- Makes the scripts executable (`chmod +x` on Unix, noop on Windows)
- `uninstall-hooks`: removes only the files that have the `# Speedy — managed hook` marker at the top
- Prints a report: which hooks installed, where, whether any pre-existing ones were found

**Why not `include_str!` verbatim**: the templates have two placeholders that are replaced at runtime:
- `{{SPEEDY_WORKER_EXE}}` → absolute path of `speedy-ai-context` (resolved by `current_exe()`)
- `{{SPEEDY_CLI_EXE}}` → absolute path of `speedy-cli` (looked up in the same directory as the worker)

This ensures the hooks work even if the binaries are not in `PATH`.

```rust
// hooks.rs — core logic
let worker = std::env::current_exe()?.canonicalize()?;
// Look for speedy-cli next to the worker (same installation folder)
let cli = worker.with_file_name(format!("speedy-cli{}", std::env::consts::EXE_SUFFIX));
let script = HOOK_POST_COMMIT_TEMPLATE
    .replace("{{SPEEDY_WORKER_EXE}}", &worker.to_string_lossy())
    .replace("{{SPEEDY_CLI_EXE}}", &cli.to_string_lossy());
std::fs::write(&hook_path, script)?;
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
}
```

**Modify `packages/speedy-ai-context/src/cli.rs`**:
```rust
InstallHooks {
    /// Repo path (default: CWD)
    #[arg(long)]
    path: Option<PathBuf>,
    /// Use a symlink instead of a copy (more convenient for development)
    #[arg(long)]
    symlink: bool,
},
UninstallHooks {
    #[arg(long)]
    path: Option<PathBuf>,
},
```

---

## Files to modify

### `packages/speedy-core/src/config.rs`

Add an optional field (default `true`):
```rust
pub hooks_enabled: bool,   // default: true
```

Used by `install-hooks` to decide whether to skip and by any warnings.

---

### `packages/speedy-daemon/src/main.rs`

**IPC command `ping`** — already exists (`ping` → `pong`). The hooks invoke it through `speedy-cli daemon ping`. **No changes needed** to the daemon.

**New IPC command: `notify-commit\t<path>\t<file1>\t<file2>...`** (optional, phase 2):
- More efficient than sending N separate `speedy-cli index <file>` requests
- Receives a list of files and queues them to the workspace indexer without going through a subprocess
- Handler in `dispatch_command()`, around line 887

For phase 1 it is enough to use `speedy-cli -p <ROOT> index <file>`, which already routes via the daemon (IPC `exec index <file>`).

---

## User installation flow

```
# 1. Register the workspace (already exists)
speedy-cli workspace add .

# 2. Install the hooks in the current repo
speedy-ai-context install-hooks

# Expected output:
# ✓ Installed post-commit    → .git/hooks/post-commit
# ✓ Installed post-checkout  → .git/hooks/post-checkout
# ✓ Installed post-merge     → .git/hooks/post-merge
# ✓ Installed post-rewrite   → .git/hooks/post-rewrite
# Tip: run `speedy-ai-context uninstall-hooks` to remove them.
```

---

## Embedding the templates in the binary

The templates are embedded into `speedy-ai-context` with `include_str!` at compile time. They have the `{{SPEEDY_EXE}}` placeholder that is replaced with the absolute path at runtime:

```rust
// packages/speedy-ai-context/src/hooks.rs
const HOOK_POST_COMMIT_TPL:   &str = include_str!("../../scripts/git-hooks/post-commit.tpl");
const HOOK_POST_CHECKOUT_TPL: &str = include_str!("../../scripts/git-hooks/post-checkout.tpl");
const HOOK_POST_MERGE_TPL:    &str = include_str!("../../scripts/git-hooks/post-merge.tpl");
const HOOK_POST_REWRITE_TPL:  &str = include_str!("../../scripts/git-hooks/post-rewrite.tpl");
// Windows
const HOOK_POST_COMMIT_PS1_TPL: &str = include_str!("../../scripts/git-hooks/post-commit.ps1.tpl");
```

Writing to disk (two separate placeholders for the two binaries):
```rust
let worker = std::env::current_exe()?.canonicalize()?;
let cli = worker.with_file_name(format!("speedy-cli{}", std::env::consts::EXE_SUFFIX));
// on Windows Git-Bash the path must be in POSIX format: /c/Users/...
let worker_str = normalize_for_sh(&worker);
let cli_str    = normalize_for_sh(&cli);
let script = TPL
    .replace("{{SPEEDY_WORKER_EXE}}", &worker_str)
    .replace("{{SPEEDY_CLI_EXE}}",    &cli_str);
```

On Windows you write both the `.sh` script (used by Git-Bash) and a `.bat` wrapper that invokes PowerShell for those who use CMD.

---

## Edge cases to handle

| Case | Behavior |
|---|---|
| Pre-existing (non-Speedy) hook | `install-hooks` prints a warning and asks for confirmation before overwriting |
| Global `core.hooksPath` | Respected: `git rev-parse --git-path hooks` returns the correct path |
| Repo without `.speedy/` | The hooks install anyway; on run they will execute `speedy-ai-context index`, which creates `.speedy/` |
| `--no-verify` | Bypasses the hooks: document as a known limitation |
| Submodules | The hooks must be installed per-submodule; `install-hooks --recursive` as a phase 2 flag |
| CI/CD (GitHub Actions, etc.) | The `SPEEDY_SKIP_HOOKS=1` env var causes an immediate exit 0 in all hooks |

---

## Development phases

### Phase 1 — MVP (high priority)
1. Create `scripts/git-hooks/post-commit`, `post-checkout`, `post-merge`, `post-rewrite`
2. Create `packages/speedy-ai-context/src/hooks.rs` with install/uninstall logic
3. Add `InstallHooks` / `UninstallHooks` to `packages/speedy-ai-context/src/cli.rs` and `main.rs`
4. Manual test on Windows (Git-Bash) and Linux

### Phase 2 — Optimizations
5. IPC command `notify-commit` for batches of files (avoids N subprocesses)
6. `--recursive` flag for submodules
7. `SPEEDY_SKIP_HOOKS` env var
8. Native PowerShell hook (`.ps1`) with a `.bat` wrapper on Windows

### Phase 3 — UX
9. `speedy-cli workspace add .` (or explicit `speedy-ai-context install-hooks`) installs the hooks automatically if `hooks_enabled = true`
10. `speedy-ai-context install-hooks --check` (or `status`) shows whether the hooks are installed for the current repo

---

## New dependencies

None. All the necessary code is already available on the daemon IPC side:
- `ping` → `pong` — already exists; the hooks invoke it via `speedy-cli daemon ping`
- IPC `exec <args>` — already exists (line 1022-1026 daemon/main.rs); `speedy-cli index` uses it automatically
- IPC `sync <path>` — already exists (line 979-986); `speedy-cli sync` uses it automatically
- IPC `reindex <path>` — already exists (line 988-995)
- `speedy-ai-context -p <ROOT> index <file>` (standalone) — already exists, used in the daemon-down fallback
