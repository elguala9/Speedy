#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy install-hooks)
# Routes through speedy-cli, which orchestrates every enabled context
# (ai-context, language-context, text-context). Indexing is opt-in: each
# context no-ops unless it is enabled here. Runs entirely daemon-free.
SPEEDY="{{SPEEDY_CLI_EXE}}"
[ -x "$SPEEDY" ] || SPEEDY=$(command -v speedy-cli 2>/dev/null)
[ -n "$SPEEDY" ] || exit 0
[ -n "$SPEEDY_SKIP_HOOKS" ] && exit 0

# $3 = 1 for branch switch, 0 for file checkout — only re-sync on branch switch
[ "$3" = "0" ] && exit 0
ROOT=$(git rev-parse --show-toplevel)

# Incremental sync across all enabled contexts (hash-skips unchanged files,
# prunes files removed by the branch switch).
SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" sync

exit 0
