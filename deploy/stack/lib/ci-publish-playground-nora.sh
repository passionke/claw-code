#!/usr/bin/env bash
# Author: kejiqing
#
# Nora Admin/playground-only publish (claw-gateway-playground). No cargo.
# Formal v*: push :vX.Y.Z only. Fast sha-*: push :sha-* + :latest.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
bash "$ROOT/deploy/verify-release-boundaries.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/compose-include.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/claw-region.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/release-images.sh"
# shellcheck source=/dev/null
source "$ROOT/deploy/stack/lib/ci-publish-common-nora.sh"

claw_nora_publish_init_env
claw_nora_publish_preflight

stage "2/3" "build+push claw-gateway-playground (skopeo → Nora + ACR)"
NORA_IMAGE="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/claw-gateway-playground"
ACR_IMAGE="${ACR_REGISTRY}/claw-gateway-playground"
echo "==> build ${NORA_IMAGE}:${RELEASE_TAG} (also → ${ACR_IMAGE})"
"$ROOT/deploy/stack/lib/container-build.sh" docker deploy/stack/Containerfile.gateway-playground \
  --platform "${PLATFORM}" \
  --build-arg "DEBIAN_BASE_IMAGE=${DEBIAN_IMAGE}" \
  --build-arg "NODE_BASE_IMAGE=${NODE_IMAGE}" \
  --build-arg "CLAW_USE_CN_APT_MIRROR=${APT_CN}" \
  -t "${NORA_IMAGE}:${RELEASE_TAG}"
claw_nora_skopeo_push_package "${NORA_IMAGE}:${RELEASE_TAG}" "${NORA_IMAGE}" "${ACR_IMAGE}"

stage "3/3" "verify playground admin assets"
docker run --rm "${NORA_IMAGE}:${RELEASE_TAG}" \
  sh -c 'test -f /app/admin-dist/index.html && test -n "$(ls /app/admin-dist/assets/*.js 2>/dev/null)"'

echo "published Nora+ACR claw-gateway-playground:${RELEASE_TAG}"
echo "  nora: ${NORA_IMAGE}"
echo "  acr:  ${ACR_IMAGE}"
