#!/usr/bin/env bash
# Deploy Warships to Fly.io.
#
# The Dockerfile needs BOTH `warships/` and its sibling
# `warships-kodiak/` in the build context. This script assembles them
# into a temporary directory, copies fly.toml + Dockerfile in, runs
# `fly deploy` from there, and cleans up on exit.
#
# Usage:
#   ./deploy.sh                         # normal deploy
#   ./deploy.sh -- --local-only         # anything after -- is passed
#                                         through to fly deploy
set -euo pipefail

# ─── resolve paths ───────────────────────────────────────────────────
WARSHIPS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KODIAK_DIR="$(cd "${WARSHIPS_DIR}/.." && pwd)/warships-kodiak"

if [[ ! -d "${KODIAK_DIR}" ]]; then
  echo "error: expected sibling directory at ${KODIAK_DIR}" >&2
  echo "Dockerfile needs both warships/ and warships-kodiak/ to build." >&2
  exit 1
fi

if ! command -v fly >/dev/null 2>&1; then
  echo "error: flyctl not installed. Run: brew install flyctl" >&2
  exit 1
fi

if ! command -v rsync >/dev/null 2>&1; then
  echo "error: rsync not installed" >&2
  exit 1
fi

# ─── stage a temp build context ──────────────────────────────────────
STAGE=$(mktemp -d -t warships-deploy-XXXXXXXX)
trap 'rm -rf "${STAGE}"' EXIT

echo "→ staging build context at ${STAGE}"

# rsync is ~10x faster than cp -r for the kodiak tree and lets us
# exclude big scratch dirs. -a preserves perms/timestamps, --delete
# ensures the target is a clean mirror.
rsync -a --delete \
  --exclude 'target/' \
  --exclude '.git/' \
  --exclude 'node_modules/' \
  --exclude '.DS_Store' \
  --exclude '.fly/' \
  "${WARSHIPS_DIR}/" "${STAGE}/warships/"

rsync -a --delete \
  --exclude 'target/' \
  --exclude '.git/' \
  --exclude 'node_modules/' \
  --exclude '.DS_Store' \
  "${KODIAK_DIR}/" "${STAGE}/warships-kodiak/"

# Dockerfile and fly.toml go at the context root where fly deploy
# expects them (they're already COPIED above as part of warships/,
# but fly reads fly.toml from the current dir).
cp "${WARSHIPS_DIR}/Dockerfile" "${STAGE}/Dockerfile"
cp "${WARSHIPS_DIR}/fly.toml"   "${STAGE}/fly.toml"
if [[ -f "${WARSHIPS_DIR}/.dockerignore" ]]; then
  cp "${WARSHIPS_DIR}/.dockerignore" "${STAGE}/.dockerignore"
fi

# ─── deploy ──────────────────────────────────────────────────────────
cd "${STAGE}"

# Pass through any extra args after `--` to fly deploy.
PASSTHROUGH=()
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "--" ]]; then
    shift
    PASSTHROUGH+=("$@")
    break
  fi
  PASSTHROUGH+=("$1")
  shift
done

echo "→ fly deploy ${PASSTHROUGH[*]:-}"
fly deploy "${PASSTHROUGH[@]:-}"
