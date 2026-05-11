#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ROOT_ENV="${ROOT_DIR}/.env"
PID_FILE="${SCRIPT_DIR}/claude-tap.pid"
LOG_FILE="${SCRIPT_DIR}/claude-tap.log"

if [[ ! -f "${ROOT_ENV}" ]]; then
  echo "missing ${ROOT_ENV}" >&2
  echo "copy ${ROOT_DIR}/.env.example to ${ROOT_ENV} and edit" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_ENV}"
set +a

CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-docker}"
CLAUDE_TAP_IMAGE="${CLAUDE_TAP_IMAGE:-ghcr.io/passionke/claude-tap:latest}"
UPSTREAM_OPENAI_BASE_URL="${UPSTREAM_OPENAI_BASE_URL:-${OPENAI_BASE_URL:-}}"
CLAUDE_TAP_HOST_PORT="${CLAUDE_TAP_HOST_PORT:-8080}"
CLAUDE_TAP_PORT="${CLAUDE_TAP_PORT:-${CLAUDE_TAP_HOST_PORT}}"
CLAUDE_TAP_LIVE_PORT="${CLAUDE_TAP_LIVE_PORT:-3000}"

if [[ -z "${UPSTREAM_OPENAI_BASE_URL}" ]]; then
  echo "UPSTREAM_OPENAI_BASE_URL is empty (set it in root .env; used as claude-tap --tap-target)" >&2
  exit 1
fi

if [[ "${CLAUDE_TAP_MODE}" == "host" ]]; then
  # Non-login shells (SSH, cron) often omit pyenv / pip --user paths. Author: kejiqing
  export PATH="${HOME}/.local/bin:${HOME}/.pyenv/shims:${HOME}/.pyenv/bin:${PATH}"
  for _tap in "${HOME}/.pyenv/versions"/*/bin/claude-tap; do
    if [[ -x "${_tap}" ]]; then
      export PATH="$(cd "$(dirname "${_tap}")" && pwd):${PATH}"
      break
    fi
  done
  if ! command -v claude-tap >/dev/null 2>&1; then
    echo "claude-tap not installed. install with:" >&2
    echo "  uv tool install claude-tap" >&2
    echo "or set CLAUDE_TAP_MODE=docker (default) to use ${CLAUDE_TAP_IMAGE}" >&2
    exit 1
  fi
  if [[ -f "${PID_FILE}" ]] && kill -0 "$(cat "${PID_FILE}")" >/dev/null 2>&1; then
    echo "claude-tap already running pid=$(cat "${PID_FILE}")"
  else
    nohup claude-tap \
      --tap-no-launch \
      --tap-live \
      --tap-port "${CLAUDE_TAP_PORT}" \
      --tap-live-port "${CLAUDE_TAP_LIVE_PORT}" \
      --tap-target "${UPSTREAM_OPENAI_BASE_URL}" \
      >"${LOG_FILE}" 2>&1 &
    echo $! >"${PID_FILE}"
    sleep 1
    if ! kill -0 "$(cat "${PID_FILE}")" >/dev/null 2>&1; then
      echo "failed to start claude-tap, check ${LOG_FILE}" >&2
      exit 1
    fi
    echo "claude-tap (host) started pid=$(cat "${PID_FILE}") port=${CLAUDE_TAP_PORT} live=${CLAUDE_TAP_LIVE_PORT}"
  fi
else
  echo "claude-tap: container ${CLAUDE_TAP_IMAGE} (compose; CLAUDE_TAP_MODE=${CLAUDE_TAP_MODE})"
fi

# Compose bind-mounts repo-root `.claw.json`. Never overwrite an existing file — only create `{}` if missing. kejiqing
CLAW_JSON="${ROOT_DIR}/.claw.json"
if [[ ! -f "${CLAW_JSON}" ]]; then
  echo "note: ${CLAW_JSON} missing; creating empty {} stub (existing files are never touched)." >&2
  printf '%s\n' '{}' > "${CLAW_JSON}"
fi

# shellcheck disable=SC1090
source "${SCRIPT_DIR}/compose-include.sh"
claw_podman_export_pool_workspace "${SCRIPT_DIR}"
claw_podman_load_compose_args "${SCRIPT_DIR}" "${ROOT_ENV}"

if [[ "${CLAW_SOLVE_ISOLATION:-podman_pool}" != "inprocess" ]] && [[ "${CLAW_POOL_HOST_DAEMON:-1}" == "1" ]]; then
  "${SCRIPT_DIR}/pool-daemon-up.sh" "${SCRIPT_DIR}" "${ROOT_DIR}"
fi

claw_compose --env-file "${ROOT_ENV}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" up -d --force-recreate
echo "gateway started on port ${GATEWAY_HOST_PORT}"
echo "claude-tap live viewer: http://127.0.0.1:${CLAUDE_TAP_LIVE_PORT}"
