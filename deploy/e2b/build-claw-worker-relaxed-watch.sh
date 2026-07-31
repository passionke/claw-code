#!/usr/bin/env bash
# Build claw-worker-relaxed with live log (no blind wait). Author: kejiqing
# Prefer: ./deploy/stack/gateway.sh e2b-worker-deploy (builds strict + relaxed).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

# Caller exports win over .env (bash source would otherwise clobber). Author: kejiqing
_SAVED_WORKER_IMAGE="${CLAW_E2B_WORKER_IMAGE:-}"
_SAVED_RUNTIME="${CLAW_CONTAINER_RUNTIME:-}"
_SAVED_COPY_DIR="${CLAW_E2B_TEMPLATE_COPY_DIR:-}"
[[ -f .env ]] && set -a && source .env && set +a
[[ -n "${_SAVED_WORKER_IMAGE}" ]] && export CLAW_E2B_WORKER_IMAGE="${_SAVED_WORKER_IMAGE}"
[[ -n "${_SAVED_RUNTIME}" ]] && export CLAW_CONTAINER_RUNTIME="${_SAVED_RUNTIME}"
[[ -n "${_SAVED_COPY_DIR}" ]] && export CLAW_E2B_TEMPLATE_COPY_DIR="${_SAVED_COPY_DIR}"

export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-podman}"
export CLAW_E2B_WORKER_RELAXED_ALIAS="${CLAW_E2B_WORKER_RELAXED_ALIAS:-claw-worker-relaxed}"
LOG="/tmp/claw-relaxed-build-$(date +%Y%m%d-%H%M%S).log"
echo "==> log: $LOG"
echo "==> tail -f $LOG   # 另开终端看进度"
"${ROOT}/.venv-fc/bin/python3" -u deploy/e2b/build-claw-worker-relaxed-selfhosted.py 2>&1 | tee "$LOG"
echo "==> done; log kept at $LOG"
