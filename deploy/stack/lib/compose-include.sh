# shellcheck shell=bash
# Sets CLAW_POOL_WORK_ROOT_HOST and CLAW_PODMAN_COMPOSE_ARGS. Default solve mode is podman_pool (second compose file). Author: kejiqing

# Resolve a socket path that exists on the **host** (compose / pool-daemon talk to the API here).
# Docker hosts: /var/run/docker.sock only. Podman: never guess docker.sock on macOS (VM socket ≠ host path).
# Author: kejiqing
claw_socket_usable() {
  local s="$1"
  [[ -n "${s}" && -S "${s}" && -r "${s}" && -w "${s}" ]]
}

# RPC transport: local default Unix socket (bind-mount, no host.containers.internal:9943).
# Production: set CLAW_POOL_DAEMON_TCP_HOST (or CLAW_POOL_RPC_TRANSPORT=tcp). Author: kejiqing
claw_pool_rpc_transport() {
  case "${CLAW_POOL_RPC_TRANSPORT:-}" in
    tcp | unix)
      printf '%s' "${CLAW_POOL_RPC_TRANSPORT}"
      return 0
      ;;
  esac
  if [[ -n "${CLAW_POOL_DAEMON_TCP_HOST:-}" ]]; then
    case "${CLAW_POOL_DAEMON_TCP_HOST}" in
      host.containers.internal | host.docker.internal)
        if [[ "$(uname -s)" == Darwin ]]; then
          printf '%s' tcp
        else
          printf '%s' unix
        fi
        ;;
      *)
        printf '%s' tcp
        ;;
    esac
    return 0
  fi
  if [[ "$(uname -s)" == Darwin ]]; then
    printf '%s' tcp
  else
    printf '%s' unix
  fi
}

claw_pool_host_socket_path() {
  local script_dir="${1:?script_dir}"
  printf '%s' "${script_dir}/.claw-pool-rpc/pool.sock"
}

claw_pool_container_socket_path() {
  printf '%s' '/run/claw-pool-rpc/pool.sock'
}

# v1: host `claw-pool-daemon` only (no compose pool sidecar). Set CLAW_POOL_HOST_DAEMON=0 to fail fast. Author: kejiqing
claw_pool_daemon_on_host() {
  case "${CLAW_POOL_HOST_DAEMON:-}" in
    0 | false | no)
      echo "error: compose pool sidecar removed; use host claw-pool-daemon (unset CLAW_POOL_HOST_DAEMON or =1)" >&2
      return 1
      ;;
  esac
  return 0
}

# Gateway container -> host pool RPC: production uses LAN IP (e.g. 192.168.9.252), not host.docker.internal. kejiqing
claw_pool_gateway_to_host_rpc_ip() {
  if [[ -n "${CLAW_POOL_DAEMON_TCP_HOST:-}" ]]; then
    printf '%s' "${CLAW_POOL_DAEMON_TCP_HOST}"
    return 0
  fi
  if [[ -n "${CLAW_POOL_ADVERTISE_HOST:-}" ]]; then
    printf '%s' "${CLAW_POOL_ADVERTISE_HOST}"
    return 0
  fi
  if [[ "$(uname -s)" == Darwin ]]; then
    if [[ "$(claw_container_runtime_cli 2>/dev/null || true)" == docker ]]; then
      printf '%s' '127.0.0.1'
    else
      printf '%s' 'host.containers.internal'
    fi
    return 0
  fi
  echo "error: set CLAW_POOL_DAEMON_TCP_HOST or CLAW_POOL_ADVERTISE_HOST to this machine LAN IP (e.g. 192.168.9.252)" >&2
  return 1
}

claw_podman_machine_host_socket() {
  command -v podman >/dev/null 2>&1 || return 1
  local p=""
  p="$(podman machine inspect --format '{{.ConnectionInfo.PodmanSocket.Path}}' 2>/dev/null || true)"
  if claw_socket_usable "${p}"; then
    printf '%s' "${p}"
    return 0
  fi
  return 1
}

