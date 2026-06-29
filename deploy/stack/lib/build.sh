#!/usr/bin/env bash
set -euo pipefail

# Build gateway + worker images. Default: linux-compile + prebuilt Containerfiles (no cargo in image
# build). Pass --in-container for legacy in-Dockerfile cargo (slow; not used in CI).
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
  --clean          Run gateway.sh clean before build (full rebuild)
  --no-clean       Skip clean (default; incremental compile)
  --log PATH       Tee full stdout/stderr to PATH (default: deploy/stack/.build.log)
  --no-log         Print only to terminal
  --in-container   Force in-image cargo build (slow on macOS; hits crates.io in build)
  --skip-playground  Skip playground image npm build (pack-deploy default; uses slim + bind mount)
  -h, --help       Show this help

Default: container-run compile + prebuilt images (see deploy/stack/lib/linux-compile.sh).
EOF
}

CLAW_BUILD_IN_CONTAINER_IMAGE=0
CLAW_BUILD_CLEAN=0
CLAW_BUILD_SKIP_PLAYGROUND=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      build_usage
      exit 0
      ;;
    --clean)
      CLAW_BUILD_CLEAN=1
      shift
      ;;
    --no-clean)
      CLAW_BUILD_CLEAN=0
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
RELAXED_WORKER_IMAGE_NAME="claw-gateway-worker-relaxed:${IMAGE_TAG}"
PLAYGROUND_IMAGE_NAME="claw-gateway-playground:${IMAGE_TAG}"
OVS_IMAGE_NAME="claw-openvscode-server:${IMAGE_TAG}"

# FC backend runs the worker inside e2b sandboxes via CLAW_FC_WORKER_IMAGE (remote registry),
# so the local claw-gateway-worker[-relaxed] images are unused on dev machines. Skip them to
# cut pack-deploy time. CI/release (GITHUB_ACTIONS) still bakes worker images for all backends. kejiqing
skip_local_worker_images() {
  [[ "${GITHUB_ACTIONS:-}" != "true" && "${CLAW_INTERACTIVE_BACKEND:-}" == "fc" ]]
}
WORKER_IMAGES_NOTE="${WORKER_IMAGE_NAME} ${RELAXED_WORKER_IMAGE_NAME}"
if skip_local_worker_images; then
  WORKER_IMAGES_NOTE="(skipped: FC backend uses CLAW_FC_WORKER_IMAGE)"
fi

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

if [[ "${CLAW_BUILD_CLEAN}" == "1" ]]; then
  step "0/N clean (gateway.sh build --clean)"
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
OVS_BASE_IMAGE="${CLAW_OVS_UPSTREAM_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/openvscode-server:1.109.5-ovs-chat}"
echo "==> Rust locked: ${CLAW_RUST_VERSION} (image ${RUST_BASE_IMAGE})"

cn_mirror_enabled() {
  [[ "${GITHUB_ACTIONS:-}" == "true" ]] && return 1
  [[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" ]] || [[ "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]]
}


claw_build_ovs_image() {
  local container_cli="$1"
  local image_name="$2"
  local ovs_base="$3"
  local root_dir="$4"
  shift 4
  if [[ "${CLAW_FORCE_REBUILD_OVS:-0}" != "1" ]] && [[ "${CLAW_OVS_IMAGE:-}" != "${image_name}" ]] && \
    [[ "${CLAW_OVS_IMAGE:-}" != "claw-openvscode-server:"* ]]; then
    step "skip ovs layer build (CLAW_OVS_IMAGE=${CLAW_OVS_IMAGE:-<upstream>}; set CLAW_OVS_IMAGE=${image_name} + CLAW_FORCE_REBUILD_OVS=1 to bake claw-vscode)"
    return 0
  fi
  if [[ "${CLAW_FORCE_REBUILD_OVS:-0}" != "1" ]] && "${container_cli}" image exists "${image_name}" 2>/dev/null; then
    step "skip ovs image (exists: ${image_name}; CLAW_FORCE_REBUILD_OVS=1 to rebuild)"
    return 0
  fi
  step "image ${image_name} (Containerfile.openvscode)"
  chmod +x "${root_dir}/deploy/stack/lib/package-ovs-extension-vsix.sh"
  "${root_dir}/deploy/stack/lib/package-ovs-extension-vsix.sh" \
    "${root_dir}/extensions/claw-vscode" \
    "${root_dir}/deploy/stack/claw.claw-vscode-0.2.0.vsix"
  # shellcheck disable=SC2086
  "${container_cli}" build \
    --build-arg "OVS_BASE_IMAGE=${ovs_base}" \
    "$@" \
    -f "${root_dir}/deploy/stack/Containerfile.openvscode" \
    -t "${image_name}" \
    "${root_dir}"
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
  [[ "${CLAW_BUILD_IN_CONTAINER_IMAGE}" != "1" ]]
}

CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1
CN_FLAG=0
cn_mirror_enabled && CN_FLAG=1

