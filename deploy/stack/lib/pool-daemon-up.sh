#!/usr/bin/env bash
# Host claw-sandbox: ensure HTTP up (skip if healthy). --restart = down then up. Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-daemon-binary.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"

RESTART=0
WITH_WORKERS=0
for arg in "$@"; do
  case "${arg}" in
    --restart) RESTART=1 ;;
    --ensure) RESTART=0 ;;
    --with-workers) WITH_WORKERS=1 ;;
    --profile=* | --profile | strict | relaxed | all)
      echo "error: --profile removed (single claw-sandbox on CLAW_POOL_HTTP_PORT)" >&2
      exit 1
      ;;
  esac
done

_pool_bin_from_up="${CLAW_POOL_DAEMON_BIN:-}"
_pool_env_file="${CLAW_POOL_UP_ENV_FILE:-${REPO_ROOT}/.env}"
if [[ -f "${_pool_env_file}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${_pool_env_file}"
  set +a
fi

claw_cleanup_legacy_dual_pool "${PODMAN_DIR}"

if [[ -n "${_pool_bin_from_up}" ]]; then
  export CLAW_POOL_DAEMON_BIN="${_pool_bin_from_up}"
fi

claw_apply_deploy_profile 2>/dev/null || true
# shellcheck source=release-images.sh
source "${LIB_DIR}/release-images.sh"
claw_reapply_pool_image_pins "${PODMAN_DIR}"

if [[ -z "${CLAW_POOL_WORK_ROOT_BIND_SRC:-}" ]]; then
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
fi

WORK_ROOT="${CLAW_POOL_WORK_ROOT_BIND_SRC:?missing CLAW_POOL_WORK_ROOT_BIND_SRC; run gateway.sh up first}"
# shellcheck source=claw-pool-registry-env.sh
source "${LIB_DIR}/claw-pool-registry-env.sh"
_base_pool_id="$(claw_default_pool_id)"
RPC_DIR="$(claw_pool_rpc_root "${PODMAN_DIR}")"
HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"
_pool_daemon_tcp_port="${CLAW_POOL_DAEMON_TCP_PORT:-9943}"

export CLAW_POOL_ID="${CLAW_POOL_ID:-${_base_pool_id}}"
export CLAW_PODMAN_POOL_SIZE="${CLAW_STRICT_PODMAN_POOL_SIZE:-${CLAW_PODMAN_POOL_SIZE:-5}}"
export CLAW_PODMAN_POOL_MIN_IDLE="${CLAW_STRICT_PODMAN_POOL_MIN_IDLE:-${CLAW_PODMAN_POOL_MIN_IDLE:-1}}"
export CLAW_DOCKER_POOL_SIZE="${CLAW_STRICT_DOCKER_POOL_SIZE:-${CLAW_STRICT_PODMAN_POOL_SIZE:-${CLAW_DOCKER_POOL_SIZE:-${CLAW_PODMAN_POOL_SIZE:-5}}}}"
export CLAW_DOCKER_POOL_MIN_IDLE="${CLAW_STRICT_DOCKER_POOL_MIN_IDLE:-${CLAW_STRICT_PODMAN_POOL_MIN_IDLE:-${CLAW_DOCKER_POOL_MIN_IDLE:-${CLAW_PODMAN_POOL_MIN_IDLE:-1}}}}"

if claw_relaxed_worker_allowed_from_env; then
  _pool_allow_relaxed=true
  _pool_emit_relaxed_image=1
else
  _pool_allow_relaxed=false
  _pool_emit_relaxed_image=0
fi

export CLAW_POOL_HTTP_PORT="${HTTP_PORT}"
if [[ -n "${CLAW_POOL_DAEMON_BIN:-}" ]]; then
  BIN="${CLAW_POOL_DAEMON_BIN}"
else
  BIN="$(claw_ensure_pool_daemon_binary "${PODMAN_DIR}" "${REPO_ROOT}")" || exit 1
fi

case "${CLAW_POOL_RPC_TRANSPORT:-}" in
  tcp | unix | http) TRANSPORT="${CLAW_POOL_RPC_TRANSPORT}" ;;
  *)
    if [[ "$(claw_deploy_profile_name 2>/dev/null || true)" == local ]]; then
      TRANSPORT=http
    elif [[ "$(uname -s)" == "Linux" ]] && [[ "$(claw_deploy_profile_name 2>/dev/null || true)" == production ]]; then
      TRANSPORT=tcp
    elif declare -F claw_pool_rpc_transport >/dev/null 2>&1; then
      TRANSPORT="$(claw_pool_rpc_transport)"
    else
      TRANSPORT=tcp
    fi
    ;;
