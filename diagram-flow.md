# Speedy — current flow

## Components

```
speedy-core (lib)          shared orchestration: contexts (Features,
                           exe resolver, reindex/sync), DaemonClient, workspace
speedy-cli                 front-end: daemon-if-alive, otherwise direct workers
speedy-gui                 actions via speedy-cli; monitoring via DaemonClient
speedy-daemon              optional, never auto-started; watcher + IPC
speedy-ai-context          worker: semantic index (feature speedy_indexer)
speedy-language-context    worker: code graph     (feature language_context)
speedy-text-context        worker: text index     (feature text_context)
```

## Routing: daemon-if-alive, otherwise standalone

```
                 is_alive()?
                     │
        ┌────── yes ─┴── no ──────┐
        ▼                         ▼
   DAEMON                    STANDALONE
   IPC → daemon              speedy_core::contexts
   daemon spawns workers     spawns workers in-process
        │                         │
        └──────────┬──────────────┘
                   ▼
         the 3 workers (gated by the features in .speedy/config.toml)
         speedy_indexer → ai-context
         language_context → language-context
         text_context → text-context
```

## CLI

```
speedy-cli <cmd>
  │ probe is_alive() once   (SPEEDY_NO_DAEMON=1 forces standalone)
  │
  ├─ index/query/context/reembed   (ai-context only)
  │     daemon → exec → speedy-ai-context
  │     standalone → spawn speedy-ai-context <cmd>
  │
  ├─ sync / force        (fan-out over the 3, INCREMENTAL)
  │     daemon → IPC sync
  │     standalone → contexts::sync_workspace
  │
  ├─ update <files...>   (fan-out over the 3, PER-FILE — used by post-commit)
  │     always in-process → contexts::update_files
  │
  ├─ reindex            (fan-out over the 3, FULL rebuild)
  │     daemon → IPC reindex
  │     standalone → contexts::reindex_workspace
  │
  ├─ workspace add/remove
  │     daemon → IPC ;  standalone → speedy_core::workspace
  │     + best-effort install/uninstall of the Speedy git hooks (if a git repo)
  │
  ├─ workspace list → speedy_core::workspace::list()  (always in-process)
  │
  └─ daemon status/ping/stop/list → requires a live daemon (errors if down)
```

## GUI

```
speedy-gui
  │
  ├─ ACTIONS (add/remove/sync/reindex/prune)
  │     spawn: speedy-cli --daemon-socket <s> <cmd>
  │     (the cli is the one choosing daemon or standalone)
  │
  └─ MONITORING (only with a live daemon)
        DaemonClient → status / metrics / subscribe-log

  state: alive ──► metrics + live log
         !alive ─► standalone = true
                   • workspace list via `speedy-cli workspace list`
                   • Dashboard: "Standalone mode" badge, no metrics
                   • Scan disabled (daemon-only)
                   • actions still operational via speedy-cli
```

## reindex — full rebuild, fan-out over the 3 contexts

```
reindex_workspace(path)               [speedy_core::contexts]
  load_features(path)
  ├─ speedy_indexer?  → speedy-ai-context index --clear .   (timeout 30m)
  ├─ language_context? → speedy-language-context clear-index; index
  └─ text_context?    → speedy-text-context index <path>
  → summary: "ai-context: .. | slc: .. | text: .."
  (feature off = skip; each worker independent, one failure does not block the others)
```

## sync — incremental, fan-out over the 3 contexts

```
sync_workspace(path, force)           [speedy_core::contexts]
  load_features(path)
  ├─ speedy_indexer?  → speedy-ai-context sync          (mtime+hash skip; prunes deletions)
  ├─ language_context? → speedy-language-context sync   (mtime+hash skip; prunes deletions)
  └─ text_context?    → speedy-text-context sync <path> (mtime+hash skip; prunes deletions)
  (NEVER slower than a full index: unchanged files are skipped by content hash)
```

## update — incremental, PER-FILE, fan-out over the 3 contexts

```
update_files(path, files, force)      [speedy_core::contexts]
  load_features(path)            for each given file (no whole-tree walk):
  ├─ speedy_indexer?  → speedy-ai-context -p <path> index <file>
  ├─ language_context? → speedy-language-context --path <path> update <file>
  └─ text_context?    → speedy-text-context update <path> <file>
  (the cheap path for the post-commit hook, which already knows the changed files)
```

## git hooks (default sync mechanism — no daemon)

```
installed by `speedy-cli workspace add` (best-effort) or `speedy install-hooks`
removed   by `speedy-cli workspace remove`            or `speedy uninstall-hooks`
all 4 hooks call speedy-cli with SPEEDY_NO_DAEMON=1 (→ standalone fan-out):

  post-commit   → speedy-cli update <changed files>   (per-file, all contexts)
  post-checkout → speedy-cli sync                      (branch switch only; $3≠0)
  post-merge    → speedy-cli sync
  post-rewrite  → speedy-cli sync                      (rebase / amend)

  (each context still no-ops unless enabled; SPEEDY_SKIP_HOOKS disables all)
```

## Watcher (daemon only)

```
file saved → watcher → debounce → ignore/hash check
  → per active feature: speedy-ai-context index <file>
                        speedy-language-context update <file>
                        speedy-text-context update <file>
  (all with SPEEDY_NO_DAEMON=1)
```

## Files on disk

```
~/.config/speedy/ (Win: %APPDATA%\speedy)
  workspaces.json   workspace registry (written by daemon or, standalone, by the cli)
  daemon.pid        current daemon pid

<workspace>/.speedy/
  config.toml       [features] speedy_indexer / language_context / text_context
  index.sqlite      ai-context vectors
  graph.db          language-context graph
  (text-context index)
```
