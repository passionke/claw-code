#!/usr/bin/env bash
# Stop host claw-pool-daemon(s). --profile=strict|relaxed|all (default all). Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-daemon-launchd.sh"

CLAW_POOL_PROFILE=all
for arg in "$@"; do
  case "${arg}" in
    --profile=strict) CLAW_POOL_PROFILE=strict ;;
    --profile=relaxed) CLAW_POOL_PROFILE=relaxed ;;
    --profile=all) CLAW_POOL_PROFILE=all ;;
  esac
done

claw_pool_down_one() {
  local rpc_dir="$1" http_port="$2" profile="${3:-}"
  local AUDIT_LOG="${rpc_dir}/daemon-down.audit.log"
  mkdir -p "${rpc_dir}"
  {
    printf '\n%s pool-daemon-down begin profile=%s ppid=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${profile:-legacy}" "$PPID"
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
    claw_pool_launchd_bootout "${profile}"
  elif [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]]; then
    # shellcheck disable=SC1091
    source "${LIB_DIR}/pool-daemon-systemd.sh"
    if claw_pool_use_systemd 2>/dev/null && claw_pool_systemd_installed "${profile}"; then
      echo "==> stopping claw-pool-daemon (systemd) profile=${profile:-legacy}" >&2
      claw_pool_systemd_stop "${profile}" || true
    fi
  fi

  if [[ -f "${rpc_dir}/daemon.pid" ]]; then
    local pid
    pid="$(cat "${rpc_dir}/daemon.pid")"
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      echo "==> stopping claw-pool-daemon pid=${pid} profile=${profile:-legacy}" >&2
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

# Legacy single-pool layout (pre dual-pool).
if [[ "${CLAW_POOL_PROFILE}" == "all" ]]; then
  claw_pool_down_one "${PODMAN_DIR}/.claw-pool-rpc" "${CLAW_POOL_HTTP_PORT:-9944}" ""
fi
if [[ "${CLAW_POOL_PROFILE}" == "all" || "${CLAW_POOL_PROFILE}" == "strict" ]]; then
  claw_pool_down_one "${PODMAN_DIR}/.claw-pool-rpc/strict" "${CLAW_STRICT_POOL_HTTP_PORT:-9944}" "strict"
fi
if [[ "${CLAW_POOL_PROFILE}" == "all" || "${CLAW_POOL_PROFILE}" == "relaxed" ]]; then
  claw_pool_down_one "${PODMAN_DIR}/.claw-pool-rpc/relaxed" "${CLAW_RELAXED_POOL_HTTP_PORT:-9954}" "relaxed"
fi

if [[ "${CLAW_POOL_PROFILE}" == "all" ]]; then
  while read -r pid; do
    [[ -n "${pid}" ]] || continue
    kill "${pid}" 2>/dev/null || true
  done < <(pgrep -f '[/]claw-pool-daemon' 2>/dev/null || true)
  sleep 0.3
fi
