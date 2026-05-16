#!/usr/bin/env bash
set -euo pipefail

# Build gateway (http-gateway-rs + claw-pool-daemon) and worker (claw) images in one run.
# Same rust/ tree, same base images and rustup build-args — pair after any Rust change. Author: kejiqing
#
# Full output is always tee'd to deploy/stack/.build.log (override: --log PATH, disable: --no-log).
# Do not pipe this script to `tail` if you need the middle — open the log file instead.
#
# Base image registry (hostname only, no path); same name as GitHub Actions variable
# CONTAINER_BASE_REGISTRY in claw-code-image workflow.
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
  --log PATH    Tee full stdout/stderr to PATH (default: deploy/stack/.build.log)
  --no-log      Print only to terminal (no log file)
  -h, --help    Show this help

Tips:
  Full log while building:  tail -f deploy/stack/.build.log
  Avoid losing output:      do not pipe build to \`tail\`; read the log file instead.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      build_usage
      exit 0
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
  echo "==> tip: in another terminal run: tail -f ${BUILD_LOG}"
  exec > >(tee -a "${BUILD_LOG}") 2>&1
}

setup_build_log

if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  REG="docker.io"
  step "config: base images from docker.io (CI or CLAW_USE_DOCKER_IO=1)"
else
  REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  REG="${REG%/}"
  step "config: base images from ${REG}"
fi
RUST_BASE_IMAGE="${REG}/library/rust:1.88-bookworm"
DEBIAN_BASE_IMAGE="${REG}/library/debian:bookworm-slim"

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

step "1/3 image ${IMAGE_NAME} (Containerfile.gateway-rs)"
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
step "2/3 image ${WORKER_IMAGE_NAME} (Containerfile.gateway-worker)"
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

if [[ "$(uname -s)" == "Darwin" ]] && command -v cargo >/dev/null 2>&1; then
  step "3/3 host claw-pool-daemon (cargo release, Darwin)"
  (cd "${ROOT_DIR}/rust" && cargo build --release -p http-gateway-rs --bin claw-pool-daemon)
  echo "Host binary: ${ROOT_DIR}/rust/target/release/claw-pool-daemon"
else
  step "3/3 host claw-pool-daemon skipped (not Darwin or no cargo in PATH)"
fi

step "done"
echo "finished: $(date '+%Y-%m-%d %H:%M:%S %z')"
if [[ "${CLAW_BUILD_NO_LOG}" != "1" ]]; then
  echo "full log: ${BUILD_LOG}"
fi
