#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy install-hooks)
SPEEDY="{{SPEEDY_EXE}}"
[ -x "$SPEEDY" ] || SPEEDY=$(command -v speedy 2>/dev/null)
[ -n "$SPEEDY" ] || exit 0
[ -n "$SPEEDY_SKIP_HOOKS" ] && exit 0

ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY" ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY" daemon sync "$ROOT"
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" sync
fi

# speedy-language-context: full reindex after merge (optional)
SLC="{{SLC_EXE}}"
[ -x "$SLC" ] || SLC=$(command -v speedy-language-context 2>/dev/null)
[ -n "$SLC" ] && "$SLC" --path "$ROOT" index

exit 0