claw_container_socket_path() {
  if [[ -n "${CLAW_CONTAINER_SOCKET:-}" ]]; then
    printf '%s' "${CLAW_CONTAINER_SOCKET}"
    return 0
  fi

  local rt
  rt="$(claw_container_runtime_cli)" || return 1

  if [[ "${rt}" == docker ]]; then
    printf '%s' /var/run/docker.sock
    return 0
  fi

  # podman — try host-visible sockets only (skip VM-only paths from `podman info` on macOS).
  local -a tried=() p=""
  if command -v podman >/dev/null 2>&1; then
    p="$(podman info --format '{{.Host.RemoteSocket.Path}}' 2>/dev/null || true)"
    if [[ -n "${p}" && "${p}" != "<nil>" ]]; then
      tried+=("${p}")
      if claw_socket_usable "${p}"; then
        printf '%s' "${p}"
        return 0
      fi
    fi
    if p="$(claw_podman_machine_host_socket)"; then
      tried+=("${p}")
      printf '%s' "${p}"
      return 0
    fi
  fi

  local uid_path="/run/user/$(id -u)/podman/podman.sock"
  tried+=("${uid_path}" /run/podman/podman.sock)
  for p in "${uid_path}" /run/podman/podman.sock; do
    if claw_socket_usable "${p}"; then
      printf '%s' "${p}"
      return 0
    fi
  done

  # Linux only: podman-docker shim may expose docker.sock (do not use this default on Darwin).
  if [[ "$(uname -s)" != Darwin ]] && claw_socket_usable /var/run/docker.sock; then
    printf '%s' /var/run/docker.sock
    return 0
  fi

  echo "error: no podman API socket on host (runtime=podman)" >&2
  echo "  tried: ${tried[*]}" >&2
  echo "hint: macOS → podman machine start; Linux rootless → set CLAW_CONTAINER_SOCKET in .env" >&2
  echo "      (podman info --format '{{.Host.RemoteSocket.Path}}' or podman machine inspect)" >&2
  return 1
}

# docker-compose v1 (podman compose backend on many Linux hosts) needs DOCKER_HOST + socket RW. Author: kejiqing
claw_compose_prepare_socket() {
  local sock rt
  rt="$(claw_container_runtime_cli)" || return 1
  sock="$(claw_container_socket_path)" || return 1
  export CLAW_CONTAINER_SOCKET="${sock}"
  export DOCKER_HOST="unix://${sock}"
  if [[ ! -S "${sock}" ]]; then
    echo "error: container API socket not found: ${sock}" >&2
    echo "hint: start ${rt}, or set CLAW_CONTAINER_SOCKET in .env" >&2
    return 1
  fi
  if [[ ! -r "${sock}" || ! -w "${sock}" ]]; then
    echo "error: permission denied on ${sock} (compose cannot talk to ${rt})" >&2
    echo "hint: sudo usermod -aG podman \"\$(whoami)\"  # or: docker" >&2
    echo "      then log out and SSH in again (or: newgrp podman)" >&2
    return 1
  fi
}

# Host pool daemon env (legacy name; v1 no compose sidecar). Author: kejiqing
claw_podman_write_pool_daemon_sidecar_env() {
  local script_dir="$1"
  local repo_root ws sock
  repo_root="$(cd "${script_dir}/../.." && pwd)"
  ws="${CLAW_POOL_WORK_ROOT_BIND_SRC:?CLAW_POOL_WORK_ROOT_BIND_SRC unset; call claw_podman_export_pool_workspace first}"
  sock="$(claw_container_socket_path)" || return 1
  export CLAW_REPO_ROOT="${repo_root}"
  export CLAW_CONTAINER_SOCKET="${sock}"
  {
    printf '%s\n' '# GENERATED — do not edit. Overwritten by compose-include (host pool). kejiqing'
    printf '%s\n' "CLAW_WORK_ROOT=${ws}"
    printf '%s\n' "CLAW_POOL_WORK_ROOT_HOST=${ws}"
    printf '%s\n' "CLAW_WORKER_ENV_FILE=${script_dir}/.claw-worker-runtime.env:${repo_root}/.env"
    if [[ -n "${CLAW_PODMAN_NETWORK:-}" ]]; then
      printf '%s\n' "CLAW_PODMAN_NETWORK=${CLAW_PODMAN_NETWORK}"
    fi
    printf '%s\n' "CLAW_POOL_HTTP_BIND=0.0.0.0:${CLAW_POOL_HTTP_PORT:-9944}"
    if [[ -n "${CLAW_POOL_ADVERTISE_HOST:-}" ]]; then
      printf '%s\n' "CLAW_POOL_ADVERTISE_HOST=${CLAW_POOL_ADVERTISE_HOST}"
    fi
    if [[ -n "${CLAW_POOL_ID:-}" ]]; then
      printf '%s\n' "CLAW_POOL_ID=${CLAW_POOL_ID}"
    fi
  } >"${script_dir}/.claw-pool-daemon.env"
}

