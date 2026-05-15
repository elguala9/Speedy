# Speedy Daemon IPC Protocol

The daemon listens on a local socket (Windows Named Pipe via `interprocess`,
Unix Domain Socket elsewhere). The name is configured with `--daemon-socket`
(default `speedy-daemon`).

## Wire format

- One request per connection.
- Request: a single line of UTF-8 text terminated by `\n`.
- Response: a single line of UTF-8 text terminated by `\n`.
- The connection is closed by the daemon after writing the response.

## Path encoding

`exec` accepts a tab-separated form so that paths with spaces survive
transport:

```
exec\t<cwd>\t<arg1>\t<arg2>...
```

`<cwd>` (which may be empty to skip the chdir) sets the working directory
that `speedy` will run in. The whitespace form `exec <args>` is still
accepted for legacy callers that do not need a cwd.

## Commands

| Command | Description | Response |
|---------|-------------|----------|
| `ping` | Health check. | `pong` |
| `status` | Daemon status, as JSON. | `{"pid":..., "uptime_secs":..., "workspace_count":..., "watcher_count":..., "version":..., "protocol_version":1}` |
| `list` | Monitored workspace paths, as JSON array. | `["/path/1", "/path/2"]` |
| `watch-count` | Number of active watchers. | `3` |
| `daemon-pid` | Daemon process ID. | `12345` |
| `stop` | Graceful shutdown. | `ok` |
| `reload` | Reload workspaces from disk, sync watchers. | `ok: N workspaces reloaded` |
| `add <path>` | Register a workspace and start a watcher. | `ok` or `error: ...` |
| `remove <path>` | Stop the watcher and unregister the workspace. | `ok` or `error: ...` |
| `is-workspace <path>` | Whether the canonical path is monitored. | `true` or `false` |
| `sync <path>` | Incrementally sync the workspace index (spawns `speedy.exe -p <path> sync`). | `ok` or `error: ...` |
| `metrics` | Cumulative counters since daemon start (queries, indexes, syncs, watcher_events, exec_calls). | JSON object |
| `query-all\t<top_k>\t<query>` | (v2) Fan-out query across every registered workspace; returns the merged top-K. | JSON array of `{workspace, path, line, text, score}` |
| `exec <args>` | Run `speedy <args>` and return its stdout. | command output |

## Examples

### Ping

```
> ping
< pong
```

### Status

```
> status
< {"pid":4582,"uptime_secs":1287,"workspace_count":2,"watcher_count":2,"version":"0.1.0"}
```

### Add a workspace with a space in the path

The CLI converts argv into the tab-separated form before sending:

```
> exec\tC:\Users\me\Projects\My App\twatch
< Watcher detached (PID: 9123).
```

### Add / remove

```
> add C:\code\myproj
< ok

> remove C:\code\myproj
< ok
```

### Unknown command

```
> nope
< error: unknown command: nope
```

## Notes

- The daemon listens with a 1-second timeout on `accept()` so it can poll
  `running` and exit cleanly within the same tick after `stop`.
- `exec` runs `speedy.exe` with `SPEEDY_NO_DAEMON=1` so the child never
  re-enters the daemon and we cannot fork-bomb the workspace.
- On boot, entries in `workspaces.json` whose path no longer exists are
  pruned automatically.
