#!/usr/bin/env bash
set -euo pipefail

# Base image registry:
# - Local default: docker.1ms.run (China network friendly; kejiqing)
# - docker.io when GITHUB_ACTIONS=true (GitHub CI) or CLAW_USE_DOCKER_IO=1
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE_TAG="${1:-local}"
IMAGE_NAME="claw-gateway-rs:${IMAGE_TAG}"

if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  RUST_BASE_IMAGE="docker.io/library/rust:1.88-bookworm"
  DEBIAN_BASE_IMAGE="docker.io/library/debian:bookworm-slim"
  echo "Using docker.io base images (CI or CLAW_USE_DOCKER_IO=1)"
else
  RUST_BASE_IMAGE="docker.1ms.run/library/rust:1.88-bookworm"
  DEBIAN_BASE_IMAGE="docker.1ms.run/library/debian:bookworm-slim"
  echo "Using docker.1ms.run base images (local default; set CLAW_USE_DOCKER_IO=1 for docker.io)"
fi

podman build \
  --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
  -f "${ROOT_DIR}/deploy/podman/Containerfile.gateway-rs" \
  -t "${IMAGE_NAME}" \
  "${ROOT_DIR}"

echo "Built image: ${IMAGE_NAME}"
