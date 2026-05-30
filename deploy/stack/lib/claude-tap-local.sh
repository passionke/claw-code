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
  "${rt}" build "${build_args[@]}" -f "${ctx}/Dockerfile" -t "${image}" "${ctx}"
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
  local url="${CLAW_GATEWAY_DATABASE_URL:-}"
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

claw_claude_tap_tap_database_url() {
  local for_container="${1:-0}"
  local url
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
  local traces_dir="${CLAUDE_TAP_TRACES_DIR:-${podman_dir}/claude-tap-data/traces}"
  local tap_target="$5"
  local log_file="${podman_dir}/claude-tap.log"
  local upstream_cfg
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

  # shellcheck disable=SC2086
  "${rt}" run -d --name "${container_name}" \
    -e "CLAW_CLUSTER_ID=${CLAW_CLUSTER_ID}" \
    -e "CLAW_GATEWAY_DATABASE_URL=${CLAW_GATEWAY_DATABASE_URL}" \
    -p "${port}:8080" \
    -p "${live_port}:3000" \
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
  echo "claude-tap container ${container_name} (${image}) port=${port} live=${live_port} traces=${traces_dir}"
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
  local traces_dir="${CLAUDE_TAP_TRACES_DIR:-${podman_dir}/claude-tap-data/traces}"
  local bin="${CLAUDE_TAP_SOURCE_BIN:-}"

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
  traces_dir="${CLAUDE_TAP_TRACES_DIR:-${podman_dir}/claude-tap-data/traces}"
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
    echo "UPSTREAM_OPENAI_BASE_URL is empty and ${root_dir}/.claw/claw-tap-upstream.json has no target" >&2
    echo "hint: configure active LLM in Admin (PG); claude-tap polls PG for upstream (see docs/claw-tap-integration-requirements.md)" >&2
    exit 1
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
}
