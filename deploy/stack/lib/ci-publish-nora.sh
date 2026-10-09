#!/usr/bin/env bash
# Author: kejiqing
#
# Nora publish — ONE default path (Jenkins / private cluster).
# Contract (do not invent forks):
#   1) Job: checkout tag → run THIS script only. Never mutate host Docker / Nora.
#   2) This script: preflight → compile once → build → skopeo v2s2 push only.
#   3) Explicit --platform + TARGETARCH (classic builder does not inject them).
#   4) Fail at the stage that breaks; no silent continue.
# Tags: vX.Y.Z or release-v* → ${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}
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

NEXUS_PUSH_REGISTRY="${NEXUS_PUSH_REGISTRY:-nora.home.passionke.top}"
NEXUS_NS="${NEXUS_NS:-passionke}"
REGION="${REGION:-china}"
: "${NEXUS_USER:?NEXUS_USER is required}"
: "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required}"

# Single platform for this publish path (Nora amd64 agents). Author: kejiqing
export CLAW_LINUX_COMPILE_PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
case "${CLAW_LINUX_COMPILE_PLATFORM}" in
  *arm64*|*aarch64*) export TARGETARCH="${TARGETARCH:-arm64}" ;;
  *) export TARGETARCH="${TARGETARCH:-amd64}" ;;
esac
PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM}"

if [[ "$REGION" != "china" && "$REGION" != "else" ]]; then
  echo "REGION must be china or else, got: $REGION" >&2
  exit 1
fi

RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
if [[ -z "$RELEASE_TAG" ]]; then
  RELEASE_TAG="$(git -C "$ROOT" describe --tags --exact-match HEAD 2>/dev/null || true)"
fi
if [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && ! "$RELEASE_TAG" =~ ^release-v[0-9] ]]; then
  echo "RELEASE_TAG must match vX.Y.Z or release-v*, got: ${RELEASE_TAG:-<empty>}" >&2
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

stage() {
  echo "==> [$1] $2"
}

skopeo_push_tags() {
  local local_ref="$1"
  local image="$2"
  chmod +x "$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh"
  "$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" \
    "${local_ref}" \
    "${image}:${RELEASE_TAG}" \
    "${image}:sha-${SHA12}" \
    "${image}:latest"
}

# --- [1] preflight: tools + registry auth challenge (fail before compile) ---
stage "1/6" "preflight publish ${RELEASE_TAG} (sha-${SHA12}) → ${NEXUS_PUSH_REGISTRY}/${NEXUS_NS} REGION=${REGION} platform=${PLATFORM} arch=${TARGETARCH}"

if ! command -v skopeo >/dev/null 2>&1; then
  echo "preflight: skopeo required (only skopeo v2s2 push; no docker push)" >&2
  exit 1
fi
if ! command -v docker >/dev/null 2>&1; then
  echo "preflight: docker CLI required for build/login" >&2
  exit 1
fi

# Unauthenticated /v2/ must 401 so docker/skopeo attach Basic on push.
# If this is 200, login looks OK but push omits Authorization. Author: kejiqing
V2_CODE="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 10 \
  "https://${NEXUS_PUSH_REGISTRY}/v2/" || echo 000)"
if [[ "$V2_CODE" != "401" ]]; then
  echo "preflight: https://${NEXUS_PUSH_REGISTRY}/v2/ returned HTTP ${V2_CODE}, want 401" >&2
  echo "preflight: Nora must challenge (docker_anon_pull=false). Fix registry ops; do not patch this script." >&2
  exit 1
fi

echo "$NEXUS_PASSWORD" | docker login "$NEXUS_PUSH_REGISTRY" -u "$NEXUS_USER" --password-stdin
# Prove credentials work for a write-scope probe (blob upload initiate).
AUTH_B64="$(printf '%s:%s' "$NEXUS_USER" "$NEXUS_PASSWORD" | base64 | tr -d '\n')"
UP_CODE="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 10 \
  -X POST -H "Authorization: Basic ${AUTH_B64}" -H "Content-Length: 0" \
  "https://${NEXUS_PUSH_REGISTRY}/v2/${NEXUS_NS}/claw-code/blobs/uploads/" || echo 000)"
if [[ "$UP_CODE" != "202" ]]; then
  echo "preflight: blob upload initiate returned HTTP ${UP_CODE}, want 202 (bad creds or Nora write auth)" >&2
  exit 1
fi
echo "preflight: ok (v2=401, upload-init=202, login+skopeo present)"

# --- [2] compile image ---
stage "2/6" "ensure rust compile image"
COMPILE_IMAGE="$(claw_ensure_rust_compile_image "$ROOT" "$CONTAINER_CLI" "$REG")"

# --- [3] linux compile once ---
stage "3/6" "linux compile once"
export CLAW_CONTAINER_RUNTIME=docker
export CLAW_LINUX_COMPILE_CI=1
export CLAW_RUST_COMPILE_IMAGE="$COMPILE_IMAGE"
export CONTAINER_BASE_REGISTRY="$REG"
CN_FLAG=0
claw_cn_mirror_enabled && CN_FLAG=1
claw_linux_compile_release "$ROOT" "$CONTAINER_CLI" "$COMPILE_IMAGE" "$CN_FLAG"

push_package() {
  local package="$1"
  local dockerfile="$2"
  local -a build_args=()
  local image
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
  "$ROOT/deploy/stack/lib/container-build.sh" docker "${dockerfile}" \
    --platform "${PLATFORM}" \
    "${build_args[@]}" \
    -t "${image}:${RELEASE_TAG}" \
    -t "${image}:sha-${SHA12}" \
    -t "${image}:latest"
  skopeo_push_tags "${image}:${RELEASE_TAG}" "${image}"
}

# --- [4] core images ---
stage "4/6" "build+push core images (skopeo only)"
push_package claw-code deploy/stack/Containerfile.gateway-rs.prebuilt
push_package claw-gateway-worker deploy/stack/Containerfile.gateway-worker.prebuilt
push_package claw-gateway-playground deploy/stack/Containerfile.gateway-playground
push_package claw-gateway-worker-relaxed deploy/stack/Containerfile.gateway-worker-relaxed

# --- [5] engine workers ---
stage "5/6" "build+push engine workers"
STRICT_WORKER="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-worker:${RELEASE_TAG}"
export CONTAINER_BASE_REGISTRY="$REG"
bash "$ROOT/deploy/neuro-harness/build-worker-images.sh" "$STRICT_WORKER" "$RELEASE_TAG"
for package in claw-gateway-worker-opencode claw-gateway-worker-appserver; do
  local_img="${package}:${RELEASE_TAG}"
  image="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/${package}"
  docker tag "${local_img}" "${image}:${RELEASE_TAG}"
  docker tag "${local_img}" "${image}:sha-${SHA12}"
  docker tag "${local_img}" "${image}:latest"
  skopeo_push_tags "${image}:${RELEASE_TAG}" "${image}"
done

# --- [6] verify ---
stage "6/6" "verify images"
docker run --rm "${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-playground:${RELEASE_TAG}" \
  sh -c 'test -f /app/admin-dist/index.html && test -n "$(ls /app/admin-dist/assets/*.js 2>/dev/null)"'
docker run --rm --entrypoint sh \
  "${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-worker-relaxed:${RELEASE_TAG}" \
  -c 'command -v curl && command -v git && command -v python3'

echo "published ${NEXUS_NS}/{claw-code,claw-gateway-worker,claw-gateway-worker-relaxed,claw-gateway-worker-opencode,claw-gateway-worker-appserver,claw-gateway-playground}:${RELEASE_TAG}"