if use_prebuilt_linux_path; then
  echo "==> config: linux-compile + prebuilt images (no cargo in image build)"
  STACK_DIR="${ROOT_DIR}/deploy/stack"
  step "1/3 linux compile (podman run; volumes claw-cargo-registry / claw-cargo-git / claw-sccache persist)"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/linux-compile.sh"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/rust-compile-image.sh"
  COMPILE_IMAGE="$(claw_ensure_rust_compile_image "${ROOT_DIR}" "${CONTAINER_CLI}" "${REG}")"
  claw_linux_compile_release "${ROOT_DIR}" "${CONTAINER_CLI}" "${COMPILE_IMAGE}" "${CN_FLAG}"

  step "package claw-vscode VSIX (gateway OVS bootstrap)"
  "${ROOT_DIR}/deploy/stack/lib/package-claw-vscode-vsix.sh"

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

  if skip_local_worker_images; then
    step "skip worker images (FC backend: worker runs in e2b via CLAW_FC_WORKER_IMAGE; set CLAW_INTERACTIVE_BACKEND!=fc or GITHUB_ACTIONS to build)"
  else
    step "3/4 image ${WORKER_IMAGE_NAME} (Containerfile.gateway-worker.prebuilt)"
    # shellcheck disable=SC2086
    "${CONTAINER_CLI}" build \
      --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_BASE_IMAGE}" \
      "${APT_MIRROR_BUILD_ARGS[@]}" \
      -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker.prebuilt" \
      -t "${WORKER_IMAGE_NAME}" \
      "${ROOT_DIR}"

    step "4/4 image ${RELAXED_WORKER_IMAGE_NAME} (Containerfile.gateway-worker-relaxed.prebuilt)"
    # shellcheck disable=SC2086
    "${CONTAINER_CLI}" build \
      --build-arg "WORKER_BASE_IMAGE=${WORKER_IMAGE_NAME}" \
      "${APT_MIRROR_BUILD_ARGS[@]}" \
      -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker-relaxed.prebuilt" \
      -t "${RELAXED_WORKER_IMAGE_NAME}" \
      "${ROOT_DIR}"
  fi

  claw_build_playground_image "${CONTAINER_CLI}" "${PLAYGROUND_IMAGE_NAME}" "${DEBIAN_BASE_IMAGE}" "${NODE_BASE_IMAGE}" "${ROOT_DIR}" "${APT_MIRROR_BUILD_ARGS[@]}"
  claw_build_ovs_image "${CONTAINER_CLI}" "${OVS_IMAGE_NAME}" "${OVS_BASE_IMAGE}" "${ROOT_DIR}" "${APT_MIRROR_BUILD_ARGS[@]}"

else
  step "config: in-image cargo build (Containerfile.gateway-rs)"
  RUSTUP_BUILD_ARGS=()
  CARGO_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_CRATES_MIRROR=0")
  APT_MIRROR_BUILD_ARGS=(--build-arg "CLAW_USE_CN_APT_MIRROR=0")
  if cn_mirror_enabled; then
    RUSTUP_BUILD_ARGS=(
      --build-arg "RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static"
      --build-arg "RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup"
    )
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

  if skip_local_worker_images; then
    step "skip worker images (FC backend: worker runs in e2b via CLAW_FC_WORKER_IMAGE)"
  else
    step "2/5 image ${WORKER_IMAGE_NAME}"
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

    step "3/5 image ${RELAXED_WORKER_IMAGE_NAME} (Containerfile.gateway-worker-relaxed)"
    # shellcheck disable=SC2086
    "${CONTAINER_CLI}" build \
      --build-arg "WORKER_BASE_IMAGE=${WORKER_IMAGE_NAME}" \
      "${APT_MIRROR_BUILD_ARGS[@]}" \
      -f "${ROOT_DIR}/deploy/stack/Containerfile.gateway-worker-relaxed" \
      -t "${RELAXED_WORKER_IMAGE_NAME}" \
      "${ROOT_DIR}"
  fi

  claw_build_playground_image "${CONTAINER_CLI}" "${PLAYGROUND_IMAGE_NAME}" "${DEBIAN_BASE_IMAGE}" "${NODE_BASE_IMAGE}" "${ROOT_DIR}" "${APT_MIRROR_BUILD_ARGS[@]}"
  claw_build_ovs_image "${CONTAINER_CLI}" "${OVS_IMAGE_NAME}" "${OVS_BASE_IMAGE}" "${ROOT_DIR}" "${APT_MIRROR_BUILD_ARGS[@]}"

fi

"${ROOT_DIR}/deploy/stack/lib/claw-write-build-stamp.sh"

step "done"
echo "Built: ${IMAGE_NAME} ${WORKER_IMAGES_NOTE} ${PLAYGROUND_IMAGE_NAME} ${OVS_IMAGE_NAME}"
claw_timing_summary
if [[ "${CLAW_BUILD_NO_LOG}" != "1" ]]; then
  echo "log: ${BUILD_LOG}"
fi
