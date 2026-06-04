#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy install-hooks)
# Runs the worker standalone (no daemon). Indexing is opt-in: the worker
# no-ops unless the workspace has enabled the relevant feature.
SPEEDY="{{SPEEDY_EXE}}"
[ -x "$SPEEDY" ] || SPEEDY=$(command -v speedy 2>/dev/null)
[ -n "$SPEEDY" ] || exit 0
[ -n "$SPEEDY_SKIP_HOOKS" ] && exit 0

# $1 = "rebase" or "amend" — both warrant a full reindex
ROOT=$(git rev-parse --show-toplevel)

# speedy-ai-context: full re-index after rebase/amend
SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" index .

# speedy-language-context: full reindex after rebase/amend
SLC="{{SLC_EXE}}"
[ -x "$SLC" ] || SLC=$(command -v speedy-language-context 2>/dev/null)
[ -n "$SLC" ] && "$SLC" --path "$ROOT" index

exit 0