# Repo-root LLM runtime files: gateway (rw) + claude-tap (ro) + pool workers (.env mount). Author: kejiqing
claw_export_llm_runtime_layout() {
  local script_dir="$1"
  local repo_root host_env host_upstream
  repo_root="$(cd "${script_dir}/../.." && pwd)"
  host_env="${repo_root}/.env"
  host_upstream="${repo_root}/.claw/claw-tap-upstream.json"
  mkdir -p "${repo_root}/.claw"
  local legacy="${repo_root}/.openclaw/claude-tap-upstream.json"
  if [[ -f "${legacy}" && ! -f "${host_upstream}" ]]; then
    mv "${legacy}" "${host_upstream}"
    rmdir "${repo_root}/.openclaw" 2>/dev/null || true
  fi
  local llm_runtime="${repo_root}/.claw/claw-llm-runtime.env"
  export CLAW_REPO_ROOT="${repo_root}"
  export CLAW_LLM_RUNTIME_ENV_FILE="${llm_runtime}"
  export CLAW_TAP_UPSTREAM_CONFIG_FILE="${host_upstream}"
  {
    printf '%s\n' '# GENERATED — do not edit. Overwritten by up.sh / tap-up.sh. kejiqing'
    printf '%s\n' '# Human deploy only (ro in gateway container):'
    printf '%s\n' "#   ${host_env}"
    printf '%s\n' '# PG-synced LLM keys (gateway writes; pool workers merge via worker-llm-wiring):'
    printf '%s\n' "#   ${llm_runtime}"
    printf '%s\n' "#   ${host_upstream}"
    printf '%s\n' '# Inside gateway container (bind-mount under /run/claw/claw):'
    printf '%s\n' 'CLAW_REPO_ROOT=/run/claw/claw'
    printf '%s\n' 'CLAW_LLM_RUNTIME_ENV_FILE=/run/claw/claw/claw-llm-runtime.env'
    printf '%s\n' 'CLAW_TAP_UPSTREAM_CONFIG_FILE=/run/claw/claw/claw-tap-upstream.json'
  } >"${script_dir}/.claw-llm-runtime.env"
}

claw_podman_export_pool_workspace() {
  local script_dir="$1"
  mkdir -p "${script_dir}/claw-workspace"
  # Host directory for the compose bind mount (Mac/Linux laptop path). Not the same as CLAW_POOL_WORK_ROOT_HOST
  # inside the gateway container — see .claw-pool-workspace.env below. Author: kejiqing
  export CLAW_POOL_WORK_ROOT_BIND_SRC="$(cd "${script_dir}" && pwd)/claw-workspace"
  # Merged last in podman-compose.yml. MUST be a path that exists inside the gateway container: the gateway
  # runs Linux and calls canonicalize() before podman run. A macOS /Users/... path breaks startup with
  # "No such file or directory". For this stack, pool data lives under the same mount as CLAW_WORK_ROOT.
  {
    printf '%s\n' '# GENERATED — do not edit. Overwritten by up.sh / down.sh / start-with-tap.sh. kejiqing'
    printf '%s\n' 'CLAW_POOL_WORK_ROOT_HOST=/var/lib/claw/workspace'
  } >"${script_dir}/.claw-pool-workspace.env"
}


