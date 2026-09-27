#!/usr/bin/env bash
# Author: kejiqing
# Jenkins / private-cluster: same package chain as .github/workflows/claw-code-image.yaml,
# but push images to Nora only (no GHCR / ACR). Trigger: release-v* tag.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/rust-compile-image.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/linux-compile.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/rust-version.env"

NEXUS_PUSH_REGISTRY="${NEXUS_PUSH_REGISTRY:-nora.workbox.spone.xyz}"
NEXUS_NS="${NEXUS_NS:-passionke}"
REGION="${REGION:-china}"
: "${NEXUS_USER:?NEXUS_USER is required}"
: "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required}"

if [[ "$REGION" != "china" && "$REGION" != "else" ]]; then
  echo "REGION must be china or else, got: $REGION" >&2
  exit 1
fi

RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
if [[ -z "$RELEASE_TAG" ]]; then
  RELEASE_TAG="$(git -C "$ROOT" describe --tags --exact-match HEAD 2>/dev/null || true)"
fi
if [[ ! "$RELEASE_TAG" =~ ^release-v[0-9] ]]; then
  echo "RELEASE_TAG must match release-v*, got: ${RELEASE_TAG:-<empty>}" >&2
  exit 1
fi

SHA12="$(git -C "$ROOT" rev-parse --short=12 HEAD)"
CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1

if [[ "$REGION" == "china" ]]; then
  export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-1}"
  export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-1}"
  export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-1}"
  REG="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
else
  export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-0}"
  export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-0}"
  export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-0}"
  REG="${CONTAINER_BASE_REGISTRY:-docker.io}"
fi
REG="${REG%/}"
DEBIAN_IMAGE="${REG}/library/debian:bookworm-slim"
NODE_IMAGE="${REG}/library/node:20-alpine"
APT_CN="$CLAW_USE_CN_APT_MIRROR"

echo "==> publish ${RELEASE_TAG} (sha-${SHA12}) → ${NEXUS_PUSH_REGISTRY}/${NEXUS_NS} REGION=${REGION}"

echo "$NEXUS_PASSWORD" | docker login "$NEXUS_PUSH_REGISTRY" -u "$NEXUS_USER" --password-stdin

echo "==> ensure rust compile image"
COMPILE_IMAGE="$(claw_ensure_rust_compile_image "$ROOT" "$CONTAINER_CLI" "$REG")"

echo "==> linux compile once"
export CLAW_CONTAINER_RUNTIME=docker
export CLAW_LINUX_COMPILE_CI=1
export CLAW_RUST_COMPILE_IMAGE="$COMPILE_IMAGE"
export CONTAINER_BASE_REGISTRY="$REG"
CN_FLAG=0
claw_cn_mirror_enabled && CN_FLAG=1
claw_linux_compile_release "$ROOT" "$CONTAINER_CLI" "$COMPILE_IMAGE" "$CN_FLAG"

echo "==> package claw-vscode VSIX"
bash "$ROOT/deploy/stack/lib/package-claw-vscode-vsix.sh"

push_package() {
  local package="$1"
  local dockerfile="$2"
  local -a build_args=()
  local image ref
  image="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/${package}"
  case "$package" in
    claw-gateway-worker-relaxed)
      build_args=(
        --build-arg "WORKER_BASE_IMAGE=${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-worker:${RELEASE_TAG}"
        --build-arg "CLAW_USE_CN_APT_MIRROR=${APT_CN}"
      )
      ;;
    claw-gateway-playground)
      build_args=(
        --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_IMAGE}"
        --build-arg "NODE_BASE_IMAGE=${NODE_IMAGE}"
        --build-arg "CLAW_USE_CN_APT_MIRROR=${APT_CN}"
      )
      ;;
    *)
      build_args=(
        --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_IMAGE}"
        --build-arg "CLAW_USE_CN_APT_MIRROR=${APT_CN}"
      )
      ;;
  esac
  echo "==> build ${image}:${RELEASE_TAG}"
  docker build \
    -f "$ROOT/${dockerfile}" \
    "${build_args[@]}" \
    -t "${image}:${RELEASE_TAG}" \
    -t "${image}:sha-${SHA12}" \
    -t "${image}:latest" \
    "$ROOT"
  for tag in "${RELEASE_TAG}" "sha-${SHA12}" latest; do
    echo "==> push ${image}:${tag}"
    docker push "${image}:${tag}"
  done
}

push_package claw-code deploy/stack/Containerfile.gateway-rs.prebuilt
push_package claw-gateway-worker deploy/stack/Containerfile.gateway-worker.prebuilt
push_package claw-gateway-playground deploy/stack/Containerfile.gateway-playground
push_package claw-gateway-worker-relaxed deploy/stack/Containerfile.gateway-worker-relaxed

echo "==> verify playground admin assets"
docker run --rm "${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-playground:${RELEASE_TAG}" \
  sh -c 'test -f /app/admin-dist/index.html && test -n "$(ls /app/admin-dist/assets/*.js 2>/dev/null)"'

echo "==> verify relaxed worker tools"
docker run --rm --entrypoint sh \
  "${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-worker-relaxed:${RELEASE_TAG}" \
  -c 'command -v curl && command -v git && command -v python3'

echo "published ${NEXUS_NS}/{claw-code,claw-gateway-worker,claw-gateway-worker-relaxed,claw-gateway-playground}:${RELEASE_TAG}"
