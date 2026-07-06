#!/usr/bin/env bash
# Build claw-worker-relaxed with live log (no blind wait). Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
[[ -f .env ]] && set -a && source .env && set +a
export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-docker}"
export CLAW_E2B_WORKER_RELAXED_ALIAS="${CLAW_E2B_WORKER_RELAXED_ALIAS:-claw-worker-relaxed}"
LOG="/tmp/claw-relaxed-build-$(date +%Y%m%d-%H%M%S).log"
echo "==> log: $LOG"
echo "==> tail -f $LOG   # 另开终端看进度"
"${ROOT}/.venv-fc/bin/python3" -u deploy/e2b/build-claw-worker-relaxed-selfhosted.py 2>&1 | tee "$LOG"
echo "==> done; log kept at $LOG"