esac
RUN_SH="${RPC_DIR}/pool-daemon-run.sh"
LOG="${RPC_DIR}/daemon.log"
LOCKDIR="${RPC_DIR}/.pool-up.lockdir"

mkdir -p "${RPC_DIR}"
chmod 1777 "${RPC_DIR}" 2>/dev/null || true

if ! mkdir "${LOCKDIR}" 2>/dev/null; then
  echo "error: another pool-daemon-up is running" >&2
  exit 1
fi
trap 'rmdir "${LOCKDIR}" 2>/dev/null || true' EXIT

claw_pool_http_alive() {
  curl -fsS --connect-timeout 2 "http://127.0.0.1:${HTTP_PORT}/healthz/live-report" >/dev/null 2>&1
}

if [[ "${RESTART}" == 0 ]] && claw_pool_http_alive; then
  # shellcheck source=claw-pool-registry-env.sh
  source "${LIB_DIR}/claw-pool-registry-env.sh"
  claw_export_pool_registry_env "${RPC_DIR}"
  if claw_pool_registry_row_fresh "${PODMAN_DIR}"; then
    pid="$(claw_pool_refresh_pid_file "${RPC_DIR}" 2>/dev/null || true)"
    if [[ -z "${pid}" ]] && [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]]; then
      # shellcheck disable=SC1091
      source "${LIB_DIR}/pool-daemon-systemd.sh"
      if claw_pool_use_systemd 2>/dev/null && claw_pool_systemd_active; then
        pid="$(claw_pool_systemd_main_pid)"
      fi
    fi
    echo "claw-sandbox already on 127.0.0.1:${HTTP_PORT} (pid=${pid:-unknown}, claw_pool ok, skipped)" >&2
    exit 0
  fi
  echo "==> pool-daemon-up: HTTP up but claw_pool registry missing/stale for ${CLAW_POOL_ID}; restarting…" >&2
  RESTART=1
fi

if [[ "${RESTART}" == 1 ]]; then
  echo "==> pool-daemon-up: --restart" >&2
  export CLAW_POOL_UP_ENV_FILE="${_pool_env_file}"
  "${PODMAN_DIR}/lib/pool-daemon-down.sh"
  if [[ "${WITH_WORKERS}" == 1 ]]; then
    # shellcheck disable=SC1091
    source "${LIB_DIR}/nuclear-pool-reset.sh"
    claw_remove_all_gateway_workers
  fi
elif claw_pool_http_alive; then
  exit 0
else
  echo "==> pool-daemon-up: HTTP down, starting…" >&2
fi

# shellcheck source=claw-pool-registry-env.sh
source "${LIB_DIR}/claw-pool-registry-env.sh"
claw_export_pool_registry_env "${RPC_DIR}"

if [[ ! -x "${BIN}" ]]; then
  echo "error: claw-sandbox missing: ${BIN}" >&2
  exit 1
fi

_iso="${CLAW_SOLVE_ISOLATION:-podman_pool}"
if [[ "${_iso}" == docker_pool && -z "${CLAW_DOCKER_IMAGE:-}" ]]; then
  echo "error: CLAW_DOCKER_IMAGE unset (production docker_pool)" >&2
  exit 1
fi
if ! pool_db_url="$(claw_pool_daemon_database_url 2>/dev/null)"; then
  echo "error: CLAW_GATEWAY_DATABASE_URL unset in .env" >&2
  exit 1
