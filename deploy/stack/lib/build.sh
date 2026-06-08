#!/usr/bin/env bash
set -euo pipefail

# Build gateway + worker images. Darwin default: linux-compile (podman run + cache volumes) then
# prebuilt Containerfiles (no cargo during `podman build`). CI/Linux: full Containerfile.*.
# Author: kejiqing
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/compose-include.sh"
if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
  claw_apply_deploy_profile || exit 1
fi

# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/claw-step-timing.sh"

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
  --skip-playground  Skip playground image npm build (pack-deploy default; uses slim + bind mount)
  -h, --help       Show this help

Darwin default: podman run compile + prebuilt images (see deploy/stack/lib/linux-compile.sh).
EOF
}

CLAW_BUILD_IN_CONTAINER_IMAGE=0
CLAW_BUILD_NO_CLEAN=0
CLAW_BUILD_SKIP_PLAYGROUND=0
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
    --skip-playground)
      CLAW_BUILD_SKIP_PLAYGROUND=1
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
  claw_step_begin "$*"
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

CLAW_TIMING_LABEL="build timing"
claw_timing_init

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
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/rust-version.env"
export CLAW_RUST_VERSION CLAW_RUST_IMAGE_TAG
RUST_BASE_IMAGE="${REG}/library/rust:${CLAW_RUST_IMAGE_TAG}"
DEBIAN_BASE_IMAGE="${REG}/library/debian:bookworm-slim"
NODE_BASE_IMAGE="${REG}/library/node:20-alpine"
echo "==> Rust locked: ${CLAW_RUST_VERSION} (image ${RUST_BASE_IMAGE})"

