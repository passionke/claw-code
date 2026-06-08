#!/usr/bin/env bash
# CI single-host: render .env from env → build → retag local/*:release-* → up --release (no ACR pull).
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"

TAG="${CLAW_RELEASE_TAG:-release-${CI_COMMIT_SHORT_SHA:-local}}"
export CLAW_RELEASE_SKIP_PULL="${CLAW_RELEASE_SKIP_PULL:-1}"
PREFIX="${CLAW_IMAGE_PREFIX:-local}"

cd "${REPO_ROOT}"

"${LIB_DIR}/render-env-from-ci.sh"

echo "==> build images (tag=local)"
"${LIB_DIR}/build.sh" --no-clean local

echo "==> retag → ${PREFIX}/claw-code:${TAG} (+ worker, playground)"
docker tag claw-gateway-rs:local "${PREFIX}/claw-code:${TAG}"
docker tag claw-gateway-worker:local "${PREFIX}/claw-gateway-worker:${TAG}"
docker tag claw-gateway-playground:local "${PREFIX}/claw-gateway-playground:${TAG}"

echo "==> up --release ${TAG} (CLAW_RELEASE_SKIP_PULL=${CLAW_RELEASE_SKIP_PULL})"
"${LIB_DIR}/up.sh" --release "${TAG}"

echo "==> verify + connectivity"
"${LIB_DIR}/claw-stack-verify.sh"
"${LIB_DIR}/check-connectivity.sh"

echo "==> CI release deploy ok (${TAG})"