fi

claw_pool_env_kv() {
  local k="$1" v="$2"
  v="${v//\'/\'\\\'\'}"
  printf "%s='%s'\n" "$k" "$v"
}
{
  claw_pool_env_kv CLAW_POOL_DAEMON_BIN "${BIN}"
  claw_pool_env_kv CLAW_REPO_ROOT "${REPO_ROOT}"
  claw_pool_env_kv CLAW_WORK_ROOT "${WORK_ROOT}"
  claw_pool_env_kv CLAW_POOL_WORK_ROOT_HOST "${WORK_ROOT}"
  claw_pool_env_kv CLAW_SOLVE_ISOLATION "${CLAW_SOLVE_ISOLATION:-podman_pool}"
  claw_pool_env_kv CLAW_WORKER_ENV_FILE "${CLAW_WORKER_ENV_FILE:-${REPO_ROOT}/.env}"
  claw_pool_env_kv CLAW_POOL_HTTP_BIND "0.0.0.0:${HTTP_PORT}"
  if [[ "${CLAW_POOL_ADVERTISE_HOST_PINNED:-0}" == 1 ]]; then
    claw_pool_env_kv CLAW_POOL_ADVERTISE_HOST "${CLAW_POOL_ADVERTISE_HOST}"
  fi
  claw_pool_env_kv CLAW_POOL_ADVERTISE_HOST_PINNED "${CLAW_POOL_ADVERTISE_HOST_PINNED:-0}"
  claw_pool_env_kv CLAW_POOL_ID "${CLAW_POOL_ID}"
  [[ -n "${CLAW_POOL_GATEWAY_BASE:-}" ]] && claw_pool_env_kv CLAW_POOL_GATEWAY_BASE "${CLAW_POOL_GATEWAY_BASE}"
  [[ -n "${GATEWAY_HOST_PORT:-}" ]] && claw_pool_env_kv GATEWAY_HOST_PORT "${GATEWAY_HOST_PORT}"
  [[ -n "${CLAW_DOCKER_IMAGE:-}" ]] && claw_pool_env_kv CLAW_DOCKER_IMAGE "${CLAW_DOCKER_IMAGE}"
  [[ -n "${CLAW_PODMAN_IMAGE:-}" ]] && claw_pool_env_kv CLAW_PODMAN_IMAGE "${CLAW_PODMAN_IMAGE}"
  [[ -n "${CLAW_DOCKER_NETWORK:-}" ]] && claw_pool_env_kv CLAW_DOCKER_NETWORK "${CLAW_DOCKER_NETWORK}"
  [[ -n "${CLAW_PODMAN_NETWORK:-}" ]] && claw_pool_env_kv CLAW_PODMAN_NETWORK "${CLAW_PODMAN_NETWORK}"
  [[ -n "${CLAW_DOCKER_POOL_SIZE:-}" ]] && claw_pool_env_kv CLAW_DOCKER_POOL_SIZE "${CLAW_DOCKER_POOL_SIZE}"
  [[ -n "${CLAW_PODMAN_POOL_SIZE:-}" ]] && claw_pool_env_kv CLAW_PODMAN_POOL_SIZE "${CLAW_PODMAN_POOL_SIZE}"
  [[ -n "${CLAW_DOCKER_POOL_MIN_IDLE:-}" ]] && claw_pool_env_kv CLAW_DOCKER_POOL_MIN_IDLE "${CLAW_DOCKER_POOL_MIN_IDLE}"
  [[ -n "${CLAW_PODMAN_POOL_MIN_IDLE:-}" ]] && claw_pool_env_kv CLAW_PODMAN_POOL_MIN_IDLE "${CLAW_PODMAN_POOL_MIN_IDLE}"
  [[ -n "${CLAW_DOCKER_EXTRA_ARGS:-}" ]] && claw_pool_env_kv CLAW_DOCKER_EXTRA_ARGS "${CLAW_DOCKER_EXTRA_ARGS}"
  [[ -n "${CLAW_PODMAN_EXTRA_ARGS:-}" ]] && claw_pool_env_kv CLAW_PODMAN_EXTRA_ARGS "${CLAW_PODMAN_EXTRA_ARGS}"
  if [[ "${_pool_emit_relaxed_image:-0}" == 1 ]]; then
    _relaxed_img="${CLAW_RELAXED_PODMAN_IMAGE:-claw-gateway-worker-relaxed:local}"
    claw_pool_env_kv CLAW_PODMAN_RELAXED_IMAGE "${_relaxed_img}"
    claw_pool_env_kv CLAW_DOCKER_RELAXED_IMAGE "${_relaxed_img}"
    claw_pool_env_kv CLAW_RELAXED_PODMAN_IMAGE "${_relaxed_img}"
    _relaxed_pool_size="${CLAW_RELAXED_PODMAN_POOL_SIZE:-${CLAW_PODMAN_RELAXED_POOL_SIZE:-1}}"
    _relaxed_pool_min_idle="${CLAW_RELAXED_PODMAN_POOL_MIN_IDLE:-${CLAW_PODMAN_RELAXED_POOL_MIN_IDLE:-0}}"
    claw_pool_env_kv CLAW_PODMAN_RELAXED_POOL_SIZE "${_relaxed_pool_size}"
    claw_pool_env_kv CLAW_DOCKER_RELAXED_POOL_SIZE "${_relaxed_pool_size}"
    claw_pool_env_kv CLAW_PODMAN_RELAXED_POOL_MIN_IDLE "${_relaxed_pool_min_idle}"
    claw_pool_env_kv CLAW_DOCKER_RELAXED_POOL_MIN_IDLE "${_relaxed_pool_min_idle}"
  fi
  _pool_exec_user="${CLAW_PODMAN_POOL_EXEC_USER:-claw}"
  claw_pool_env_kv CLAW_PODMAN_POOL_EXEC_USER "${_pool_exec_user}"
  claw_pool_env_kv CLAW_DOCKER_POOL_EXEC_USER "${CLAW_DOCKER_POOL_EXEC_USER:-${_pool_exec_user}}"
  claw_pool_env_kv CLAW_ALLOW_RELAXED_WORKER "${_pool_allow_relaxed}"
  claw_pool_env_kv CLAW_SECURITY_BOOST "${CLAW_SECURITY_BOOST:-true}"
  claw_pool_env_kv CLAW_GATEWAY_DATABASE_URL "${pool_db_url}"
  for _otel_k in CLAW_OTEL_ENABLED CLAW_OTEL_LOG_PROMPTS LANGFUSE_PUBLIC_KEY LANGFUSE_SECRET_KEY LANGFUSE_BASE_URL OTEL_EXPORTER_OTLP_ENDPOINT OTEL_EXPORTER_OTLP_HEADERS; do
    if [[ -n "${!_otel_k:-}" ]]; then
      claw_pool_env_kv "${_otel_k}" "${!_otel_k}"
    fi
  done
  if [[ "${TRANSPORT}" == tcp ]]; then
    claw_pool_env_kv CLAW_POOL_DAEMON_TCP_BIND "0.0.0.0:${_pool_daemon_tcp_port}"
  elif [[ "${TRANSPORT}" == unix ]]; then
    claw_pool_env_kv CLAW_POOL_DAEMON_LISTEN "$(claw_pool_host_socket_path "${PODMAN_DIR}")"
  fi
} >"${RPC_DIR}/pool-daemon.env"

