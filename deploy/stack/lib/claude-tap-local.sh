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

# True when this host runs claude-tap (production docker tap or local sidecar). Author: kejiqing
claw_stack_manages_local_claude_tap() {
  case "${CLAW_LLM_PROXY:-direct}" in
    remote) return 1 ;;
  esac
  case "${CLAUDE_TAP_MODE:-docker}" in
    off | none | disabled | false | '0') return 1 ;;
    *) return 0 ;;
  esac
}

claw_claude_tap_build_image() {
  local rt="$1"
  local ctx="$2"
  local image="$3"
  local platform="${CLAUDE_TAP_PLATFORM:-}"

  [[ -d "${ctx}" ]] || {
    echo "CLAUDE_TAP_BUILD_CONTEXT not found: ${ctx}" >&2
    exit 1
  }
  [[ -f "${ctx}/Dockerfile" ]] || {
    echo "missing Dockerfile in ${ctx}" >&2
    exit 1
  }

  local -a build_args=()
  if [[ -n "${platform}" ]]; then
    build_args+=(--platform "${platform}")
  fi
  echo "==> building ${image} from ${ctx} (${rt})" >&2
  if ((${#build_args[@]})); then
    "${rt}" build "${build_args[@]}" -f "${ctx}/Dockerfile" -t "${image}" "${ctx}"
  else
    "${rt}" build -f "${ctx}/Dockerfile" -t "${image}" "${ctx}"
  fi
}

# Local fork build, or `docker pull` when only CLAUDE_TAP_IMAGE is set (production). Author: kejiqing
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

claw_claude_tap_ensure_image() {
  local rt="$1"
  local ctx="$2"
  local image="$3"

  if [[ "${CLAUDE_TAP_REBUILD:-0}" == "1" ]] && [[ -d "${ctx}" && -f "${ctx}/Dockerfile" ]]; then
    claw_claude_tap_build_image "${rt}" "${ctx}" "${image}"
    return 0
  fi
  if "${rt}" image exists "${image}" >/dev/null 2>&1; then
    return 0
  fi
  if [[ -d "${ctx}" && -f "${ctx}/Dockerfile" ]]; then
    claw_claude_tap_build_image "${rt}" "${ctx}" "${image}"
    return 0
  fi
  echo "==> pull ${image} (${rt})" >&2
  "${rt}" pull "${image}"
}

claw_claude_tap_start_docker() {
  local rt="$1"
  local podman_dir="$2"
  local ctx="$3"
  local root_dir="$4"
  local image="${CLAUDE_TAP_IMAGE:-claude-tap:local}"
  local container_name="${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"
  local port="${CLAUDE_TAP_PORT:-8080}"
  local live_port="${CLAUDE_TAP_LIVE_PORT:-3000}"
  local traces_dir tap_target log_file upstream_cfg
  traces_dir="$(claw_claude_tap_resolve_traces_dir "${podman_dir}")"
  tap_target="$5"
  log_file="${podman_dir}/claude-tap.log"
  upstream_cfg="$(claw_claude_tap_upstream_args "${root_dir}" "${tap_target}")"

  mkdir -p "${traces_dir}"
  traces_dir="$(cd "${traces_dir}" && pwd)"

  if [[ "${CLAUDE_TAP_REBUILD:-0}" == "1" ]] && [[ ! -d "${ctx}" || ! -f "${ctx}/Dockerfile" ]]; then
    echo "==> pull ${image} (CLAUDE_TAP_REBUILD=1, no local Dockerfile)" >&2
    "${rt}" pull "${image}"
  else
    claw_claude_tap_ensure_image "${rt}" "${ctx}" "${image}"
  fi

  "${rt}" rm -f "${container_name}" 2>/dev/null || true

  claw_claude_tap_export_cluster_env 1

  local -a run_args=(run -d --name "${container_name}")
  if [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
    local net
    net="$(claw_claude_tap_compose_network_name)"
    "${rt}" network inspect "${net}" >/dev/null 2>&1 || {
      echo "error: docker network ${net} missing; run ./deploy/stack/gateway.sh up first (compose creates it)" >&2
      exit 1
    }
    run_args+=(--network "${net}")
  fi
  # macOS /bin/bash 3.2 has no mapfile; read publish -p lines into arrays. kejiqing
  local proxy_publish=() live_publish=() line
  while IFS= read -r line; do
    [[ -n "${line}" ]] && proxy_publish+=("${line}")
  done < <(claw_claude_tap_docker_publish_args proxy "${port}" 8080)
  while IFS= read -r line; do
    [[ -n "${line}" ]] && live_publish+=("${line}")
  done < <(claw_claude_tap_docker_publish_args live "${live_port}" 3000)
  if ((${#proxy_publish[@]})); then
    run_args+=("${proxy_publish[@]}")
  fi
  if ((${#live_publish[@]})); then
    run_args+=("${live_publish[@]}")
  fi

  # shellcheck disable=SC2086
  "${rt}" "${run_args[@]}" \
    -e "CLAW_CLUSTER_ID=${CLAW_CLUSTER_ID}" \
    -e "CLAW_GATEWAY_DATABASE_URL=${CLAW_GATEWAY_DATABASE_URL}" \
    -v "${traces_dir}:/data/traces" \
    -v "${upstream_cfg}:${upstream_cfg}:ro" \
    "${image}" \
    claude-tap \
    --tap-no-launch \
    --tap-host 0.0.0.0 \
    --tap-port 8080 \
    --tap-live \
    --tap-live-port 3000 \
    --tap-target "${tap_target}" \
    --tap-upstream-config "${upstream_cfg}" \
    --tap-output-dir /data/traces \
    --tap-no-update-check \
    --tap-no-auto-update \
    >"${log_file}" 2>&1

  local cid
  cid="$("${rt}" ps -q --filter "name=^${container_name}$")"
  [[ -n "${cid}" ]] || {
    echo "claude-tap container failed to start; see ${log_file}" >&2
    "${rt}" logs "${container_name}" 2>&1 | tail -30 >&2 || true
    exit 1
  }
  echo "container:${container_name}" >"${podman_dir}/claude-tap.pid"
  local net_hint=""
  if [[ -n "${CLAUDE_TAP_DOCKER_NETWORK:-}" ]]; then
    net_hint=" network=$(claw_claude_tap_compose_network_name) adminHost=${container_name}"
  fi
  echo "claude-tap container ${container_name} (${image}) port=${port} live=${live_port}${net_hint} traces=${traces_dir}"
}

claw_claude_tap_start_source() {
  local podman_dir="$1"
  local ctx="$2"
  local root_dir="$3"
  local port="${CLAUDE_TAP_PORT:-8080}"
  local live_port="${CLAUDE_TAP_LIVE_PORT:-3000}"
  local tap_target="$4"
  local log_file="${podman_dir}/claude-tap.log"
  local upstream_cfg
  upstream_cfg="$(claw_claude_tap_upstream_args "${root_dir}" "${tap_target}")"
  local traces_dir bin
  traces_dir="$(claw_claude_tap_resolve_traces_dir "${podman_dir}")"
  bin="${CLAUDE_TAP_SOURCE_BIN:-}"

  [[ -d "${ctx}" ]] || {
    echo "CLAUDE_TAP_BUILD_CONTEXT not found: ${ctx}" >&2
    exit 1
  }

  if [[ -z "${bin}" ]]; then
    if [[ -x "${ctx}/.venv/bin/claude-tap" ]]; then
      bin="${ctx}/.venv/bin/claude-tap"
    elif command -v uv >/dev/null 2>&1; then
      echo "==> syncing claude-tap venv in ${ctx} (uv sync)" >&2
      (cd "${ctx}" && uv sync)
      bin="${ctx}/.venv/bin/claude-tap"
    else
      echo "no ${ctx}/.venv/bin/claude-tap; install with: cd ${ctx} && uv sync" >&2
      exit 1
    fi
  fi
  [[ -x "${bin}" ]] || {
    echo "claude-tap binary not executable: ${bin}" >&2
    exit 1
  }

  mkdir -p "${traces_dir}"
  claw_claude_tap_export_cluster_env 0
  nohup env CLAW_CLUSTER_ID="${CLAW_CLUSTER_ID}" CLAW_GATEWAY_DATABASE_URL="${CLAW_GATEWAY_DATABASE_URL}" \
    "${bin}" \
    --tap-no-launch \
    --tap-live \
    --tap-host 0.0.0.0 \
    --tap-port "${port}" \
    --tap-live-port "${live_port}" \
    --tap-target "${tap_target}" \
    --tap-upstream-config "${upstream_cfg}" \
    --tap-output-dir "${traces_dir}" \
    --tap-no-update-check \
    --tap-no-auto-update \
    >"${log_file}" 2>&1 &
  echo $! >"${podman_dir}/claude-tap.pid"
  sleep 1
  if ! kill -0 "$(cat "${podman_dir}/claude-tap.pid")" >/dev/null 2>&1; then
    echo "failed to start source claude-tap, check ${log_file}" >&2
    exit 1
  fi
  echo "claude-tap source ${bin} pid=$(cat "${podman_dir}/claude-tap.pid") port=${port} live=${live_port}"
}

# PyPI distribution is claw-tap; CLI remains claude-tap (https://pypi.org/project/claw-tap/). Author: kejiqing
claw_claude_tap_pypi_version() {
  printf '%s\n' "${CLAW_TAP_PYPI_VERSION:-${CLAUDE_TAP_PYPI_VERSION:-0.0.7}}"
}

claw_claude_tap_ensure_pypi_bin() {
  local version="$1"
  local bin="${CLAUDE_TAP_NATIVE_BIN:-}"

  if [[ -n "${bin}" ]]; then
    [[ -x "${bin}" ]] || {
      echo "CLAUDE_TAP_NATIVE_BIN not executable: ${bin}" >&2
      exit 1
    }
    printf '%s\n' "${bin}"
    return 0
  fi

  if command -v claude-tap >/dev/null 2>&1; then
    command -v claude-tap
    return 0
  fi

  if ! command -v uv >/dev/null 2>&1; then
    echo "claude-tap not in PATH; install uv then re-run tap-up, or:" >&2
    echo "  uv tool install claw-tap==${version}" >&2
    echo "  pip install claw-tap==${version}" >&2
    exit 1
  fi

  echo "==> uv tool install claw-tap==${version} (PyPI; GHCR image has no linux/arm64)" >&2
  # Optional: UV_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple for CN networks
  uv tool install "claw-tap==${version}" --force
  command -v claude-tap
}

claw_claude_tap_start_native() {
  local podman_dir="$1"
  local root_dir="$2"
  local port="${CLAUDE_TAP_PORT:-8080}"
  local live_port="${CLAUDE_TAP_LIVE_PORT:-3000}"
  local tap_target="$3"
  local log_file="${podman_dir}/claude-tap.log"
  local upstream_cfg traces_dir bin pypi_ver
  upstream_cfg="$(claw_claude_tap_upstream_args "${root_dir}" "${tap_target}")"
  traces_dir="$(claw_claude_tap_resolve_traces_dir "${podman_dir}")"
  pypi_ver="$(claw_claude_tap_pypi_version)"
  bin="$(claw_claude_tap_ensure_pypi_bin "${pypi_ver}")"
  mkdir -p "${traces_dir}"
  claw_claude_tap_export_cluster_env 0

  nohup env CLAW_CLUSTER_ID="${CLAW_CLUSTER_ID}" CLAW_GATEWAY_DATABASE_URL="${CLAW_GATEWAY_DATABASE_URL}" \
    "${bin}" \
    --tap-no-launch \
    --tap-live \
    --tap-host 0.0.0.0 \
    --tap-port "${port}" \
    --tap-live-port "${live_port}" \
    --tap-target "${tap_target}" \
    --tap-upstream-config "${upstream_cfg}" \
    --tap-output-dir "${traces_dir}" \
    --tap-no-update-check \
    --tap-no-auto-update \
    >"${log_file}" 2>&1 &
  echo $! >"${podman_dir}/claude-tap.pid"
  sleep 1
  if ! kill -0 "$(cat "${podman_dir}/claude-tap.pid")" >/dev/null 2>&1; then
    echo "failed to start claude-tap, check ${log_file}" >&2
    exit 1
  fi
  echo "claude-tap pypi claw-tap==${pypi_ver} ${bin} pid=$(cat "${podman_dir}/claude-tap.pid") port=${port} live=${live_port}"
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

claw_claude_tap_start() {
  local podman_dir="$1"
  local root_dir="$2"
  local mode="${CLAUDE_TAP_MODE:-docker}"
  local ctx
  ctx="$(claw_claude_tap_resolve_context "${root_dir}")"
  local upstream="${UPSTREAM_OPENAI_BASE_URL:-${OPENAI_BASE_URL:-}}"
  if [[ -z "${upstream}" ]]; then
    local cfg
    cfg="$(claw_claude_tap_upstream_config_path "${root_dir}")"
    if [[ -f "${cfg}" ]]; then
      upstream="$(python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); print((d.get("target") or "").strip())' "${cfg}" 2>/dev/null || true)"
    fi
  fi
  if [[ -z "${upstream}" ]]; then
    upstream="https://bootstrap.invalid/v1"
    echo "note: no UPSTREAM in .env or claw-tap-upstream.json; bootstrap placeholder until Admin LLM apply (PG hot-reloads tap)" >&2
  fi

  if claw_claude_tap_is_running "${podman_dir}"; then
    echo "claude-tap already running ($(cat "${podman_dir}/claude-tap.pid")) mode=${mode}"
    return 0
  fi

  case "${mode}" in
    docker | podman)
      local rt
      rt="$(claw_claude_tap_runtime_cli)"
      claw_claude_tap_start_docker "${rt}" "${podman_dir}" "${ctx}" "${root_dir}" "${upstream}"
      ;;
    source | editable | local)
      claw_claude_tap_start_source "${podman_dir}" "${ctx}" "${root_dir}" "${upstream}"
      ;;
    native | pypi)
      claw_claude_tap_start_native "${podman_dir}" "${root_dir}" "${upstream}"
      ;;
    *)
      echo "unknown CLAUDE_TAP_MODE=${mode} (use docker, source, native/pypi)" >&2
      exit 1
      ;;
  esac
  claw_claude_tap_wait_healthy 30 "${podman_dir}" || exit 1
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
