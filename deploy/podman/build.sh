#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE_TAG="${1:-local}"
IMAGE_NAME="claw-gateway-rs:${IMAGE_TAG}"

podman build \
  -f "${ROOT_DIR}/deploy/podman/Containerfile.gateway-rs" \
  -t "${IMAGE_NAME}" \
  "${ROOT_DIR}"

echo "Built image: ${IMAGE_NAME}"
