#!/usr/bin/env bash
# Host pool: default ensure (skip if HTTP up). --restart = down then up. Author: kejiqing
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
  esac
done

_pool_bin_from_up="${CLAW_POOL_DAEMON_BIN:-}"
if [[ -f "${REPO_ROOT}/.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${REPO_ROOT}/.env"
  set +a
fi
if [[ -n "${_pool_bin_from_up}" ]]; then
  export CLAW_POOL_DAEMON_BIN="${_pool_bin_from_up}"
fi

claw_apply_deploy_profile 2>/dev/null || true
# pool-up alone must see release pin + GATEWAY_IMAGE→CLAW_DOCKER_IMAGE (same as gateway.sh up). kejiqing
# shellcheck source=release-images.sh
source "${LIB_DIR}/release-images.sh"
claw_reapply_pool_image_pins "${PODMAN_DIR}"

if [[ -z "${CLAW_POOL_WORK_ROOT_BIND_SRC:-}" ]]; then
  claw_podman_export_pool_workspace "${PODMAN_DIR}"
fi

WORK_ROOT="${CLAW_POOL_WORK_ROOT_BIND_SRC:?missing CLAW_POOL_WORK_ROOT_BIND_SRC; run gateway.sh up first}"
RPC_DIR="${PODMAN_DIR}/.claw-pool-rpc"
BIN="${CLAW_POOL_DAEMON_BIN:-$(claw_default_pool_daemon_bin "${PODMAN_DIR}")}"
HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"
case "${CLAW_POOL_RPC_TRANSPORT:-}" in
  tcp | unix) TRANSPORT="${CLAW_POOL_RPC_TRANSPORT}" ;;
  *)
    # Linux production: gateway container → host pool TCP :9943. kejiqing
    if [[ "$(uname -s)" == "Linux" ]] && [[ "$(claw_deploy_profile_name 2>/dev/null || true)" == production ]]; then
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
  pid="$(claw_pool_refresh_pid_file "${RPC_DIR}" 2>/dev/null || true)"
  if [[ -z "${pid}" ]] && [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]]; then
    # shellcheck disable=SC1091
    source "${LIB_DIR}/pool-daemon-systemd.sh"
    if claw_pool_use_systemd 2>/dev/null && claw_pool_systemd_active; then
      pid="$(claw_pool_systemd_main_pid)"
    fi
  fi
  echo "claw-pool-daemon already on 127.0.0.1:${HTTP_PORT} (pid=${pid:-unknown}, skipped)" >&2
  exit 0
fi

if [[ "${RESTART}" == 1 ]]; then
  echo "==> pool-daemon-up: --restart" >&2
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
  echo "error: claw-pool-daemon missing: ${BIN}" >&2
  exit 1
fi

_iso="${CLAW_SOLVE_ISOLATION:-podman_pool}"
if [[ "${_iso}" == docker_pool && -z "${CLAW_DOCKER_IMAGE:-}" ]]; then
  echo "error: CLAW_DOCKER_IMAGE unset (production docker_pool)" >&2
  echo "  fix: add GATEWAY_IMAGE=ghcr.io/.../claw-code:release-vX.Y.Z to .env" >&2
  echo "    or run once: ./deploy/stack/gateway.sh up --release release-vX.Y.Z" >&2
  echo "    then pool-up (reads deploy/stack/.claw-image-release.env)" >&2
  exit 1
fi
if ! pool_db_url="$(claw_pool_daemon_database_url 2>/dev/null)"; then
  echo "error: CLAW_GATEWAY_DATABASE_URL unset in .env" >&2
  echo "  fix: CLAW_GATEWAY_DATABASE_URL=postgres://claw_gateway:PASS@postgres:5432/claw_gateway" >&2
  echo "  (host pool rewrites @postgres:5432 → 127.0.0.1:\${CLAW_GATEWAY_PG_HOST_PORT:-5433})" >&2
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
  claw_pool_env_kv CLAW_POOL_HTTP_BIND "0.0.0.0:${CLAW_POOL_HTTP_PORT:-9944}"
  claw_pool_env_kv CLAW_POOL_ADVERTISE_HOST "${CLAW_POOL_ADVERTISE_HOST}"
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
  claw_pool_env_kv CLAW_GATEWAY_DATABASE_URL "${pool_db_url}"
  if [[ "${TRANSPORT}" == tcp ]]; then
    claw_pool_env_kv CLAW_POOL_DAEMON_TCP_BIND "0.0.0.0:${CLAW_POOL_DAEMON_PORT:-9943}"
  elif [[ "${TRANSPORT}" == unix ]]; then
    claw_pool_env_kv CLAW_POOL_DAEMON_LISTEN "$(claw_pool_host_socket_path "${PODMAN_DIR}")"
  fi
} >"${RPC_DIR}/pool-daemon.env"

cp -f "${LIB_DIR}/pool-daemon-run.sh" "${RUN_SH}"
chmod +x "${RUN_SH}"

printf '\n%s pool-daemon-up: starting %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${BIN}" >>"${LOG}"

# macOS: launchd owns the process (agent shell teardown must not SIGKILL pool). kejiqing
if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-launchd.sh"
  claw_pool_launchd_bootstrap "${RPC_DIR}" "${RUN_SH}" "${LOG}"
elif [[ -f "${LIB_DIR}/pool-daemon-systemd.sh" ]] && {
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-daemon-systemd.sh"
  claw_pool_use_systemd
}; then
  claw_pool_systemd_install_and_restart "${RPC_DIR}" "${RUN_SH}" "${REPO_ROOT}"
  pid="$(claw_pool_systemd_main_pid)"
  [[ -n "${pid}" && "${pid}" != "0" ]] && printf '%s' "${pid}" >"${RPC_DIR}/daemon.pid"
else
  # Linux local / fallback: nohup as current user.
  set -a
  # shellcheck disable=SC1090
  source "${RPC_DIR}/pool-daemon.env"
  set +a
  nohup "${BIN}" >>"${LOG}" 2>&1 < /dev/null &
  pid=$!
  printf '%s' "${pid}" >"${RPC_DIR}/daemon.pid"
  disown "${pid}" 2>/dev/null || true
fi

for _i in $(seq 1 120); do
  if claw_pool_http_alive; then
    pid="$(claw_pool_refresh_pid_file "${RPC_DIR}" 2>/dev/null || echo "${pid}")"
    echo "claw-pool-daemon HTTP 0.0.0.0:${HTTP_PORT} (pid=${pid})" >&2
    echo "  pool_id=${CLAW_POOL_ID} advertise=${CLAW_POOL_ADVERTISE_HOST}" >&2
    exit 0
  fi
  if [[ "${_i}" == 120 ]]; then
    echo "error: claw-pool-daemon did not listen on :${HTTP_PORT}" >&2
    tail -20 "${LOG}" >&2 || true
    exit 1
  fi
  sleep 0.1
done
