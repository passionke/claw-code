#!/usr/bin/env bash
# Build and publish the complete e2b Worker protocol layer. Author: kejiqing
# This entry never builds Gateway/Admin or Agent engine artifacts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/rust-compile-image.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/linux-compile.sh"

TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
PREFIX="${CLAW_IMAGE_PREFIX:-${ACR_REGISTRY:-}}"
if [[ -z "$TAG" || -z "$PREFIX" ]]; then
  echo "usage: RELEASE_TAG=<version> CLAW_IMAGE_PREFIX=<registry/namespace> $0" >&2
  exit 2
fi
PREFIX="${PREFIX%/}"
REGION="${REGION:-china}"
claw_region_load

PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
export CLAW_LINUX_COMPILE_PLATFORM="$PLATFORM"
export CLAW_LINUX_RELEASE_BINS="claw neuro-opencode neuro-appserver"
export CLAW_LINUX_COMPILE_CI=1
export CLAW_CONTAINER_RUNTIME=docker

if claw_region_is_china; then
  export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-1}"
  export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-1}"
  export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-1}"
  BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
else
  export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-0}"
  export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-0}"
  export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-0}"
  BASE_REGISTRY="${CONTAINER_BASE_REGISTRY:-docker.io}"
fi
BASE_REGISTRY="${BASE_REGISTRY%/}"
export CONTAINER_BASE_REGISTRY="$BASE_REGISTRY"

CLI="$(claw_container_runtime_cli)"
COMPILE_IMAGE="$(claw_ensure_rust_compile_image "$ROOT" "$CLI" "$BASE_REGISTRY")"
CN_FLAG=0
claw_cn_mirror_enabled && CN_FLAG=1
claw_linux_compile_release "$ROOT" "$CLI" "$COMPILE_IMAGE" "$CN_FLAG"

USER_NAME="${CLAW_REGISTRY_USER:-${ACR_USERNAME:-${NEXUS_USER:-}}}"
PASSWORD="${CLAW_REGISTRY_PASSWORD:-${ACR_PASSWORD:-${NEXUS_PASSWORD:-}}}"
: "${USER_NAME:?set CLAW_REGISTRY_USER/ACR_USERNAME/NEXUS_USER}"
: "${PASSWORD:?set CLAW_REGISTRY_PASSWORD/ACR_PASSWORD/NEXUS_PASSWORD}"
printf '%s' "$PASSWORD" | docker login "${PREFIX%%/*}" -u "$USER_NAME" --password-stdin

SHA12="$(git -C "$ROOT" rev-parse --short=12 HEAD)"
# Worker-series protocol source (claw + neuro-*). e2b register retags to
# debian-bookworm-claw-worker* via Dockerfile.claw-worker-*-selfhosted + Template.build.
# Author: kejiqing
STRICT="${PREFIX}/claw-worker-base"
RELAXED="${PREFIX}/claw-worker-base-relaxed"
DEBIAN="${BASE_REGISTRY}/library/debian:bookworm-slim"

"$ROOT/deploy/stack/lib/container-build.sh" docker \
  "$ROOT/deploy/e2b/Containerfile.worker-protocol" \
  --platform "$PLATFORM" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN}" \
  --build-arg "CLAW_USE_CN_APT_MIRROR=${CLAW_USE_CN_APT_MIRROR}" \
  -t "${STRICT}:${TAG}"
"$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" \
  "${STRICT}:${TAG}" "${STRICT}:${TAG}" "${STRICT}:sha-${SHA12}"

"$ROOT/deploy/stack/lib/container-build.sh" docker \
  "$ROOT/deploy/e2b/Containerfile.worker-protocol-relaxed" \
  --platform "$PLATFORM" \
  --build-arg "WORKER_BASE_IMAGE=${STRICT}:${TAG}" \
  --build-arg "CLAW_USE_CN_APT_MIRROR=${CLAW_USE_CN_APT_MIRROR}" \
  -t "${RELAXED}:${TAG}"
"$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" \
  "${RELAXED}:${TAG}" "${RELAXED}:${TAG}" "${RELAXED}:sha-${SHA12}"

for image in "${STRICT}:${TAG}" "${RELAXED}:${TAG}"; do
  docker run --rm --entrypoint sh "$image" -c \
    'command -v claw && command -v neuro-opencode && command -v neuro-appserver && test ! -e /usr/local/lib/neuro-engines'
done

# Host docker builds the e2b-facing layer (python may run inside a container without docker).
# Author: kejiqing
E2B_STRICT="${PREFIX}/debian-bookworm-claw-worker"
E2B_RELAXED="${PREFIX}/debian-bookworm-claw-worker-relaxed"
APT_MIRROR=""
if claw_region_is_china; then
  APT_MIRROR="mirrors.aliyun.com"
fi
docker build \
  -f "$ROOT/deploy/e2b/Dockerfile.claw-worker-selfhosted" \
  --build-arg "WORKER_BASE_IMAGE=${STRICT}:${TAG}" \
  --build-arg "DEBIAN_APT_MIRROR=${APT_MIRROR}" \
  --platform "$PLATFORM" \
  -t "${E2B_STRICT}:${TAG}" \
  "$ROOT/deploy/e2b"
"$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" \
  "${E2B_STRICT}:${TAG}" "${E2B_STRICT}:${TAG}" "${E2B_STRICT}:sha-${SHA12}"

docker build \
  -f "$ROOT/deploy/e2b/Dockerfile.claw-worker-relaxed-selfhosted" \
  --build-arg "WORKER_BASE_IMAGE=${RELAXED}:${TAG}" \
  --build-arg "DEBIAN_APT_MIRROR=${APT_MIRROR}" \
  --platform "$PLATFORM" \
  -t "${E2B_RELAXED}:${TAG}" \
  "$ROOT/deploy/e2b"
"$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" \
  "${E2B_RELAXED}:${TAG}" "${E2B_RELAXED}:${TAG}" "${E2B_RELAXED}:sha-${SHA12}"

export CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image
export CLAW_E2B_WORKER_IMAGE="${STRICT}:${TAG}"
export CLAW_E2B_TEMPLATE_FROM_IMAGE="${STRICT}:${TAG}"
export CLAW_E2B_WORKER_RELAXED_IMAGE="${RELAXED}:${TAG}"
export CLAW_E2B_WORKER_E2B_IMAGE="${E2B_STRICT}:${TAG}"
export CLAW_E2B_WORKER_RELAXED_E2B_IMAGE="${E2B_RELAXED}:${TAG}"
export CLAW_E2B_WORKER_SKIP_LOCAL_BUILD=1

# CLAW_E2B_PYTHON wins (e.g. docker-wrapped interpreter on home29). Author: kejiqing
PYTHON="${CLAW_E2B_PYTHON:-}"
if [[ -z "$PYTHON" ]]; then
  PYTHON="${CLAW_E2B_VENV:+${CLAW_E2B_VENV}/bin/python3}"
fi
if [[ -z "$PYTHON" || ! -x "$PYTHON" ]]; then
  PYTHON="$(command -v python3)"
fi
"$PYTHON" -c 'import e2b' >/dev/null
"$PYTHON" "$ROOT/deploy/e2b/build-claw-worker-selfhosted.py"
"$PYTHON" "$ROOT/deploy/e2b/build-claw-worker-relaxed-selfhosted.py"

echo "published e2b Worker protocol ${TAG}: ${STRICT}, ${RELAXED} → ${E2B_STRICT}, ${E2B_RELAXED}"
