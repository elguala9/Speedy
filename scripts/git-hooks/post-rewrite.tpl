#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy install-hooks)
# Routes through speedy-cli, which orchestrates every enabled context
# (ai-context, language-context, text-context). Indexing is opt-in: each
# context no-ops unless it is enabled here. Runs entirely daemon-free.
SPEEDY="{{SPEEDY_CLI_EXE}}"
[ -x "$SPEEDY" ] || SPEEDY=$(command -v speedy-cli 2>/dev/null)
[ -n "$SPEEDY" ] || exit 0
[ -n "$SPEEDY_SKIP_HOOKS" ] && exit 0

# $1 = "rebase" or "amend" — both rewrite history. An incremental sync re-indexes
# every changed file and prunes deleted ones across all enabled contexts; thanks
# to the per-file hash check it is far cheaper than a full clear-and-rebuild.
ROOT=$(git rev-parse --show-toplevel)

SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" sync

exit 0
