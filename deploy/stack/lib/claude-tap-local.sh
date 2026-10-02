#!/usr/bin/env bash
# Start/stop claude-tap from local fork (docker/podman image or editable venv).
# Upstream hot-reload file: CLAW_TAP_UPSTREAM_CONFIG_FILE or ${repo}/.claw/claw-tap-upstream.json
# (set by compose-include `claw_export_llm_runtime_layout`; gateway writes same path on LLM apply/poll). Author: kejiqing
set -euo pipefail

if [[ -n "${BASH_SOURCE[0]+set}" ]]; then
  _CLAUDE_TAP_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
else
  _CLAUDE_TAP_LIB_DIR="$(cd "$(dirname "$0")" && pwd)"
fi

claw_claude_tap_resolve_context() {
  local root_dir="$1"
  if [[ -n "${CLAUDE_TAP_BUILD_CONTEXT:-}" ]]; then
    printf '%s\n' "${CLAUDE_TAP_BUILD_CONTEXT}"
    return 0
  fi
  printf '%s\n' "${root_dir}/../claude-tap"
}

claw_claude_tap_runtime_cli() {
  # shellcheck source=/dev/null
  source "${_CLAUDE_TAP_LIB_DIR}/compose-include.sh"
  claw_container_runtime_cli
}

claw_claude_tap_stop() {
  local podman_dir="$1"
  local pid_file="${podman_dir}/claude-tap.pid"
  local container_name="${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"

  if [[ -f "${pid_file}" ]]; then
    local pid
    pid="$(cat "${pid_file}")"
    if [[ "${pid}" =~ ^[0-9]+$ ]] && kill -0 "${pid}" >/dev/null 2>&1; then
      kill "${pid}" 2>/dev/null || true
    fi
    rm -f "${pid_file}"
  fi

  if command -v podman >/dev/null 2>&1; then
    podman rm -f "${container_name}" 2>/dev/null || true
  fi
  if command -v docker >/dev/null 2>&1; then
    docker rm -f "${container_name}" 2>/dev/null || true
  fi
  pkill -f 'claude-tap.*--tap-no-launch' 2>/dev/null || true
}

# Always false: host claude-tap sidecar path removed; tap/observe is e2b-only. Author: kejiqing
claw_stack_manages_local_claude_tap() {
  return 1
}

# Local Dockerfile build of claude-tap removed (up must never cargo-compile tap).
# Published CLAUDE_TAP_IMAGE only (ACR/CI pull). Author: kejiqing
claw_claude_tap_upstream_config_path() {
  local root_dir="$1"
  if [[ -n "${CLAW_TAP_UPSTREAM_CONFIG_FILE:-}" ]]; then
    printf '%s\n' "${CLAW_TAP_UPSTREAM_CONFIG_FILE}"
    return 0
  fi
  printf '%s\n' "${root_dir}/.claw/claw-tap-upstream.json"
}

# Shared trace dir for pool tap (NAS when mounted; mergeable across restarts). Author: kejiqing
claw_claude_tap_resolve_traces_dir() {
  local podman_dir="$1"
  if [[ -n "${CLAW_TAP_TRACES_DIR:-}" ]]; then
    printf '%s\n' "${CLAW_TAP_TRACES_DIR}"
    return 0
  fi
  if [[ -n "${CLAW_NAS_HOST_MOUNT:-}" ]]; then
    printf '%s\n' "${CLAW_NAS_HOST_MOUNT}/tap-traces"
    return 0
  fi
  printf '%s\n' "${podman_dir}/claude-tap-data/traces"
}

claw_claude_tap_ensure_upstream_config_file() {
  local root_dir="$1"
  local upstream="$2"
  local cfg
  cfg="$(claw_claude_tap_upstream_config_path "${root_dir}")"
  mkdir -p "$(dirname "${cfg}")"
  if [[ ! -f "${cfg}" ]] && [[ -n "${upstream}" ]]; then
    printf '{"target":"%s"}\n' "${upstream}" >"${cfg}"
  fi
  (cd "$(dirname "${cfg}")" && printf '%s/%s\n' "$(pwd)" "$(basename "${cfg}")")
}

claw_claude_tap_upstream_args() {
  local root_dir="$1"
  local upstream="$2"
  local cfg
  cfg="$(claw_claude_tap_ensure_upstream_config_file "${root_dir}" "${upstream}")"
  printf '%s\n' "${cfg}"
}

