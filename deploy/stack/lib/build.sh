#!/usr/bin/env bash
set -euo pipefail

# Build gateway + worker images. Darwin default: linux-compile (podman run + cache volumes) then
# prebuilt Containerfiles (no cargo during `podman build`). CI/Linux: full Containerfile.*.
# Author: kejiqing
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/compose-include.sh"

CLAW_BUILD_NO_LOG="${CLAW_BUILD_NO_LOG:-}"
BUILD_LOG="${CLAW_BUILD_LOG:-${ROOT_DIR}/deploy/stack/.build.log}"

build_usage() {
  cat <<EOF
Usage: gateway.sh build [IMAGE_TAG] [options]

Options:
  --no-clean       Skip gateway.sh clean before build (default: clean first)
  --log PATH       Tee full stdout/stderr to PATH (default: deploy/stack/.build.log)
  --no-log         Print only to terminal
  --in-container   Force in-image cargo build (slow on macOS; hits crates.io in build)
  -h, --help       Show this help

Darwin default: podman run compile + prebuilt images (see deploy/stack/lib/linux-compile.sh).
EOF
}

CLAW_BUILD_IN_CONTAINER_IMAGE=0
CLAW_BUILD_NO_CLEAN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      build_usage
      exit 0
      ;;
    --no-clean)
      CLAW_BUILD_NO_CLEAN=1
      shift
      ;;
    --log)
      [[ $# -ge 2 ]] || {
        echo "--log requires a path" >&2
        exit 2
      }
      BUILD_LOG="$2"
      shift 2
      ;;
    --no-log)
      CLAW_BUILD_NO_LOG=1
      shift
      ;;
    --in-container)
      CLAW_BUILD_IN_CONTAINER_IMAGE=1
      shift
      ;;
    --*)
      echo "unknown build option: $1" >&2
      build_usage >&2
      exit 2
      ;;
    *)
      break
      ;;
  esac
done

IMAGE_TAG="${1:-local}"
IMAGE_NAME="claw-gateway-rs:${IMAGE_TAG}"
WORKER_IMAGE_NAME="claw-gateway-worker:${IMAGE_TAG}"
PLAYGROUND_IMAGE_NAME="claw-gateway-playground:${IMAGE_TAG}"

step() {
  echo ""
  echo "========== $* =========="
  echo ""
}

setup_build_log() {
  if [[ "${CLAW_BUILD_NO_LOG}" == "1" ]]; then
    return 0
  fi
  mkdir -p "$(dirname "${BUILD_LOG}")"
  : >"${BUILD_LOG}"
  echo "==> build log: ${BUILD_LOG}"
  echo "==> started: $(date '+%Y-%m-%d %H:%M:%S %z')"
  exec > >(tee -a "${BUILD_LOG}") 2>&1
}

setup_build_log

if [[ "${CLAW_BUILD_NO_CLEAN}" != "1" ]]; then
  step "0/N clean (default before build; gateway.sh build --no-clean to skip)"
  "${ROOT_DIR}/deploy/stack/lib/clean.sh"
fi

if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  REG="docker.io"
else
  REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  REG="${REG%/}"
fi
RUST_BASE_IMAGE="${REG}/library/rust:1.88-bookworm"
DEBIAN_BASE_IMAGE="${REG}/library/debian:bookworm-slim"

cn_mirror_enabled() {
  [[ "${GITHUB_ACTIONS:-}" == "true" ]] && return 1
  [[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" ]] || [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]]
}

use_prebuilt_linux_path() {
  [[ "${CLAW_BUILD_IN_CONTAINER_IMAGE}" == "1" ]] && return 1
  [[ "$(uname -s)" == "Darwin" ]]
}

CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1
CN_FLAG=0
cn_mirror_enabled && CN_FLAG=1

if use_prebuilt_linux_path; then
  step "config: Darwin → podman run compile + prebuilt images (no cargo in podman build)"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/linux-compile.sh"
  step "1/4 linux compile (podman run; volumes claw-cargo-registry / claw-cargo-git persist)"
  claw_linux_compile_release "${ROOT_DIR}" "${CONTAINER_CLI}" "${RUST_BASE_IMAGE}" "${CN_FLAG}"

  step "2/4 image ${IMAGE_NAME} (Containerfile.gateway-rs.prebuilt)"
  "${CONTAINER_CLI}" build \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-rs.prebuilt" \
    -t "${IMAGE_NAME}" \
    "${ROOT_DIR}"

  step "3/4 image ${WORKER_IMAGE_NAME} (Containerfile.gateway-worker.prebuilt)"
  "${CONTAINER_CLI}" build \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker.prebuilt" \
    -t "${WORKER_IMAGE_NAME}" \
    "${ROOT_DIR}"

  step "4/4 gateway-admin dist + image ${PLAYGROUND_IMAGE_NAME}"
  "${ROOT_DIR}/deploy/stack/lib/build-gateway-admin.sh"
  "${CONTAINER_CLI}" build \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-playground" \
    -t "${PLAYGROUND_IMAGE_NAME}" \
    "${ROOT_DIR}"

  if command -v cargo >/dev/null 2>&1; then
    step "host claw-pool-daemon (macOS sidecar; optional if image binary used)"
    (cd "${ROOT_DIR}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon) || true
    echo "Host binary (pool sidecar): ${ROOT_DIR}/rust/target/release/claw-pool-daemon"
  fi
else
  step "config: in-image cargo build (Containerfile.gateway-rs)"
  RUSTUP_BUILD_ARGS=()
  CARGO_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_CRATES_MIRROR=0")
  if [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]] && [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
    RUSTUP_BUILD_ARGS=(
      --build-arg "RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static"
      --build-arg "RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup"
    )
  fi
  if cn_mirror_enabled; then
    CARGO_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_CRATES_MIRROR=1")
  fi

  step "1/3 image ${IMAGE_NAME}"
  # shellcheck disable=SC2086
  "${CONTAINER_CLI}" build \
    --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    "${RUSTUP_BUILD_ARGS[@]}" \
    "${CARGO_MIRROR_BUILD_ARGS[@]}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-rs" \
    -t "${IMAGE_NAME}" \
    "${ROOT_DIR}"

  step "2/4 image ${WORKER_IMAGE_NAME}"
  # shellcheck disable=SC2086
  "${CONTAINER_CLI}" build \
    --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    "${RUSTUP_BUILD_ARGS[@]}" \
    "${CARGO_MIRROR_BUILD_ARGS[@]}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker" \
    -t "${WORKER_IMAGE_NAME}" \
    "${ROOT_DIR}"

  step "3/4 gateway-admin dist + image ${PLAYGROUND_IMAGE_NAME}"
  "${ROOT_DIR}/deploy/stack/lib/build-gateway-admin.sh"
  "${CONTAINER_CLI}" build \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-playground" \
    -t "${PLAYGROUND_IMAGE_NAME}" \
    "${ROOT_DIR}"

  if [[ "$(uname -s)" == "Darwin" ]] && command -v cargo >/dev/null 2>&1; then
    step "4/4 host claw-pool-daemon"
    (cd "${ROOT_DIR}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)
  fi
fi

step "done"
echo "Built: ${IMAGE_NAME} ${WORKER_IMAGE_NAME} ${PLAYGROUND_IMAGE_NAME}"
if [[ "${CLAW_BUILD_NO_LOG}" != "1" ]]; then
  echo "log: ${BUILD_LOG}"
fi
