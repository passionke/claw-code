#!/usr/bin/env bash
# Stop host claw-sandbox. Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-daemon-launchd.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"

REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
if [[ -f "${REPO_ROOT}/.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${REPO_ROOT}/.env"
  set +a
fi

claw_pool_down_one() {
  local rpc_dir="$1" http_port="$2"
  local AUDIT_LOG="${rpc_dir}/daemon-down.audit.log"
  mkdir -p "${rpc_dir}"
  {
    printf '\n%s pool-daemon-down begin ppid=%s port=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$PPID" "${http_port}"
  } >>"${AUDIT_LOG}" 2>/dev/null || true

  claw_pool_wait_http_down() {
    local i
    for i in $(seq 1 30); do
      if ! curl -fsS --connect-timeout 1 "http://127.0.0.1:${http_port}/healthz/live-report" >/dev/null 2>&1; then
        return 0
      fi
      sleep 0.1
    done
    return 1
  }

  if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
    claw_pool_launchd_bootout
  elif [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]]; then
    # shellcheck disable=SC1091
    source "${LIB_DIR}/pool-daemon-systemd.sh"
    if claw_pool_use_systemd 2>/dev/null && claw_pool_systemd_installed; then
      echo "==> stopping claw-sandbox (systemd)" >&2
      claw_pool_systemd_stop || true
    fi
  fi

  if [[ -f "${rpc_dir}/daemon.pid" ]]; then
    local pid
    pid="$(cat "${rpc_dir}/daemon.pid")"
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      echo "==> stopping claw-sandbox pid=${pid}" >&2
      kill "${pid}" 2>/dev/null || true
      if ! claw_pool_wait_http_down; then
        kill -9 "${pid}" 2>/dev/null || true
        claw_pool_wait_http_down 2>/dev/null || true
      fi
    fi
    rm -f "${rpc_dir}/daemon.pid"
  fi

  # shellcheck source=nuclear-pool-reset.sh
  source "${LIB_DIR}/nuclear-pool-reset.sh"
  claw_kill_tcp_listeners "${http_port}" 2>/dev/null || true
  rm -f "${rpc_dir}/pool.sock"
}

HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"
RPC_DIR="$(claw_pool_rpc_root "${PODMAN_DIR}")"
claw_pool_down_one "${RPC_DIR}" "${HTTP_PORT}"
claw_cleanup_legacy_dual_pool "${PODMAN_DIR}"

while read -r pid; do
  [[ -n "${pid}" ]] || continue
  kill "${pid}" 2>/dev/null || true
done < <(pgrep -f '[/](claw-sandbox|claw-pool-daemon)' 2>/dev/null || true)
