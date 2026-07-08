#!/usr/bin/env bash
# 252 e2b one-click deploy → gateway.sh 252-up (gateway + Admin + e2b singletons).
# Author: kejiqing
#
# Usage (on 252, repo at e.g. /home/admin/claw-code):
#   cp deploy/server-252/env.pre-252.e2b.example .env   # fill PG / e2b / LLM secrets
#   ./deploy/server-252/deploy-one-click.sh --release release-v1.6.18
#
# Legacy host-pool deploy-one-click (claude-tap sidecar + claw-pool-daemon) removed.
# Use ./deploy/stack/gateway.sh 252-up directly if you prefer.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

[[ -f "${REPO_ROOT}/.env" ]] || {
  echo "error: missing ${REPO_ROOT}/.env" >&2
  echo "hint: cp deploy/server-252/env.pre-252.e2b.example .env" >&2
  exit 1
}

exec "${REPO_ROOT}/deploy/stack/gateway.sh" 252-up "$@"
