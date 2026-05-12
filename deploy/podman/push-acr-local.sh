#!/usr/bin/env bash
# Build linux/amd64 gateway + worker images locally and push to any Docker registry
# (Aliyun ACR personal, Azure ACR, Harbor, …). No GitHub Actions required. kejiqing
#
# Prereq: log in once (hostname only, no path):
#   podman login crpi-xxxx.cn-hangzhou.personal.cr.aliyuncs.com
#   # or: docker login …
#
# Usage:
#   ./deploy/podman/push-acr-local.sh <registry-prefix> <tag>
# Example:
#   ./deploy/podman/push-acr-local.sh crpi-xxxx.cn-hangzhou.personal.cr.aliyuncs.com/passionke release-v1.0.27
#
# Env overrides (optional):
#   CLAW_PUSH_PLATFORM=linux/amd64   # default; use linux/arm64 on ARM servers, etc.
#   CLAW_CONTAINER_RUNTIME=podman|docker
#   CLAW_USE_DOCKER_IO=1             # base images from docker.io (same as build.sh)
#   CLAW_USE_CN_RUST_MIRROR=1      # USTC rustup mirror during build (see build.sh)
#   CLAW_LOCAL_PUSH_DRY_RUN=1       # build + tag only, no push
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  ./deploy/podman/push-acr-local.sh <registry-prefix> <tag>

<registry-prefix>  Full image prefix without tag, no https://. Examples:
  crpi-xxxx.cn-hangzhou.personal.cr.aliyuncs.com/your-namespace
  myregistry.azurecr.io

<tag>                e.g. release-v1.0.27 (pushed as prefix/claw-code:tag and …/claw-gateway-worker:tag)

Optional env: CLAW_PUSH_PLATFORM (default linux/amd64), CLAW_LOCAL_PUSH_DRY_RUN=1
EOF
}

PREFIX_RAW="${1:-${CLAW_LOCAL_PUSH_PREFIX:-}}"
TAG="${2:-${CLAW_LOCAL_PUSH_TAG:-}}"
if [[ -z "${PREFIX_RAW}" ]] || [[ -z "${TAG}" ]]; then
  usage >&2
  exit 2
fi
REGISTRY_PREFIX="${PREFIX_RAW%/}"

if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/podman/compose-include.sh"

CLI="$(claw_container_runtime_cli)" || exit 1
PLATFORM="${CLAW_PUSH_PLATFORM:-linux/amd64}"
BUILD_PLATFORM=(--platform "${PLATFORM}")

if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  REG="docker.io"
else
  REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  REG="${REG%/}"
fi
RUST_BASE_IMAGE="${REG}/library/rust:1.88-bookworm"
DEBIAN_BASE_IMAGE="${REG}/library/debian:bookworm-slim"

RUSTUP_BUILD_ARGS=()
if [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]] && [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
  RUSTUP_BUILD_ARGS=(
    --build-arg "RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static"
    --build-arg "RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup"
  )
fi

GATEWAY_REMOTE="${REGISTRY_PREFIX}/claw-code:${TAG}"
WORKER_REMOTE="${REGISTRY_PREFIX}/claw-gateway-worker:${TAG}"

echo "CLI=${CLI} PLATFORM=${PLATFORM}"
echo "push gateway  -> ${GATEWAY_REMOTE}"
echo "push worker   -> ${WORKER_REMOTE}"
if [[ "${CLAW_LOCAL_PUSH_DRY_RUN:-0}" == "1" ]]; then
  echo "(CLAW_LOCAL_PUSH_DRY_RUN=1: will not push)"
fi

set +u
"${CLI}" build \
  "${BUILD_PLATFORM[@]}" \
  --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
  "${RUSTUP_BUILD_ARGS[@]}" \
  -f "${ROOT_DIR}/deploy/podman/Containerfile.gateway-rs" \
  -t "${GATEWAY_REMOTE}" \
  "${ROOT_DIR}"
"${CLI}" build \
  "${BUILD_PLATFORM[@]}" \
  --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
  "${RUSTUP_BUILD_ARGS[@]}" \
  -f "${ROOT_DIR}/deploy/podman/Containerfile.gateway-worker" \
  -t "${WORKER_REMOTE}" \
  "${ROOT_DIR}"
set -u

if [[ "${CLAW_LOCAL_PUSH_DRY_RUN:-0}" == "1" ]]; then
  echo "dry-run done (images built and tagged locally)."
  exit 0
fi

"${CLI}" push "${GATEWAY_REMOTE}"
"${CLI}" push "${WORKER_REMOTE}"
echo "Pushed OK. On server set CLAW_IMAGE_PREFIX=${REGISTRY_PREFIX} and ./deploy/podman/up.sh --release ${TAG}"