# Host-run tap must use published PG port; hash uses scheme/user/dbname only (matches gateway).
# Optional override: CLAW_TAP_DATABASE_URL. Author: kejiqing
claw_claude_tap_host_database_url() {
  if [[ -n "${CLAW_TAP_DATABASE_URL:-}" ]]; then
    printf '%s\n' "${CLAW_TAP_DATABASE_URL}"
    return 0
  fi
  local url="${CLAW_GATEWAY_DATABASE_URL:-postgres://claw_gateway:clawGw9Dev_Pg@postgres:5432/claw_gateway}"
  if [[ -z "${url}" ]]; then
    echo "CLAW_GATEWAY_DATABASE_URL is not set" >&2
    return 1
  fi
  if [[ "${url}" == *"@postgres:"* ]] || [[ "${url}" == *"@postgres/"* ]]; then
    local user="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
    local pass="${CLAW_GATEWAY_PG_PASSWORD:-clawGw9Dev_Pg}"
    local db="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
    local port="${CLAW_GATEWAY_PG_HOST_PORT:-5433}"
    printf 'postgres://%s:%s@127.0.0.1:%s/%s\n' "${user}" "${pass}" "${port}" "${db}"
    return 0
  fi
  printf '%s\n' "${url}"
}

claw_claude_tap_compose_network_name() {
  printf '%s' "${CLAUDE_TAP_DOCKER_NETWORK:-${COMPOSE_PROJECT_NAME:-claw}_default}"
}

# -p args for proxy/live: unset = publish to 0.0.0.0; 0|none = skip; else full spec (e.g. 127.0.0.1:8080:8080).
claw_claude_tap_docker_publish_args() {
  local kind="$1"
  local host_port="$2"
  local container_port="$3"
  local spec=""
  case "${kind}" in
    proxy) spec="${CLAUDE_TAP_PUBLISH_PROXY-}" ;;
    live) spec="${CLAUDE_TAP_PUBLISH_LIVE-}" ;;
    *) echo "unknown publish kind: ${kind}" >&2; return 1 ;;
  esac
  if [[ -z "${spec}" ]]; then
    printf '%s\n' "-p" "${host_port}:${container_port}"
    return 0
  fi
  case "${spec}" in
    0 | none | false | off)
      return 0
      ;;
    *)
      printf '%s\n' "-p" "${spec}"
      ;;
  esac
}

claw_claude_tap_tap_database_url() {
  local for_container="${1:-0}"
  local url
  if [[ -n "${CLAUDE_TAP_DATABASE_URL:-}" ]]; then
    printf '%s\n' "${CLAUDE_TAP_DATABASE_URL}"
    return 0
  fi
  if [[ "${for_container}" == "1" && -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
    url="${CLAW_GATEWAY_DATABASE_URL:-}"
    if [[ "${url}" == *"@postgres:"* ]] || [[ "${url}" == *"@postgres/"* ]]; then
      local user="${CLAW_GATEWAY_PG_USER:-claw_gateway}"
      local pass="${CLAW_GATEWAY_PG_PASSWORD:-clawGw9Dev_Pg}"
      local db="${CLAW_GATEWAY_PG_DATABASE:-claw_gateway}"
      printf 'postgres://%s:%s@postgres:5432/%s\n' "${user}" "${pass}" "${db}"
      return 0
    fi
  fi
  url="$(claw_claude_tap_host_database_url)" || return 1
  if [[ "${for_container}" == "1" ]]; then
    local pg_host="${CLAUDE_TAP_PG_HOST:-host.containers.internal}"
    url="${url//@127.0.0.1:/@${pg_host}:}"
  fi
  printf '%s\n' "${url}"
}

claw_claude_tap_export_cluster_env() {
  local for_container="${1:-0}"
  local tap_db
  tap_db="$(claw_claude_tap_tap_database_url "${for_container}")" || return 1
  if [[ -z "${CLAW_CLUSTER_ID:-}" ]]; then
    echo "CLAW_CLUSTER_ID is required in .env for claude-tap /healthz clusterHash" >&2
    return 1
  fi
  export CLAW_CLUSTER_ID
  export CLAW_GATEWAY_DATABASE_URL="${tap_db}"
}

claw_claude_tap_container_running() {
  local rt="$1"
  local container_name="$2"
  local status
  status="$("${rt}" inspect -f '{{.State.Running}}' "${container_name}" 2>/dev/null || echo false)"
  [[ "${status}" == "true" ]]
}

claw_claude_tap_is_running() {
  local podman_dir="$1"
  local pid_file="${podman_dir}/claude-tap.pid"
  local container_name="${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"

  if [[ -f "${pid_file}" ]]; then
    local pid
    pid="$(cat "${pid_file}")"
    if [[ "${pid}" =~ ^container: ]]; then
      local rt
      rt="$(claw_claude_tap_runtime_cli)"
      if claw_claude_tap_container_running "${rt}" "${container_name}"; then
        return 0
      fi
      rm -f "${pid_file}"
      return 1
    fi
    if [[ "${pid}" =~ ^[0-9]+$ ]] && kill -0 "${pid}" >/dev/null 2>&1; then
      return 0
    fi
    rm -f "${pid_file}"
  fi
  return 1
}

claw_claude_tap_proxy_published_on_host() {
  case "${CLAUDE_TAP_PUBLISH_PROXY-}" in
    0 | none | false | off) return 1 ;;
    *) return 0 ;;
  esac
}

