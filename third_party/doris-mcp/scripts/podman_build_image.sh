#!/usr/bin/env bash
set -euo pipefail

# Build claw-code image (with Doris MCP capability) for podman.
# Author: kejiqing

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLAW_ROOT="$(cd "${ROOT_DIR}/../.." && pwd)"
IMAGE_TAG="${IMAGE_TAG:-localhost/claw-code:local}"
NPM_REGISTRY="${NPM_REGISTRY:-https://registry.npmmirror.com}"
PIP_INDEX_URL="${PIP_INDEX_URL:-https://pypi.tuna.tsinghua.edu.cn/simple}"

echo "[build] root=${ROOT_DIR}"
echo "[build] context=${CLAW_ROOT}"
echo "[build] image=${IMAGE_TAG}"

podman build \
  -f "${ROOT_DIR}/Containerfile" \
  --build-arg "NPM_REGISTRY=${NPM_REGISTRY}" \
  --build-arg "PIP_INDEX_URL=${PIP_INDEX_URL}" \
  -t "${IMAGE_TAG}" \
  "${CLAW_ROOT}"

echo "[done] built ${IMAGE_TAG}"
