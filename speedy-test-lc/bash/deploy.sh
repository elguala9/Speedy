#!/usr/bin/env bash
# deploy.sh — build, test, and deploy a Rust service to a remote host.
set -euo pipefail

APP="speedy-service"
REMOTE="${DEPLOY_HOST:?DEPLOY_HOST not set}"
REMOTE_DIR="/opt/${APP}"
TARGET="x86_64-unknown-linux-musl"

log()  { echo "[$(date '+%H:%M:%S')] $*"; }
die()  { log "ERROR: $*" >&2; exit 1; }
check_dep() { command -v "$1" &>/dev/null || die "missing dependency: $1"; }

# ── Preflight ────────────────────────────────────────────────────────────────
check_dep cargo
check_dep ssh
check_dep rsync

# ── Build ────────────────────────────────────────────────────────────────────
log "Building release binary for ${TARGET}…"
cargo build --release --target "${TARGET}" 2>&1 | tail -5

BINARY="target/${TARGET}/release/${APP}"
[[ -f "${BINARY}" ]] || die "binary not found at ${BINARY}"
log "Binary size: $(du -sh "${BINARY}" | cut -f1)"

# ── Tests ────────────────────────────────────────────────────────────────────
log "Running tests…"
cargo test --release 2>&1 | tail -10

# ── Upload ───────────────────────────────────────────────────────────────────
log "Uploading to ${REMOTE}:${REMOTE_DIR}…"
ssh "${REMOTE}" "mkdir -p ${REMOTE_DIR}"
rsync -az --progress "${BINARY}" "${REMOTE}:${REMOTE_DIR}/${APP}.new"

# ── Atomic swap ──────────────────────────────────────────────────────────────
log "Swapping binary and restarting service…"
ssh "${REMOTE}" bash <<EOF
  set -euo pipefail
  mv "${REMOTE_DIR}/${APP}.new" "${REMOTE_DIR}/${APP}"
  chmod +x "${REMOTE_DIR}/${APP}"
  systemctl restart "${APP}" || true
  systemctl is-active --quiet "${APP}" && echo "Service is up" || echo "Service failed to start"
EOF

log "Deployment complete."