# Local dev only: bind-mount gateway-admin dist. Production/release always use image SPA. Author: kejiqing
claw_podman_append_admin_dist_bind() {
  local script_dir="$1"
  local rel="${2:-}"
  local repo_root="${CLAW_COMPOSE_WORKING_DIRECTORY:-}"
  local profile
  profile="$(printf '%s' "${CLAW_DEPLOY_PROFILE:-}" | tr '[:upper:]' '[:lower:]')"
  if [[ -z "${profile}" && "$(uname -s)" != Darwin ]]; then
    profile=production
  fi
  if [[ "${profile}" == production ]] || [[ -n "${CLAW_IMAGE_RELEASE_TAG:-}" ]]; then
    return 0
  fi
  local dist=""
  if [[ -n "${repo_root}" ]]; then
    dist="${repo_root}/web/gateway-admin/dist/index.html"
  else
    dist="$(cd "${script_dir}/../.." && pwd)/web/gateway-admin/dist/index.html"
  fi
  if [[ ! -f "${dist}" ]]; then
    return 0
  fi
  local assets_glob
  if [[ -n "${repo_root}" ]]; then
    assets_glob="${repo_root}/web/gateway-admin/dist/assets/*.js"
  else
    assets_glob="$(cd "${script_dir}/../.." && pwd)/web/gateway-admin/dist/assets/*.js"
  fi
  if ! compgen -G "${assets_glob}" >/dev/null 2>&1; then
    echo "note: skip admin-dist bind (no dist/assets/*.js); playground uses image-built admin SPA" >&2
    return 0
  fi
  if [[ -n "${rel}" ]]; then
    CLAW_PODMAN_COMPOSE_ARGS+=( -f "${rel}/podman-compose.admin-dist-bind.yml" )
  else
    CLAW_PODMAN_COMPOSE_ARGS+=( -f "${script_dir}/podman-compose.admin-dist-bind.yml" )
  fi
}

