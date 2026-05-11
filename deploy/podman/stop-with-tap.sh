#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ROOT_ENV="${REPO_ROOT}/.env"
PID_FILE="${SCRIPT_DIR}/claude-tap.pid"

CLAUDE_TAP_MODE="docker"
if [[ -f "${ROOT_ENV}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ROOT_ENV}"
  set +a
  CLAUDE_TAP_MODE="${CLAUDE_TAP_MODE:-docker}"
fi

if [[ -f "${ROOT_ENV}" ]]; then
  # shellcheck disable=SC1090
  source "${SCRIPT_DIR}/compose-include.sh"
  claw_podman_export_pool_workspace "${SCRIPT_DIR}"
  claw_podman_load_compose_args "${SCRIPT_DIR}" "${ROOT_ENV}"
  claw_compose --env-file "${ROOT_ENV}" "${CLAW_PODMAN_COMPOSE_ARGS[@]}" down
  "${SCRIPT_DIR}/pool-daemon-down.sh"
else
  # shellcheck disable=SC1090
  source "${SCRIPT_DIR}/compose-include.sh"
  claw_compose -f "${SCRIPT_DIR}/podman-compose.yml" down
  "${SCRIPT_DIR}/pool-daemon-down.sh"
fi

if [[ "${CLAUDE_TAP_MODE}" == "host" ]] && [[ -f "${PID_FILE}" ]]; then
  PID="$(cat "${PID_FILE}")"
  if kill -0 "${PID}" >/dev/null 2>&1; then
    kill "${PID}" || true
  fi
  rm -f "${PID_FILE}"
fi

echo "gateway stopped, claude-tap stopped"
