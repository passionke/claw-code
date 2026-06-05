#!/usr/bin/env bash
# Deprecated entrypoint — use ./deploy/stack/gateway.sh up or tap-up (wiring is automatic).
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

[[ -f "${REPO_ROOT}/.env" ]] || {
  echo "error: missing ${REPO_ROOT}/.env" >&2
  exit 1
}
set -a
# shellcheck disable=SC1090
source "${REPO_ROOT}/.env"
set +a

claw_ensure_worker_llm_wiring "${PODMAN_DIR}"
echo "OK: wrote ${PODMAN_DIR}/.claw-worker-runtime.env (no repo-root .env mutation). Run: ./deploy/stack/gateway.sh up"

if [[ "${1:-}" == "--restart" ]]; then
  "${PODMAN_DIR}/lib/tap-down.sh" || true
  "${PODMAN_DIR}/lib/down.sh" || true
  sleep 0.5
  "${PODMAN_DIR}/lib/tap-up.sh"
  "${PODMAN_DIR}/lib/up.sh"
fi