claw_claude_tap_probe_healthz() {
  local port="${1:-${CLAUDE_TAP_PORT:-8080}}"
  if claw_claude_tap_proxy_published_on_host; then
    curl -fsS --connect-timeout 2 "http://127.0.0.1:${port}/healthz" >/dev/null 2>&1
    return $?
  fi
  local rt gw tap_host
  rt="$(claw_claude_tap_runtime_cli)"
  gw="${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"
  tap_host="${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"
  if [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
    if "${rt}" inspect -f '{{.State.Running}}' "${gw}" 2>/dev/null | grep -qx true; then
      "${rt}" exec "${gw}" curl -fsS --connect-timeout 2 "http://${tap_host}:8080/healthz" >/dev/null 2>&1
      return $?
    fi
  fi
  local container_name="${tap_host}"
  if claw_claude_tap_container_running "${rt}" "${container_name}"; then
    "${rt}" exec "${container_name}" curl -fsS --connect-timeout 2 "http://127.0.0.1:8080/healthz" >/dev/null 2>&1
    return $?
  fi
  return 1
}

claw_claude_tap_print_startup_failure() {
  local podman_dir="$1"
  local log_file="${podman_dir}/claude-tap.log"
  local rt container_name logs_blob=""
  rt="$(claw_claude_tap_runtime_cli)"
  container_name="${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"
  if [[ -f "${log_file}" ]]; then
    tail -20 "${log_file}" >&2 || true
    logs_blob="$(tail -50 "${log_file}" 2>/dev/null || true)"
  fi
  if "${rt}" container exists "${container_name}" >/dev/null 2>&1; then
    logs_blob+=$'\n'"$("${rt}" logs "${container_name}" 2>&1 | tail -30 || true)"
  fi
  if grep -q "No active LLM for cluster" <<<"${logs_blob}"; then
    echo "hint: claude-tap requires active LLM in PostgreSQL (cluster=${CLAW_CLUSTER_ID:-unset}); --tap-target / .env upstream are ignored" >&2
    echo "      1) gateway + postgres must be up (./deploy/stack/gateway.sh up)" >&2
    echo "      2) Admin → 全局推理 Apply, or PUT /v1/gateway/global-settings/active-llm-config" >&2
    echo "      3) re-run: ./deploy/stack/gateway.sh tap-up" >&2
  fi
}

claw_claude_tap_wait_healthy() {
  local port="${CLAUDE_TAP_PORT:-8080}"
  local max_attempts="${1:-30}"
  local podman_dir="${2:-}"
  local i
  local where="127.0.0.1:${port}"
  if ! claw_claude_tap_proxy_published_on_host; then
    where="docker exec ${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}:8080"
  fi
  for i in $(seq 1 "${max_attempts}"); do
    if claw_claude_tap_probe_healthz "${port}"; then
      echo "claude-tap /healthz ok (attempt ${i}/${max_attempts}, via ${where})"
      return 0
    fi
    sleep 1
  done
  echo "error: claude-tap not healthy on ${where} after ${max_attempts}s" >&2
  if [[ -n "${podman_dir}" ]]; then
    claw_claude_tap_print_startup_failure "${podman_dir}"
  fi
  return 1
}
