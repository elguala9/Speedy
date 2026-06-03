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
  │ probe is_alive() once
  │
  ├─ index/query/context/sync/reembed
  │     daemon → exec → speedy-ai-context
  │     standalone → spawn speedy-ai-context <cmd>
  │
  ├─ reindex            (fan-out over the 3)
  │     daemon → IPC reindex
  │     standalone → contexts::reindex_workspace
  │
  ├─ workspace add/remove
  │     daemon → IPC
  │     standalone → speedy_core::workspace
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

## reindex — fan-out over the 3 contexts

```
reindex_workspace(path)               [speedy_core::contexts]
  load_features(path)
  ├─ speedy_indexer?  → speedy-ai-context index --clear .   (timeout 30m)
  ├─ language_context? → speedy-language-context clear-index; index
  └─ text_context?    → speedy-text-context index <path>
  → summary: "ai-context: .. | slc: .. | text: .."
  (feature off = skip; each worker independent, one failure does not block the others)
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