claw_podman_load_compose_args() {
  local script_dir="$1"
  local env_file="$2"
  unset CLAW_COMPOSE_WORKING_DIRECTORY
  script_dir="$(cd "${script_dir}" && pwd)"
  # Absolute `-f /.../deploy/stack/*.yml` makes Compose use `deploy/stack/` as project dir and auto-load
  # `deploy/stack/.env`, which can override `--env-file` image pins. Use `-f` relative to repo root and
  # run compose from that directory. kejiqing
  local repo_root rel=""
  if [[ -f "${env_file}" ]]; then
    repo_root="$(cd "$(dirname "${env_file}")" && pwd)"
    if [[ "${script_dir}" == "${repo_root}/"* ]]; then
      rel="${script_dir#"${repo_root}/"}"
      export CLAW_COMPOSE_WORKING_DIRECTORY="${repo_root}"
    fi
  fi
  export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-claw}"
  if [[ -n "${rel}" ]]; then
    CLAW_PODMAN_COMPOSE_ARGS=( -p "${COMPOSE_PROJECT_NAME}" -f "${rel}/podman-compose.yml" )
  else
    CLAW_PODMAN_COMPOSE_ARGS=( -p "${COMPOSE_PROJECT_NAME}" -f "${script_dir}/podman-compose.yml" )
  fi
  if [[ ! -f "${env_file}" ]]; then
    return 0
  fi
  set -a
  # shellcheck disable=SC1090
  source "${env_file}"
  set +a
  if [[ -n "${PODMAN_HOST_SOCK:-}" ]]; then
    echo "error: PODMAN_HOST_SOCK is no longer used; remove it from .env" >&2
    return 1
  fi
  mkdir -p "${script_dir}/.claw-pool-rpc"
  # Local profile clears legacy TCP keys from human .env before generating gateway.env. kejiqing
  if [[ -f "${script_dir}/lib/env-profile.sh" ]]; then
    # shellcheck source=env-profile.sh
    source "${script_dir}/lib/env-profile.sh"
    claw_apply_deploy_profile 2>/dev/null || true
  fi
  if [[ -f "${script_dir}/.claw-pool-rpc/pool-registry.env" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "${script_dir}/.claw-pool-rpc/pool-registry.env"
    set +a
  elif [[ -f "${script_dir}/lib/claw-pool-registry-env.sh" ]]; then
    # shellcheck source=lib/claw-pool-registry-env.sh
    source "${script_dir}/lib/claw-pool-registry-env.sh"
    claw_export_pool_registry_env "${script_dir}/.claw-pool-rpc"
  fi
  local pool_http_port="${CLAW_POOL_HTTP_PORT:-9944}"
  local http_host profile_name
  if claw_pool_daemon_on_host; then
    profile_name="$(claw_deploy_profile_name 2>/dev/null || true)"
    # v1 host pool on macOS/Linux: gateway container must reach host pool HTTP via
    # Docker-provided host.containers.internal mapping (not LAN IP). kejiqing
    if [[ "${profile_name}" == local ]]; then
      http_host="host.containers.internal"
    else
      http_host="$(claw_pool_gateway_to_host_rpc_ip)" || return 1
    fi
    {
      printf '%s\n' '# GENERATED — host claw-pool-daemon HTTP (live SSE + POST /v1/pool/rpc). kejiqing'
      printf '%s\n' "CLAW_POOL_HTTP_BASE=http://${http_host}:${pool_http_port}"
      printf '%s\n' "CLAW_POOL_RPC_HOST_WORK_ROOT=${CLAW_POOL_WORK_ROOT_BIND_SRC}"
      printf '%s\n' "CLAW_POOL_DAEMON_TCP="
      printf '%s\n' "CLAW_POOL_DAEMON_SOCKET="
      if [[ -n "${CLAW_POOL_ID:-}" ]]; then
        printf '%s\n' "CLAW_POOL_ID=${CLAW_POOL_ID}"
      fi
      if [[ -n "${CLAW_POOL_ADVERTISE_HOST:-}" ]]; then
        printf '%s\n' "# pool registry advertise (claw_pool.advertise_ip): ${CLAW_POOL_ADVERTISE_HOST}"
      fi
    } >"${script_dir}/.claw-pool-rpc/gateway.env"
    claw_podman_append_admin_dist_bind "${script_dir}" "${rel}"
    return 0
  fi
  echo "error: compose pool sidecar removed; start host claw-pool-daemon (gateway.sh up does this on macOS/Linux)" >&2
  return 1
}

# 同一套脚本本地/线上共用：自动选 `podman` 或 `docker`（无需按环境改两套变量）。
# - 默认 auto：PATH 里**优先 podman**，否则 docker（线上常只有 docker；本机常装 podman）。
# - 两台都装了要强制其一：`.env` 里 `CLAW_CONTAINER_RUNTIME=podman` 或 `docker`。
# - 兼容旧 `.env`：`CLAW_USE_DOCKER=1` 等价于 `CLAW_CONTAINER_RUNTIME=docker`（勿与新变量同时矛盾使用）。
# Author: kejiqing
claw_container_runtime_cli() {
  local r
  r=$(printf '%s' "${CLAW_CONTAINER_RUNTIME:-auto}" | tr '[:upper:]' '[:lower:]')
  if [[ "${CLAW_USE_DOCKER:-0}" == "1" ]] && [[ "$r" == "auto" ]]; then
    r=docker
  fi
  case "$r" in
    podman)
      if ! command -v podman >/dev/null 2>&1; then
        echo "error: CLAW_CONTAINER_RUNTIME=podman but podman is not in PATH" >&2
        return 1
      fi
      printf '%s\n' podman
      ;;
    docker)
      if ! command -v docker >/dev/null 2>&1; then
        echo "error: CLAW_CONTAINER_RUNTIME=docker but docker is not in PATH" >&2
        echo "hint: ./deploy/stack/gateway.sh install-docker   (Linux production)" >&2
        return 1
      fi
      printf '%s\n' docker
      ;;
    auto)
      if command -v podman >/dev/null 2>&1; then
        printf '%s\n' podman
      elif command -v docker >/dev/null 2>&1; then
        printf '%s\n' docker
      else
        echo "error: neither podman nor docker in PATH; install one or set CLAW_CONTAINER_RUNTIME" >&2
        if [[ "$(uname -s)" == "Linux" ]]; then
          echo "hint: ./deploy/stack/gateway.sh install-docker   (production docker_pool)" >&2
        fi
        return 1
      fi
      ;;
    *)
      echo "error: CLAW_CONTAINER_RUNTIME must be auto, podman, or docker (got ${CLAW_CONTAINER_RUNTIME})" >&2
      return 1
      ;;
  esac
}

