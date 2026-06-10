#!/usr/bin/env bash
# Tear down CI node B (claw-cib) from a prior pipeline before node A up. Shared PG stays. Author: kejiqing
set -uo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"

# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"

rt="$(claw_container_runtime_cli 2>/dev/null || true)"
if [[ -z "${rt}" ]]; then
  exit 0
fi

stop_ctn() {
  local name="$1"
  if _claw_runtime_container_exists "${rt}" "${name}"; then
    echo "ci-cluster-cleanup: stop ${name}" >&2
    "${rt}" stop "${name}" >/dev/null 2>&1 || true
    "${rt}" rm -f "${name}" >/dev/null 2>&1 || true
  fi
}

stop_ctn "claw-gateway-rs-ci-b"
stop_ctn "claw-gateway-playground-ci-b"

# node B host pool (ci-b RPC); do not call pool-daemon-down — it would use wrong env before fix. kejiqing
# shellcheck disable=SC1091
source "${LIB_DIR}/nuclear-pool-reset.sh"
for rpc_dir in \
  "${PODMAN_DIR}/.claw-pool-rpc-ci-b" \
  "${PODMAN_DIR}/.claw-pool-rpc-ci-b/strict" \
  "${PODMAN_DIR}/.claw-pool-rpc-ci-b/relaxed"; do
  pidf="${rpc_dir}/daemon.pid"
  [[ -f "${pidf}" ]] || continue
  pid="$(tr -dc '0-9' <"${pidf}" 2>/dev/null || true)"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
    echo "ci-cluster-cleanup: stop pool-daemon pid=${pid} (${rpc_dir##*/})" >&2
    kill "${pid}" 2>/dev/null || true
  fi
done
claw_kill_tcp_listeners "${CLAW_CI_NODE_B_STRICT_PORT:-9964}" 2>/dev/null || true

echo "ci-cluster-cleanup: ok" >&2
