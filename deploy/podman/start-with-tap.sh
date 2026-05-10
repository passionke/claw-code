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

if ! command -v claude-tap >/dev/null 2>&1; then
  echo "claude-tap not installed. install with:" >&2
  echo "  uv tool install claude-tap" >&2
  echo "or:" >&2
  echo "  pip install claude-tap" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_ENV}"
set +a

UPSTREAM_OPENAI_BASE_URL="${UPSTREAM_OPENAI_BASE_URL:-${OPENAI_BASE_URL:-}}"
CLAUDE_TAP_PORT="${CLAUDE_TAP_PORT:-8080}"
CLAUDE_TAP_LIVE_PORT="${CLAUDE_TAP_LIVE_PORT:-3000}"

if [[ -z "${UPSTREAM_OPENAI_BASE_URL}" ]]; then
  echo "UPSTREAM_OPENAI_BASE_URL is empty (or OPENAI_BASE_URL missing in root .env)" >&2
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
  echo "claude-tap started pid=$(cat "${PID_FILE}") port=${CLAUDE_TAP_PORT} live=${CLAUDE_TAP_LIVE_PORT}"
fi

# Compose bind-mounts repo-root `.claw.json`. Never overwrite an existing file — only create `{}` if missing. kejiqing
CLAW_JSON="${ROOT_DIR}/.claw.json"
if [[ ! -f "${CLAW_JSON}" ]]; then
  echo "note: ${CLAW_JSON} missing; creating empty {} stub (existing files are never touched)." >&2
  printf '%s\n' '{}' > "${CLAW_JSON}"
fi

podman compose --env-file "${ROOT_ENV}" -f "${SCRIPT_DIR}/podman-compose.yml" up -d
echo "gateway started on port ${GATEWAY_HOST_PORT}"
echo "claude-tap live viewer: http://127.0.0.1:${CLAUDE_TAP_LIVE_PORT}"