# `docker network exists` missing on older docker.io; fall back to network ls. Author: kejiqing
claw_network_exists() {
  local rt="$1" name="$2"
  if "${rt}" network exists "${name}" >/dev/null 2>&1; then
    return 0
  fi
  "${rt}" network ls --format '{{.Name}}' 2>/dev/null | grep -qx "${name}"
}

claw_network_ensure() {
  local rt="$1" name="$2"
  if claw_network_exists "${rt}" "${name}"; then
    return 0
  fi
  echo "creating ${rt} network ${name} …" >&2
  if ! "${rt}" network create "${name}" 2>/dev/null; then
    claw_network_exists "${rt}" "${name}" || return 1
  fi
}

claw_compose_ensure_project_network() {
  local rt project net
  rt="$(claw_container_runtime_cli)" || return 1
  project="${COMPOSE_PROJECT_NAME:-claw}"
  net="${project}_default"
  claw_network_ensure "${rt}" "${net}"
}

claw_compose() {
  local rt
  rt="$(claw_container_runtime_cli)" || return 1
  claw_compose_prepare_socket || return 1
  if [[ "${rt}" == podman ]]; then
    claw_compose_ensure_project_network || return 1
  fi
  if [[ -n "${CLAW_COMPOSE_WORKING_DIRECTORY:-}" ]]; then
    (
      cd "${CLAW_COMPOSE_WORKING_DIRECTORY}" || exit 1
      claw_compose_in_pwd "$rt" "$@"
    )
    return $?
  fi
  claw_compose_in_pwd "$rt" "$@"
}

claw_compose_in_pwd() {
  local rt="$1"
  shift
  if [[ "$rt" == docker ]]; then
    # Ubuntu 22.04 packages may ship `docker-compose` without the `docker compose` CLI plugin.
    if docker compose version >/dev/null 2>&1; then
      docker compose "$@"
    elif command -v docker-compose >/dev/null 2>&1; then
      docker-compose "$@"
    else
      echo "error: need \`docker compose\` (plugin) or \`docker-compose\` on PATH" >&2
      return 1
    fi
    return
  fi
  if [[ -z "${PODMAN_COMPOSE_PROVIDER:-}" ]] && command -v podman-compose >/dev/null 2>&1; then
    PODMAN_COMPOSE_PROVIDER="$(command -v podman-compose)"
    export PODMAN_COMPOSE_PROVIDER
  fi
  podman compose "$@"
}

# Postgres: `gateway.sh pg-up` / `pg-down`; `up` / `down` only gateway + pool. kejiqing
claw_compose_pg_container_name() {
  printf '%s' "${CLAW_COMPOSE_PG_CONTAINER:-claw-gateway-postgres}"
}

claw_compose_pg_service() {
  printf '%s' "${CLAW_COMPOSE_PG_SERVICE:-postgres}"
}

