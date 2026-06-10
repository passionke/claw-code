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

# node B host pool (ci-b RPC dirs); do not call pool-daemon-down — it re-sources repo .env (node A). kejiqing
for sub in strict relaxed; do
  pidf="${PODMAN_DIR}/.claw-pool-rpc-ci-b/${sub}/daemon.pid"
  [[ -f "${pidf}" ]] || continue
  pid="$(tr -dc '0-9' <"${pidf}" 2>/dev/null || true)"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
    echo "ci-cluster-cleanup: stop pool-daemon pid=${pid} (${sub})" >&2
    kill "${pid}" 2>/dev/null || true
  fi
done

echo "ci-cluster-cleanup: ok" >&2
