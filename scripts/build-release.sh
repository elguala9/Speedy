#!/usr/bin/env bash
# Build all four Speedy release binaries and stage them under dist/.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dist="$root/dist"
target="$root/target/release"

rm -rf "$dist"
mkdir -p "$dist"

echo "==> Building release binaries..."
cargo build --release -p speedy -p speedy-daemon -p speedy-cli -p speedy-mcp -p speedy-gui

bins=(speedy speedy-daemon speedy-cli speedy-mcp speedy-gui)
ext=""
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) ext=".exe" ;;
esac

for b in "${bins[@]}"; do
    src="$target/${b}${ext}"
    if [[ -f "$src" ]]; then
        cp "$src" "$dist/"
        echo "  Copied ${b}${ext}"
    else
        echo "  MISSING: ${b}${ext}" >&2
    fi
done

echo
echo "Binaries staged in: $dist"
ls -lh "$dist"