# Bundled compose postgres when URL uses service name or host-published loopback. kejiqing
claw_compose_uses_local_postgres() {
  local url="${CLAW_GATEWAY_DATABASE_URL:-postgres://claw_gateway:clawGw9Dev_Pg@postgres:5432/claw_gateway}"
  case "${url}" in
    *@postgres:* | *@postgres/*)
      return 0
      ;;
    *@127.0.0.1:* | *@127.0.0.1/* | *@localhost:* | *@localhost/*)
      return 0
      ;;
  esac
  return 1
}

claw_compose_pg_network() {
  local rt cname
  rt="$(claw_container_runtime_cli)" || return 1
  cname="$(claw_compose_pg_container_name)"
  if ! "${rt}" container exists "${cname}" >/dev/null 2>&1; then
    return 1
  fi
  "${rt}" inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{"\n"}}{{end}}' "${cname}" 2>/dev/null | head -1
}

# Attach gateway-rs to the network where claw-gateway-postgres already runs (e.g. legacy stack_default). kejiqing
claw_compose_append_pg_network_override() {
  local script_dir="$1"
  local net rel="${2:-}"
  net="$(claw_compose_pg_network)" || return 0
  [[ -z "${net}" ]] && return 0
  local override="${script_dir}/.claw-postgres-network.override.yml"
  {
    printf '%s\n' '# GENERATED — do not edit. kejiqing'
    printf '%s\n' 'services:'
    printf '%s\n' '  gateway-rs:'
    printf '%s\n' '    networks:'
    printf '%s\n' '      - claw_pg_net'
    printf '%s\n' 'networks:'
    printf '%s\n' '  claw_pg_net:'
    printf '%s\n' '    external: true'
    printf '%s\n' "    name: ${net}"
  } >"${override}"
  if [[ -n "${rel}" ]]; then
    CLAW_PODMAN_COMPOSE_ARGS+=( -f "${rel}/.claw-postgres-network.override.yml" )
  else
    CLAW_PODMAN_COMPOSE_ARGS+=( -f "${override}" )
  fi
}

claw_compose_prune_stale_claw_pod() {
  command -v podman >/dev/null 2>&1 || return 0
  local project pod_name pg_pod claw_pod
  project="${COMPOSE_PROJECT_NAME:-claw}"
  pod_name="pod_${project}"
  podman pod exists "${pod_name}" >/dev/null 2>&1 || return 0
  if ! podman container exists "$(claw_compose_pg_container_name)" >/dev/null 2>&1; then
    local n
    n="$(podman pod ps --filter "name=${pod_name}" --format '{{.Containers}}' 2>/dev/null | head -1 || echo 0)"
    if [[ "${n}" == "0" || -z "${n}" ]]; then
      podman pod rm -f "${pod_name}" >/dev/null 2>&1 || true
    fi
    return 0
  fi
  pg_pod="$(podman inspect -f '{{.Pod}}' "$(claw_compose_pg_container_name)" 2>/dev/null || true)"
  claw_pod="$(podman pod inspect -f '{{.Id}}' "${pod_name}" 2>/dev/null || true)"
  if [[ -n "${pg_pod}" && -n "${claw_pod}" && "${pg_pod}" != "${claw_pod}" ]]; then
    echo "removing stale ${pod_name} (postgres is on another pod) …" >&2
    podman pod rm -f "${pod_name}" >/dev/null 2>&1 || true
  fi
}

claw_compose_pg_wait_healthy() {
  if ! claw_compose_uses_local_postgres; then
    return 0
  fi
  local rt cname i
  rt="$(claw_container_runtime_cli)" || return 1
  cname="$(claw_compose_pg_container_name)"
  for i in $(seq 1 60); do
    if "${rt}" inspect -f '{{.State.Health.Status}}' "${cname}" 2>/dev/null | grep -qx healthy; then
      return 0
    fi
    if "${rt}" inspect -f '{{.State.Running}}' "${cname}" 2>/dev/null | grep -qx true; then
      if "${rt}" healthcheck run "${cname}" >/dev/null 2>&1; then
        return 0
      fi
    fi
    sleep 1
  done
  echo "error: ${cname} not healthy after 60s" >&2
  return 1
}

claw_compose_gateway_service_list() {
  local podman_dir="$1"
  local repo_env="$2"
  local pg
  pg="$(claw_compose_pg_service)"
  local svc
  while IFS= read -r svc; do
    [[ -z "${svc}" ]] && continue
    [[ "${svc}" == "${pg}" ]] && continue
    printf '%s ' "${svc}"
  done < <(
    claw_compose_with_root_env "${podman_dir}" "${repo_env}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" config --services 2>/dev/null
  )
}

claw_compose_gateway_down() {
  local podman_dir="$1"
  local repo_env="$2"
  local -a svcs=()
  # shellcheck disable=SC2206
  svcs=($(claw_compose_gateway_service_list "${podman_dir}" "${repo_env}"))
  if [[ ${#svcs[@]} -eq 0 ]]; then
    echo "error: no gateway compose services found (expected besides postgres)" >&2
    return 1
  fi
  claw_compose_with_root_env "${podman_dir}" "${repo_env}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" stop "${svcs[@]}"
  claw_compose_with_root_env "${podman_dir}" "${repo_env}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" rm -f "${svcs[@]}" 2>/dev/null || true
}

claw_compose_gateway_up() {
  local podman_dir="$1"
  local repo_env="$2"
  shift 2
  local -a extra=("$@")
  local -a svcs=()
  local rel=""
  podman_dir="$(cd "${podman_dir}" && pwd)"
  if [[ -f "${repo_env}" ]]; then
    local repo_root
    repo_root="$(cd "$(dirname "${repo_env}")" && pwd)"
    if [[ "${podman_dir}" == "${repo_root}/"* ]]; then
      rel="${podman_dir#"${repo_root}/"}"
    fi
  fi
  # shellcheck disable=SC2206
  svcs=($(claw_compose_gateway_service_list "${podman_dir}" "${repo_env}"))
  if [[ ${#svcs[@]} -eq 0 ]]; then
    echo "error: no gateway compose services found (expected besides postgres)" >&2
    return 1
  fi
  claw_compose_prune_stale_claw_pod
  claw_compose_pg_ensure "${podman_dir}" "${repo_env}"
  claw_compose_pg_wait_healthy
  claw_compose_append_pg_network_override "${podman_dir}" "${rel}"
  claw_compose_with_root_env "${podman_dir}" "${repo_env}" \
    "${CLAW_PODMAN_COMPOSE_ARGS[@]}" up -d --no-deps "${extra[@]}" "${svcs[@]}"
}

claw_compose_pg_up() {
  local podman_dir="$1"
  local repo_env="$2"
  local pg
  pg="$(claw_compose_pg_service)"
  claw_compose_with_root_env "${podman_dir}" "${repo_env}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" up -d "${pg}"
}

# Start existing postgres container or create via compose (avoids name-already-in-use on retry). kejiqing
claw_compose_pg_ensure() {
  if ! claw_compose_uses_local_postgres; then
    echo "Postgres: external (${CLAW_GATEWAY_DATABASE_URL%%@*}@…); skipping compose postgres" >&2
    return 0
  fi
  local podman_dir="$1"
  local repo_env="$2"
  local rt pg cname
  rt="$(claw_container_runtime_cli)" || return 1
  pg="$(claw_compose_pg_service)"
  cname="$(claw_compose_pg_container_name)"
  if "${rt}" container exists "${cname}" >/dev/null 2>&1; then
    if ! "${rt}" inspect -f '{{.State.Running}}' "${cname}" 2>/dev/null | grep -qx true; then
      echo "starting existing ${cname} …" >&2
      "${rt}" start "${cname}" >/dev/null
    fi
    return 0
  fi
  claw_compose_prune_stale_claw_pod
  claw_compose_pg_up "${podman_dir}" "${repo_env}"
}

claw_compose_pg_down() {
  local podman_dir="$1"
  local repo_env="$2"
  local pg
  pg="$(claw_compose_pg_service)"
  claw_compose_with_root_env "${podman_dir}" "${repo_env}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" stop "${pg}" 2>/dev/null || true
}

# Optional `up.sh --release …` image pin (.claw-image-release.env). Author: kejiqing
_claw_podman_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${_claw_podman_dir}/env-profile.sh"
# shellcheck disable=SC1091
source "${_claw_podman_dir}/release-images.sh"
# shellcheck disable=SC1091
source "${_claw_podman_dir}/worker-llm-wiring.sh"
# shellcheck disable=SC1091
source "${_claw_podman_dir}/pool-daemon-binary.sh"
