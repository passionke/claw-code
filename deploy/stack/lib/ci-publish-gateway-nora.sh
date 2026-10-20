#!/usr/bin/env bash
# Author: kejiqing
#
# Nora Gateway-only publish (claw-code). Never builds playground / worker / engines.
# Formal v*: push :vX.Y.Z only. Fast sha-*: push :sha-* + :latest.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
bash "$ROOT/deploy/verify-release-boundaries.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/rust-compile-image.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/linux-compile.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/release-images.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/rust-version.env"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/ci-publish-common-nora.sh"

claw_nora_publish_init_env
claw_nora_publish_preflight

stage "2/4" "ensure rust compile image"
COMPILE_IMAGE="$(claw_ensure_rust_compile_image "$ROOT" "$CONTAINER_CLI" "$REG")"

stage "3/4" "linux compile http-gateway-rs"
export CLAW_CONTAINER_RUNTIME=docker
export CLAW_LINUX_COMPILE_CI=1
export CLAW_RUST_COMPILE_IMAGE="$COMPILE_IMAGE"
export CONTAINER_BASE_REGISTRY="$REG"
export CLAW_LINUX_RELEASE_BINS="http-gateway-rs"
CN_FLAG=0
claw_cn_mirror_enabled && CN_FLAG=1
claw_linux_compile_release "$ROOT" "$CONTAINER_CLI" "$COMPILE_IMAGE" "$CN_FLAG"

stage "4/4" "build+push claw-code (skopeo → Nora + ACR)"
NORA_IMAGE="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-code"
ACR_IMAGE="${ACR_REGISTRY}/claw-code"
echo "==> build ${NORA_IMAGE}:${RELEASE_TAG} (also → ${ACR_IMAGE})"
"$ROOT/deploy/stack/lib/container-build.sh" docker deploy/stack/Containerfile.gateway-rs.prebuilt \
  --platform "${PLATFORM}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_IMAGE}" \
  --build-arg "CLAW_USE_CN_APT_MIRROR=${APT_CN}" \
  -t "${NORA_IMAGE}:${RELEASE_TAG}"
claw_nora_skopeo_push_package "${NORA_IMAGE}:${RELEASE_TAG}" "${NORA_IMAGE}" "${ACR_IMAGE}"

echo "published Nora+ACR claw-code:${RELEASE_TAG}"
echo "  nora: ${NORA_IMAGE}"
echo "  acr:  ${ACR_IMAGE}"
