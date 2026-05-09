#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  podman compose --env-file "${ENV_FILE}" -f "${SCRIPT_DIR}/podman-compose.yml" down
else
  podman compose -f "${SCRIPT_DIR}/podman-compose.yml" down
fi

echo "Services stopped."
