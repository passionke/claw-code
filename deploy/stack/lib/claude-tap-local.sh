#!/usr/bin/env bash
# Start/stop claude-tap from local fork (docker/podman image or editable venv). Author: kejiqing
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

claw_claude_tap_start_docker() {
  local rt="$1"
  local podman_dir="$2"
  local ctx="$3"
  local image="${CLAUDE_TAP_IMAGE:-claude-tap:local}"
  local container_name="${CLAUDE_TAP_CONTAINER_NAME:-claw-claude-tap}"
  local port="${CLAUDE_TAP_PORT:-8080}"
  local live_port="${CLAUDE_TAP_LIVE_PORT:-3000}"
  local traces_dir="${CLAUDE_TAP_TRACES_DIR:-${podman_dir}/claude-tap-data/traces}"
  local tap_target="$4"
  local log_file="${podman_dir}/claude-tap.log"

  mkdir -p "${traces_dir}"
  traces_dir="$(cd "${traces_dir}" && pwd)"

  if [[ "${CLAUDE_TAP_REBUILD:-0}" == "1" ]] || ! "${rt}" image exists "${image}" >/dev/null 2>&1; then
    claw_claude_tap_build_image "${rt}" "${ctx}" "${image}"
  fi

  "${rt}" rm -f "${container_name}" 2>/dev/null || true

  # shellcheck disable=SC2086
  "${rt}" run -d --name "${container_name}" \
    -p "${port}:8080" \
    -p "${live_port}:3000" \
    -v "${traces_dir}:/data/traces" \
    "${image}" \
    claude-tap \
    --tap-no-launch \
    --tap-host 0.0.0.0 \
    --tap-port 8080 \
    --tap-live \
    --tap-live-port 3000 \
    --tap-target "${tap_target}" \
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
  local port="${CLAUDE_TAP_PORT:-8080}"
  local live_port="${CLAUDE_TAP_LIVE_PORT:-3000}"
  local tap_target="$3"
  local log_file="${podman_dir}/claude-tap.log"
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
  nohup "${bin}" \
    --tap-no-launch \
    --tap-live \
    --tap-host 0.0.0.0 \
    --tap-port "${port}" \
    --tap-live-port "${live_port}" \
    --tap-target "${tap_target}" \
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

claw_claude_tap_start_native() {
  local podman_dir="$1"
  local port="${CLAUDE_TAP_PORT:-8080}"
  local live_port="${CLAUDE_TAP_LIVE_PORT:-3000}"
  local tap_target="$2"
  local log_file="${podman_dir}/claude-tap.log"

  if ! command -v claude-tap >/dev/null 2>&1; then
    echo "claude-tap not in PATH (CLAUDE_TAP_MODE=native). Use CLAUDE_TAP_MODE=docker|source or:" >&2
    echo "  uv tool install claude-tap" >&2
    exit 1
  fi

  nohup claude-tap \
    --tap-no-launch \
    --tap-live \
    --tap-port "${port}" \
    --tap-live-port "${live_port}" \
    --tap-target "${tap_target}" \
    >"${log_file}" 2>&1 &
  echo $! >"${podman_dir}/claude-tap.pid"
  sleep 1
  if ! kill -0 "$(cat "${podman_dir}/claude-tap.pid")" >/dev/null 2>&1; then
    echo "failed to start claude-tap, check ${log_file}" >&2
    exit 1
  fi
  echo "claude-tap native pid=$(cat "${podman_dir}/claude-tap.pid") port=${port} live=${live_port}"
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
      "${rt}" container exists "${container_name}" 2>/dev/null && return 0
      return 1
    fi
    if [[ "${pid}" =~ ^[0-9]+$ ]] && kill -0 "${pid}" >/dev/null 2>&1; then
      return 0
    fi
  fi
  return 1
}

claw_claude_tap_start() {
  local podman_dir="$1"
  local root_dir="$2"
  local mode="${CLAUDE_TAP_MODE:-native}"
  local ctx
  ctx="$(claw_claude_tap_resolve_context "${root_dir}")"
  local upstream="${UPSTREAM_OPENAI_BASE_URL:-${OPENAI_BASE_URL:-}}"

  if [[ -z "${upstream}" ]]; then
    echo "UPSTREAM_OPENAI_BASE_URL is empty (set real LLM URL for --tap-target)" >&2
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
      claw_claude_tap_start_docker "${rt}" "${podman_dir}" "${ctx}" "${upstream}"
      ;;
    source | editable | local)
      claw_claude_tap_start_source "${podman_dir}" "${ctx}" "${upstream}"
      ;;
    native | pypi)
      claw_claude_tap_start_native "${podman_dir}" "${upstream}"
      ;;
    *)
      echo "unknown CLAUDE_TAP_MODE=${mode} (use docker, source, or native)" >&2
      exit 1
      ;;
  esac
}