cn_mirror_enabled() {
  [[ "${GITHUB_ACTIONS:-}" == "true" ]] && return 1
  [[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" ]] || [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]]
}


claw_build_playground_image() {
  local container_cli="$1"
  local image_name="$2"
  local debian_base="$3"
  local node_base="$4"
  local root_dir="$5"
  shift 5

  if [[ "${CLAW_BUILD_SKIP_PLAYGROUND}" == "1" ]]; then
    if "${container_cli}" image exists "${image_name}" 2>/dev/null; then
      step "skip playground image (exists: ${image_name})"
      return 0
    fi
    step "playground slim image ${image_name} (admin via bind mount when dist/ present)"
    # shellcheck disable=SC2086
    "${container_cli}" build \
      --build-arg "DEBIAN_BASE_IMAGE=${debian_base}" \
      "$@" \
      -f "${root_dir}/deploy/stack/Containerfile.gateway-playground.slim" \
      -t "${image_name}" \
      "${root_dir}"
    return 0
  fi

  step "image ${image_name} (admin SPA built inside Containerfile / CI)"
  # shellcheck disable=SC2086
  "${container_cli}" build \
    --build-arg "DEBIAN_BASE_IMAGE=${debian_base}" \
    --build-arg "NODE_BASE_IMAGE=${node_base}" \
    "$@" \
    -f "${root_dir}/deploy/stack/Containerfile.gateway-playground" \
    -t "${image_name}" \
    "${root_dir}"
}

use_prebuilt_linux_path() {
  [[ "${CLAW_BUILD_IN_CONTAINER_IMAGE}" == "1" ]] && return 1
  [[ "$(uname -s)" == "Darwin" ]]
}

CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1
CN_FLAG=0
cn_mirror_enabled && CN_FLAG=1

if use_prebuilt_linux_path; then
  echo "==> config: Darwin → podman run compile + prebuilt images (no cargo in podman build)"
  STACK_DIR="${ROOT_DIR}/deploy/stack"
  step "1/3 linux compile (podman run; volumes claw-cargo-registry / claw-cargo-git persist)"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/linux-compile.sh"
  claw_linux_compile_release "${ROOT_DIR}" "${CONTAINER_CLI}" "${RUST_BASE_IMAGE}" "${CN_FLAG}"

  APT_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_APT_MIRROR=0")
  cn_mirror_enabled && APT_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_APT_MIRROR=1")

  step "2/3 image ${IMAGE_NAME} (Containerfile.gateway-rs.prebuilt)"
  # shellcheck disable=SC2086
  "${CONTAINER_CLI}" build \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    "${APT_MIRROR_BUILD_ARGS[@]}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-rs.prebuilt" \
    -t "${IMAGE_NAME}" \
    "${ROOT_DIR}"

  step "3/3 image ${WORKER_IMAGE_NAME} (Containerfile.gateway-worker.prebuilt)"
  # shellcheck disable=SC2086
  "${CONTAINER_CLI}" build \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    "${APT_MIRROR_BUILD_ARGS[@]}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker.prebuilt" \
    -t "${WORKER_IMAGE_NAME}" \
    "${ROOT_DIR}"

  claw_build_playground_image "${CONTAINER_CLI}" "${PLAYGROUND_IMAGE_NAME}" "${DEBIAN_BASE_IMAGE}" "${NODE_BASE_IMAGE}" "${ROOT_DIR}" "${APT_MIRROR_BUILD_ARGS[@]}"

  if command -v cargo >/dev/null 2>&1; then
    # shellcheck source=/dev/null
    source "${ROOT_DIR}/deploy/stack/lib/pool-daemon-binary.sh"
    step "host claw-pool-daemon (macOS sidecar)"
    CLAW_POOL_REBUILD_DAEMON=1 claw_ensure_pool_daemon_binary "${STACK_DIR}" "${ROOT_DIR}" >/dev/null
  fi
else
  step "config: in-image cargo build (Containerfile.gateway-rs)"
  RUSTUP_BUILD_ARGS=()
  CARGO_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_CRATES_MIRROR=0")
  APT_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_APT_MIRROR=0")
  if [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]] && [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
    RUSTUP_BUILD_ARGS=(
      --build-arg "RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static"
      --build-arg "RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup"
    )
  fi
  if cn_mirror_enabled; then
    CARGO_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_CRATES_MIRROR=1")
    APT_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_APT_MIRROR=1")
  fi

  step "1/3 image ${IMAGE_NAME}"
  # shellcheck disable=SC2086
  "${CONTAINER_CLI}" build \
    --build-arg "RUST_BASE_IMAGE=${RUST_BASE_IMAGE}" \
    --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
    "${RUSTUP_BUILD_ARGS[@]}" \
    "${CARGO_MIRROR_BUILD_ARGS[@]}" \
    "${APT_MIRROR_BUILD_ARGS[@]}" \
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
    "${APT_MIRROR_BUILD_ARGS[@]}" \
    -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker" \
    -t "${WORKER_IMAGE_NAME}" \
    "${ROOT_DIR}"

  claw_build_playground_image "${CONTAINER_CLI}" "${PLAYGROUND_IMAGE_NAME}" "${DEBIAN_BASE_IMAGE}" "${NODE_BASE_IMAGE}" "${ROOT_DIR}" "${APT_MIRROR_BUILD_ARGS[@]}"

  STACK_DIR="${ROOT_DIR}/deploy/stack"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/pool-daemon-binary.sh"
  if [[ "$(uname -s)" == "Darwin" ]] && command -v cargo >/dev/null 2>&1; then
    step "host claw-pool-daemon (macOS cargo build)"
    CLAW_POOL_REBUILD_DAEMON=1 claw_ensure_pool_daemon_binary "${STACK_DIR}" "${ROOT_DIR}" >/dev/null
  elif claw_gateway_image_carries_pool_daemon "${IMAGE_NAME}"; then
    step "host claw-pool-daemon (from ${IMAGE_NAME})"
    GATEWAY_IMAGE="${IMAGE_NAME}" claw_ensure_pool_daemon_binary "${STACK_DIR}" "${ROOT_DIR}" >/dev/null
  fi
fi

"${ROOT_DIR}/deploy/stack/lib/claw-write-build-stamp.sh"

step "done"
echo "Built: ${IMAGE_NAME} ${WORKER_IMAGE_NAME} ${PLAYGROUND_IMAGE_NAME}"
claw_timing_summary
if [[ "${CLAW_BUILD_NO_LOG}" != "1" ]]; then
  echo "log: ${BUILD_LOG}"
fi
