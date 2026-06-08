#!/usr/bin/env bash
# Install the Speedy plugin MCP binaries (speedy-language-context-mcp and
# speedy-text-context-mcp) into a directory on PATH. They ship inside the
# per-target Speedy release tarball, so this downloads that tarball and extracts
# just the two MCP binaries.
#
# Usage:
#   ./install.sh                 # latest release, installs to ~/.local/bin
#   TAG=v0.2.2 ./install.sh
#   BIN_DIR=/usr/local/bin ./install.sh
set -euo pipefail

REPO="elguala9/Speedy"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
TAG="${TAG:-}"

# --- Detect target triple ----------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  case "$arch" in x86_64) target=x86_64-unknown-linux-gnu ;; *) echo "Unsupported Linux arch: $arch" >&2; exit 1 ;; esac ;;
  Darwin)
    # Only x86_64-apple-darwin is published; on Apple Silicon it runs via Rosetta.
    target=x86_64-apple-darwin
    [ "$arch" = "arm64" ] && echo "Note: using x86_64 build on Apple Silicon (runs under Rosetta)." >&2
    ;;
  *) echo "Unsupported OS: $os (use install.ps1 on Windows)" >&2; exit 1 ;;
esac

asset="speedy-${target}.tar.gz"
if [ -z "$TAG" ]; then
  url="https://github.com/$REPO/releases/latest/download/$asset"
else
  url="https://github.com/$REPO/releases/download/$TAG/$asset"
fi

# --- Download + extract just the two MCP binaries ----------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "Downloading $asset ..."
curl -fSL "$url" -o "$tmp/$asset"
tar xzf "$tmp/$asset" -C "$tmp" speedy-language-context-mcp speedy-text-context-mcp

mkdir -p "$BIN_DIR"
for bin in speedy-language-context-mcp speedy-text-context-mcp; do
  install -m 0755 "$tmp/$bin" "$BIN_DIR/$bin"
done

echo
echo "Installed to $BIN_DIR:"
echo "  speedy-language-context-mcp"
echo "  speedy-text-context-mcp"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo; echo "NOTE: $BIN_DIR is not on your PATH. Add it, e.g.:"; echo "  export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
