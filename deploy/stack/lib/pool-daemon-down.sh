#!/usr/bin/env bash
# Stop host claw-pool-daemon (only when caller explicitly restarts). Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"
AUDIT_LOG="${RPC_DIR}/daemon-down.audit.log"

# Who stopped the pool? Append-only; used when daemon.log lacks "shutting down". kejiqing
{
  printf '\n%s pool-daemon-down begin ppid=%s cmd=' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$PPID"
  ps -p "${PPID}" -o command= 2>/dev/null || printf '?'
  printf '\n  caller_stack:'
  local_i=0
  while [[ -n "${BASH_SOURCE[${local_i}]+x}" ]]; do
    printf ' %s:%s(%s)' "${BASH_SOURCE[${local_i}]}" "${BASH_LINENO[${local_i}]}" "${FUNCNAME[${local_i}]:-main}"
    local_i=$((local_i + 1))
  done
  printf '\n'
} >>"${AUDIT_LOG}" 2>/dev/null || true

claw_pool_wait_http_down() {
  local i
  for i in $(seq 1 30); do
    if ! curl -fsS --connect-timeout 1 "http://127.0.0.1:${HTTP_PORT}/healthz/live-report" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-launchd.sh"
  claw_pool_launchd_bootout
elif [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]]; then
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-systemd.sh"
  if claw_pool_use_systemd 2>/dev/null && claw_pool_systemd_installed; then
    echo "==> stopping claw-pool-daemon (systemd)" >&2
    claw_pool_systemd_stop || true
  elif claw_pool_systemd_installed 2>/dev/null; then
    echo "==> pool systemd unit present but no passwordless sudo; stop via docker chroot" >&2
    claw_pool_systemd_stop_via_docker || true
    claw_pool_wait_http_down 2>/dev/null && exit 0
  fi
fi

if [[ -f "${RPC_DIR}/daemon.pid" ]]; then
  pid="$(cat "${RPC_DIR}/daemon.pid")"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
    echo "==> stopping claw-pool-daemon pid=${pid}" >&2
    kill "${pid}" 2>/dev/null || true
    if ! claw_pool_wait_http_down; then
      kill -9 "${pid}" 2>/dev/null || true
      claw_pool_wait_http_down 2>/dev/null || true
    fi
  fi
  rm -f "${RPC_DIR}/daemon.pid"
fi

while read -r pid; do
  [[ -n "${pid}" ]] || continue
  kill "${pid}" 2>/dev/null || true
done < <(pgrep -f '[/]claw-pool-daemon' 2>/dev/null || true)
sleep 0.3

# shellcheck source=nuclear-pool-reset.sh
source "${LIB_DIR}/nuclear-pool-reset.sh"
claw_kill_tcp_listeners "${CLAW_POOL_DAEMON_PORT:-9943}" 2>/dev/null || true
claw_kill_tcp_listeners "${HTTP_PORT}" 2>/dev/null || true
rm -f "${RPC_DIR}/pool.sock"

if claw_pool_wait_http_down 2>/dev/null; then
  exit 0
fi
echo "error: claw-pool-daemon still listening on 127.0.0.1:${HTTP_PORT} after stop" >&2
echo "hint: CLAW_POOL_DAEMON_USE_SYSTEMD=0 on CI; or NOPASSWD systemctl; check deploy/stack/.claw-pool-rpc/daemon-down.audit.log" >&2
exit 1