for _pool_proc_key in CLAW_ALLOW_RELAXED_WORKER CLAW_SECURITY_BOOST; do
  if ! grep -q "^${_pool_proc_key}=" "${RPC_DIR}/pool-daemon.env"; then
    echo "error: pool-daemon.env missing required key ${_pool_proc_key}" >&2
    exit 1
  fi
done

cp -f "${LIB_DIR}/pool-daemon-run.sh" "${RUN_SH}"
chmod +x "${RUN_SH}"

printf '\n%s pool-daemon-up: starting %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${BIN}" >>"${LOG}"

_pool_supervisor=""
if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-launchd.sh"
  claw_pool_launchd_bootstrap "${RPC_DIR}" "${RUN_SH}" "${LOG}"
  _pool_supervisor=launchd
elif [[ -f "${LIB_DIR}/pool-daemon-docker.sh" ]] && {
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-docker.sh"
  claw_pool_use_docker_supervisor
}; then
  claw_pool_docker_up "${RPC_DIR}" "${BIN}" "${REPO_ROOT}" "${WORK_ROOT}"
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-docker.sh"
  pid="$(docker inspect -f '{{.State.Pid}}' "$(claw_pool_docker_container_name)" 2>/dev/null || true)"
  [[ -n "${pid}" && "${pid}" != "0" ]] && printf '%s' "${pid}" >"${RPC_DIR}/daemon.pid"
  _pool_supervisor=docker
