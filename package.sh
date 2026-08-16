#!/usr/bin/env bash
#
# package.sh — build the frontend and zip the plugin into a uniquely,
# incrementally named archive under packages/, ready for Decky Loader's
# "Install Plugin from ZIP". Never overwrites a previous zip: each run
# produces packages/decky-hydra-launcher-1.zip, -2.zip, -3.zip, ...
#
# Note: this does NOT compile the Rust backend (backend/), it only packages
# whatever is already at bin/backend (if present). Features that shell out to
# the backend binary won't work in a zip built this way unless bin/backend
# was built separately (e.g. via the Decky CLI + Docker, or copied from an
# existing install).
#
#   ./package.sh
#
set -euo pipefail

log()  { printf '\e[1;35m[package]\e[0m %s\n' "$*"; }
die()  { printf '\e[1;31m[fail]\e[0m %s\n' "$*" >&2; exit 1; }

PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NAME="$(basename "$PLUGIN_DIR")"       # decky-hydra-launcher
REPO_ROOT="$(dirname "$PLUGIN_DIR")"
OUT_DIR="$PLUGIN_DIR/packages"

command -v pnpm >/dev/null || die "pnpm not found"
command -v zip >/dev/null || die "zip not found"

log "Building frontend..."
( cd "$PLUGIN_DIR" && pnpm run build )

mkdir -p "$OUT_DIR"
n=1
while [[ -e "$OUT_DIR/${NAME}-$n.zip" ]]; do
  n=$((n + 1))
done
OUT="$OUT_DIR/${NAME}-$n.zip"

log "Zipping -> packages/$(basename "$OUT")"
cd "$REPO_ROOT"
zip -r -q "$OUT" "$NAME" \
  -x "$NAME/node_modules/*" \
  -x "$NAME/src/*" \
  -x "$NAME/packages/*" \
  -x "$NAME/backend/*" \
  -x "$NAME/proto/*" \
  -x "$NAME/cli/*" \
  -x "$NAME/out/*" \
  -x "$NAME/.vscode/*" \
  -x "$NAME/.github/*" \
  -x "$NAME/.claude/*" \
  -x "$NAME/.git/*" \
  -x "$NAME/.gitignore" \
  -x "$NAME/.gitmodules" \
  -x "$NAME/decky.pyi" \
  -x "$NAME/tsconfig.json" \
  -x "$NAME/eslint.config.js" \
  -x "$NAME/rollup.config.js" \
  -x "$NAME/README.md" \
  -x "$NAME/LICENSE" \
  -x "$NAME/thumb.gif" \
  -x "$NAME/pnpm-lock.yaml" \
  -x "$NAME/package.sh" \
  -x '*/__pycache__/*' \
  -x '*.pyc' \
  -x '*.DS_Store'

log "Done: $OUT"
