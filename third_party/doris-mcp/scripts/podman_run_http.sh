#!/usr/bin/env bash
set -euo pipefail

# Run claw-code image in HTTP gateway mode.
# Author: kejiqing

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_TAG="${IMAGE_TAG:-localhost/claw-code:local}"
PORT="${PORT:-18080}"
DS_REGISTRY="${CLAW_DS_REGISTRY:-${ROOT_DIR}/http_gateway/config/datasources.example.yaml}"
WORK_ROOT="${CLAW_WORK_ROOT:-${ROOT_DIR}/runs}"

if [[ ! -f "${DS_REGISTRY}" ]]; then
  echo "[error] datasource registry not found: ${DS_REGISTRY}" >&2
  exit 1
fi

mkdir -p "${WORK_ROOT}"

podman run --rm -it \
  -p "${PORT}:18080" \
  -e "CLAW_SERVICE_MODE=http" \
  -e "CLAW_DS_REGISTRY=/app/http_gateway/config/datasources.yaml" \
  -e "CLAW_WORK_ROOT=/var/lib/claw-runs" \
  -e "DORIS_MCP_IMAGE=${IMAGE_TAG}" \
  -v "${DS_REGISTRY}:/app/http_gateway/config/datasources.yaml:ro,Z" \
  -v "${WORK_ROOT}:/var/lib/claw-runs:Z" \
  "${IMAGE_TAG}"
