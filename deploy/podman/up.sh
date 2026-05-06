#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${SCRIPT_DIR}/.env"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo ".env not found. Copy .env.example to .env and edit values first." >&2
  exit 1
fi

podman compose --env-file "${ENV_FILE}" -f "${SCRIPT_DIR}/podman-compose.yml" up -d
echo "Services started."
