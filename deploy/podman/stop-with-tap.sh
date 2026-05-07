#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEPLOY_ENV="${SCRIPT_DIR}/.env"
PID_FILE="${SCRIPT_DIR}/claude-tap.pid"

if [[ -f "${DEPLOY_ENV}" ]]; then
  podman compose --env-file "${DEPLOY_ENV}" -f "${SCRIPT_DIR}/podman-compose.yml" down
else
  podman compose -f "${SCRIPT_DIR}/podman-compose.yml" down
fi

if [[ -f "${PID_FILE}" ]]; then
  PID="$(cat "${PID_FILE}")"
  if kill -0 "${PID}" >/dev/null 2>&1; then
    kill "${PID}" || true
  fi
  rm -f "${PID_FILE}"
fi

echo "gateway stopped, claude-tap stopped"
