#!/usr/bin/env bash
set -euo pipefail

# Base image registry (hostname only, no path); same name as GitHub Actions variable
# CONTAINER_BASE_REGISTRY in claw-code-image workflow.
# - Local: default docker.1ms.run unless overridden in env or repo-root .env
# - docker.io when GITHUB_ACTIONS=true (GitHub CI) or CLAW_USE_DOCKER_IO=1
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi

IMAGE_TAG="${1:-local}"
IMAGE_NAME="claw-gateway-rs:${IMAGE_TAG}"

if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  REG="docker.io"
  echo "Using docker.io base images (CI or CLAW_USE_DOCKER_IO=1)"
else
  REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  REG="${REG%/}"
  echo "Using ${REG} for base images (set CONTAINER_BASE_REGISTRY or CLAW_USE_DOCKER_IO=1 for docker.io)"
fi
RUST_BASE_IMAGE="${REG}/library/rust:1.88-bookworm"
DEBIAN_BASE_IMAGE="${REG}/library/debian:bookworm-slim"

podman build \
  --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
  -f "${ROOT_DIR}/deploy/podman/Containerfile.gateway-rs" \
  -t "${IMAGE_NAME}" \
  "${ROOT_DIR}"

echo "Built image: ${IMAGE_NAME}"
