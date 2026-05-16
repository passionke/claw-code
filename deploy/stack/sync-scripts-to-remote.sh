#!/usr/bin/env bash
# Sync deploy/stack scripts + compose/Containerfiles to a remote host (no git on server). Author: kejiqing
# Does NOT copy claw-workspace, sessions, pool RPC state, or Rust sources.
#
# Usage:
#   ./deploy/stack/sync-scripts-to-remote.sh
#   ./deploy/stack/sync-scripts-to-remote.sh admin@192.168.9.252 /home/admin/claw-code
set -euo pipefail

STACK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REMOTE="${1:-admin@192.168.9.252}"
REMOTE_REPO="${2:-/home/admin/claw-code}"
DEST="${REMOTE}:${REMOTE_REPO}/deploy/stack/"

echo "rsync deploy/stack scripts -> ${DEST}" >&2
rsync -avz \
  --chmod=Du=rwx,Fu=rx,Dg=rx,Fg=rx \
  --exclude 'claw-workspace/' \
  --exclude 'claw-gateway-sessions/' \
  --exclude 'claw-logs/' \
  --exclude 'claude-tap-data/' \
  --exclude 'claude-tap.log' \
  --exclude 'claude-tap.pid' \
  --exclude '.claw-pool-rpc/' \
  --exclude '.build.log' \
  --exclude 'worker-openai.env' \
  --exclude '.claw-pool-workspace.env' \
  --exclude '.claw-image-release.env' \
  --exclude '__pycache__/' \
  --exclude 'deploy/' \
  "${STACK_DIR}/" "${DEST}"

echo "done. on remote: cd ${REMOTE_REPO} && ./deploy/stack/gateway.sh up --release <tag>" >&2
