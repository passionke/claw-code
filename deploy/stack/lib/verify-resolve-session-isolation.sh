#!/usr/bin/env bash
# Resolve: two sessionId values must use distinct gateway-solve jsonl paths. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

CLUSTER_ID="${CLAW_CLUSTER_ID:-dev-stable}"
WORK_ROOT="${CLAW_WORK_ROOT:-/var/lib/claw/workspace}"
PROJ_ID="${CLAW_RESOLVE_E2E_PROJ_ID:-1}"
SEG_A="resolve-${PROJ_ID}-panel-a"
SEG_B="resolve-${PROJ_ID}-panel-b"

path_for_seg() {
  echo "${WORK_ROOT}/${CLUSTER_ID}/proj_${PROJ_ID}/sessions/${1}/.claw/gateway-solve-session.jsonl"
}

PA="$(path_for_seg "${SEG_A}")"
PB="$(path_for_seg "${SEG_B}")"

if [[ "${PA}" == "${PB}" ]]; then
  echo "FAIL: resolve session jsonl paths must differ per sessionId" >&2
  exit 1
fi

echo "OK resolve isolation paths (cluster=${CLUSTER_ID}):"
echo "  A=${PA}"
echo "  B=${PB}"
