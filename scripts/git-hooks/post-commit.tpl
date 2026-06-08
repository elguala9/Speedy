#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy install-hooks)
# Routes through speedy-cli, which orchestrates every enabled context
# (ai-context, language-context, text-context). Indexing is opt-in: each
# context no-ops unless it is enabled here. Runs entirely daemon-free.
SPEEDY="{{SPEEDY_CLI_EXE}}"
# Robustness: fall back to PATH if the hardcoded path is missing or moved
[ -x "$SPEEDY" ] || SPEEDY=$(command -v speedy-cli 2>/dev/null)
[ -n "$SPEEDY" ] || exit 0
[ -n "$SPEEDY_SKIP_HOOKS" ] && exit 0

CHANGED=$(git diff-tree --no-commit-id -r --name-only HEAD 2>/dev/null)
[ -z "$CHANGED" ] && exit 0
ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT" 2>/dev/null || exit 0

# Collect changed files that still exist, then update them in a single call —
# speedy-cli fans the per-file update out to every enabled context.
FILES=""
for f in $CHANGED; do
    [ -f "$f" ] && FILES="$FILES $f"
done
[ -n "$FILES" ] && SPEEDY_NO_DAEMON=1 "$SPEEDY" update $FILES

exit 0
