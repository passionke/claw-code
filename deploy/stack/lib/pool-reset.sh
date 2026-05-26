#!/usr/bin/env bash
# Stop host pool daemon, free RPC port, remove all claw-worker containers. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

# shellcheck disable=SC1091
source "${LIB_DIR}/nuclear-pool-reset.sh"

if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

"${PODMAN_DIR}/lib/pool-daemon-down.sh" 2>/dev/null || true
# Orphan daemons when pid file is stale (common after manual kills).
while read -r pid; do
  [[ -n "${pid}" ]] || continue
  kill "${pid}" 2>/dev/null || true
done < <(pgrep -f '[/]claw-pool-daemon' 2>/dev/null || true)
sleep 0.3
claw_nuclear_pool_reset "${PODMAN_DIR}"
echo "==> pool reset done (daemon stopped, workers removed)"