elif [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]] && {
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-systemd.sh"
  claw_pool_use_systemd
}; then
  claw_pool_systemd_install_and_restart "${RPC_DIR}" "${RUN_SH}" "${REPO_ROOT}"
  pid="$(claw_pool_systemd_main_pid)"
  [[ -n "${pid}" && "${pid}" != "0" ]] && printf '%s' "${pid}" >"${RPC_DIR}/daemon.pid"
  _pool_supervisor=systemd
else
  case "${CLAW_POOL_DAEMON_USE_SYSTEMD:-}" in
    1 | true | yes | on)
      echo "error: CLAW_POOL_DAEMON_USE_SYSTEMD=1 but systemd install failed" >&2
      echo "hint: GitHub/GitLab runner needs passwordless sudo for systemctl (see deploy/stack/docs/github-ci-variables.md)" >&2
      exit 1
      ;;
  esac
  set -a
  # shellcheck disable=SC1090
  source "${RPC_DIR}/pool-daemon.env"
  set +a
  nohup "${BIN}" >>"${LOG}" 2>&1 < /dev/null &
  pid=$!
  printf '%s' "${pid}" >"${RPC_DIR}/daemon.pid"
  disown "${pid}" 2>/dev/null || true
  _pool_supervisor=nohup
fi

for _i in $(seq 1 120); do
  if [[ "${_pool_supervisor}" == docker ]]; then
    # shellcheck disable=SC1091
    source "${LIB_DIR}/pool-daemon-docker.sh"
    if ! claw_pool_docker_running 2>/dev/null; then
      echo "error: claw-sandbox docker container exited before :${HTTP_PORT} was ready" >&2
      claw_pool_docker_dump_logs
      exit 1
    fi
  fi
  if claw_pool_http_alive; then
    if [[ "${_pool_supervisor}" == docker ]]; then
      # shellcheck disable=SC1091
      source "${LIB_DIR}/pool-daemon-docker.sh"
      pid="$(docker inspect -f '{{.State.Pid}}' "$(claw_pool_docker_container_name)" 2>/dev/null || true)"
    else
      pid="$(claw_pool_refresh_pid_file "${RPC_DIR}" 2>/dev/null || echo "${pid:-}")"
    fi
    echo "claw-sandbox HTTP 0.0.0.0:${HTTP_PORT} (pid=${pid}, supervisor=${_pool_supervisor:-unknown})" >&2
    echo "  pool_id=${CLAW_POOL_ID} advertise=${CLAW_POOL_ADVERTISE_HOST}" >&2
    exit 0
  fi
  if [[ "${_i}" == 120 ]]; then
    echo "error: claw-sandbox did not listen on :${HTTP_PORT}" >&2
    if [[ "${_pool_supervisor}" == docker ]]; then
      # shellcheck disable=SC1091
      source "${LIB_DIR}/pool-daemon-docker.sh"
      claw_pool_docker_dump_logs
    else
      tail -20 "${LOG}" >&2 || true
    fi
    exit 1
  fi
  sleep 0.1
done
