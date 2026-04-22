#!/usr/bin/env bash
set -euo pipefail

# Run claw-code image in stdio mode for MCP clients.
# Author: kejiqing

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE_TAG="${IMAGE_TAG:-localhost/claw-code:local}"
CONFIG_PATH="${DORIS_CONFIG:-${ROOT_DIR}/config/doris_clusters.yaml}"

if [[ ! -f "${CONFIG_PATH}" ]]; then
  echo "[error] DORIS_CONFIG file not found: ${CONFIG_PATH}" >&2
  exit 1
fi

podman run --rm -i \
  -e "DORIS_CONFIG=/app/config/doris_clusters.yaml" \
  -v "${CONFIG_PATH}:/app/config/doris_clusters.yaml:ro,Z" \
  "${IMAGE_TAG}"
