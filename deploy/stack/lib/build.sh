#!/usr/bin/env bash
set -euo pipefail

# Build gateway (http-gateway-rs + claw-pool-daemon) and worker (claw) images in one run.
# Same rust/ tree, same base images and rustup build-args — pair after any Rust change. Author: kejiqing
#
# Base image registry (hostname only, no path); same name as GitHub Actions variable
# CONTAINER_BASE_REGISTRY in claw-code-image workflow.
# - Local: default docker.1ms.run unless overridden in env or repo-root .env
# - docker.io when GITHUB_ACTIONS=true (GitHub CI) or CLAW_USE_DOCKER_IO=1
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/compose-include.sh"

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

# rustup: official by default; optional USTC when CLAW_USE_CN_RUST_MIRROR=1 (DNS must resolve mirror hosts).
RUSTUP_BUILD_ARGS=()
if [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]] && [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
  RUSTUP_BUILD_ARGS=(
    --build-arg "RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static"
    --build-arg "RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup"
  )
  echo "rustup: USTC mirror (CLAW_USE_CN_RUST_MIRROR=1)"
else
  echo "rustup: static.rust-lang.org (set CLAW_USE_CN_RUST_MIRROR=1 in .env for USTC on container build)"
fi

CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1
echo "container CLI: ${CONTAINER_CLI} (override with CLAW_CONTAINER_RUNTIME=podman|docker)"

# Bash 3.2 + `set -u`: expanding an empty array with "${arr[@]}" errors; allow empty expansion here. kejiqing
set +u
"${CONTAINER_CLI}" build \
  --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
  "${RUSTUP_BUILD_ARGS[@]}" \
  -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-rs" \
  -t "${IMAGE_NAME}" \
  "${ROOT_DIR}"
set -u

echo "Built image: ${IMAGE_NAME}"

WORKER_IMAGE_NAME="claw-gateway-worker:${IMAGE_TAG}"
echo "Building worker image: ${WORKER_IMAGE_NAME} …"
set +u
"${CONTAINER_CLI}" build \
  --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
  "${RUSTUP_BUILD_ARGS[@]}" \
  -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker" \
  -t "${WORKER_IMAGE_NAME}" \
  "${ROOT_DIR}"
set -u

echo "Built image: ${WORKER_IMAGE_NAME}"

# macOS dev: host `claw-pool-daemon` is the real pool (Linux image cannot replace Mach-O); keep it in
# sync with `rust/` whenever you run `gateway.sh build`, so `up`/`restart` does not resurrect stale names/logic.
if [[ "$(uname -s)" == "Darwin" ]] && command -v cargo >/dev/null 2>&1; then
  echo "Building host claw-pool-daemon (Darwin + cargo in PATH) …"
  (cd "${ROOT_DIR}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)
  echo "Host binary: ${ROOT_DIR}/rust/target/release/claw-pool-daemon"
fi
