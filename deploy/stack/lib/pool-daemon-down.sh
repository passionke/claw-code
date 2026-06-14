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
# shellcheck disable=SC1091
source "${LIB_DIR}/log-ts.sh"

REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
_pool_env_file="${CLAW_POOL_UP_ENV_FILE:-${REPO_ROOT}/.env}"
if [[ -f "${_pool_env_file}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${_pool_env_file}"
  set +a
fi

claw_pool_http_up() {
  local http_port="$1"
  curl -fsS --connect-timeout 1 --max-time 2 \
    "http://127.0.0.1:${http_port}/healthz/live-report" >/dev/null 2>&1
}

claw_pool_signal_pid() {
  local pid="$1" sig="$2"
  kill "-${sig}" "${pid}" 2>/dev/null \
    || sudo -n kill "-${sig}" "${pid}" 2>/dev/null \
    || true
}

claw_pool_listener_pid() {
  local http_port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"${http_port}" -sTCP:LISTEN -t 2>/dev/null | head -1
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnp "sport = :${http_port}" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -1
  fi
}

claw_pool_try_stop_systemd() {
  local http_port="$1"
  [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]] || return 0
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-systemd.sh"
  if ! claw_pool_systemd_installed; then
    return 0
  fi
  if ! claw_pool_systemd_active && ! claw_pool_http_up "${http_port}"; then
    return 0
  fi
  claw_log "stopping claw-sandbox systemd unit (:${http_port} active; runs even when CLAW_POOL_DAEMON_USE_SYSTEMD=0)"
  claw_pool_systemd_stop || true
  if claw_pool_http_up "${http_port}"; then
    claw_log "systemctl stop incomplete; trying privileged systemctl via docker"
    claw_pool_systemd_stop_via_docker || true
  fi
}

claw_pool_down_one() {
  local rpc_dir="$1" http_port="$2"
  local AUDIT_LOG="${rpc_dir}/daemon-down.audit.log"
  local t0=$SECONDS wait_max="${CLAW_POOL_DOWN_WAIT_SEC:-8}"
  local listener_pid file_pid target_pid
  claw_log "pool-daemon-down: rpc_dir=${rpc_dir} http_port=${http_port}"
  mkdir -p "${rpc_dir}"
  {
    printf '\n%s pool-daemon-down begin ppid=%s port=%s\n' "$(TZ=Asia/Shanghai date '+%Y-%m-%d %H:%M:%S %Z')" "$PPID" "${http_port}"
  } >>"${AUDIT_LOG}" 2>/dev/null || true

  claw_pool_wait_http_down() {
    local deadline=$((SECONDS + wait_max)) i=0
    while ((SECONDS < deadline)); do
      i=$((i + 1))
      if ! claw_pool_http_up "${http_port}"; then
        claw_log "pool-daemon-down: :${http_port} down after ${i} probe(s), $((SECONDS - t0))s"
        return 0
      fi
      if ((i == 1 || i % 5 == 0)); then
        claw_log "pool-daemon-down: waiting :${http_port} down probe=${i} elapsed=$((SECONDS - t0))s"
      fi
      sleep 0.2
    done
    claw_log "pool-daemon-down: :${http_port} still up after ${wait_max}s wait"
    return 1
  }

  if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
    claw_pool_launchd_bootout
  fi

  if [[ -f "${LIB_DIR}/pool-daemon-docker.sh" ]]; then
    # shellcheck disable=SC1091
    source "${LIB_DIR}/pool-daemon-docker.sh"
    if claw_pool_use_docker_supervisor 2>/dev/null || claw_pool_docker_running 2>/dev/null; then
      claw_log "stopping claw-sandbox docker container (:${http_port})"
      claw_pool_docker_stop || true
      claw_pool_wait_http_down || true
      rm -f "${rpc_dir}/daemon.pid"
      claw_log "pool-daemon-down done in $((SECONDS - t0))s (docker)"
      return 0
    fi
  fi

  claw_pool_try_stop_systemd "${http_port}"

  listener_pid="$(claw_pool_listener_pid "${http_port}" || true)"
  file_pid=""
  if [[ -f "${rpc_dir}/daemon.pid" ]]; then
    file_pid="$(tr -dc '0-9' <"${rpc_dir}/daemon.pid" 2>/dev/null || true)"
  fi
  if [[ -n "${listener_pid}" || -n "${file_pid}" ]]; then
    claw_log "pool listener pid=${listener_pid:-none} daemon.pid=${file_pid:-none}"
  fi
  target_pid="${listener_pid:-${file_pid}}"

  if [[ -n "${target_pid}" ]] && kill -0 "${target_pid}" 2>/dev/null; then
    claw_log "stopping claw-sandbox pid=${target_pid} (SIGTERM)"
    claw_pool_signal_pid "${target_pid}" TERM
    if ! claw_pool_wait_http_down; then
      claw_log "claw-sandbox pid=${target_pid} still on :${http_port}; SIGKILL"
      claw_pool_signal_pid "${target_pid}" 9
      claw_pool_wait_http_down || true
    fi
  elif [[ -n "${file_pid}" ]]; then
    claw_log "pool-daemon-down: stale or missing pid in ${rpc_dir}/daemon.pid (pid=${file_pid})"
  else
    claw_log "pool-daemon-down: no listener on :${http_port} and no ${rpc_dir}/daemon.pid"
  fi
  rm -f "${rpc_dir}/daemon.pid"

  if [[ "${CLAW_POOL_DOWN_TCP_KILL:-1}" == "1" ]]; then
    # shellcheck source=nuclear-pool-reset.sh
    source "${LIB_DIR}/nuclear-pool-reset.sh"
    claw_kill_tcp_listeners "${http_port}" "pool-daemon-down"
  else
    claw_log "pool-daemon-down: skip TCP kill (CLAW_POOL_DOWN_TCP_KILL=0)"
  fi
  rm -f "${rpc_dir}/pool.sock"
  claw_log "pool-daemon-down done in $((SECONDS - t0))s"
}

HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"
RPC_DIR="$(claw_pool_rpc_root "${PODMAN_DIR}")"
claw_pool_down_one "${RPC_DIR}" "${HTTP_PORT}"
if [[ "${CLAW_POOL_DOWN_LEGACY_CLEANUP:-1}" == "1" ]] && [[ -z "${CLAW_POOL_RPC_INSTANCE:-}" ]]; then
  claw_cleanup_legacy_dual_pool "${PODMAN_DIR}"
fi
