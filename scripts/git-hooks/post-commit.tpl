#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy install-hooks)
SPEEDY="{{SPEEDY_EXE}}"
# Robustness: fall back to PATH if the hardcoded path is missing or moved
[ -x "$SPEEDY" ] || SPEEDY=$(command -v speedy 2>/dev/null)
[ -n "$SPEEDY" ] || exit 0
[ -n "$SPEEDY_SKIP_HOOKS" ] && exit 0

CHANGED=$(git diff-tree --no-commit-id -r --name-only HEAD 2>/dev/null)
[ -z "$CHANGED" ] && exit 0
ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY" ping 2>/dev/null | grep -q "pong"; then
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && "$SPEEDY" daemon exec -- index "$f"
    done
else
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" index "$f"
    done
fi

# speedy-language-context: incremental symbol-graph update (optional)
SLC="{{SLC_EXE}}"
[ -x "$SLC" ] || SLC=$(command -v speedy-language-context 2>/dev/null)
if [ -n "$SLC" ]; then
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && "$SLC" --path "$ROOT" update "$ROOT/$f"
    done
fi

exit 0
