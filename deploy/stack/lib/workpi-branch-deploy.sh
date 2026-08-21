#!/usr/bin/env bash
# workPi: ACR branch worker (e2b) + local arm64 gateway (host is aarch64; ACR claw-code is amd64).
# Usage: ./deploy/stack/lib/workpi-branch-deploy.sh <branch-tag> [--skip-gateway-build]
# Example: ./deploy/stack/lib/workpi-branch-deploy.sh branch-feat-router-finish
#
# Path:
#   1) e2b-worker-deploy --from-ci-image <tag>  # amd64 claw from ACR → e2b templates
#   2) gateway.sh build local                  # arm64 http-gateway-rs (required on workPi)
#   3) gateway.sh restart
#
# workPi cannot exec ACR amd64 gateway (no usable qemu/binfmt). Author: kejiqing
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
GATEWAY="${REPO_ROOT}/deploy/stack/gateway.sh"

TAG="${1:?usage: $0 <branch-tag> [--skip-gateway-build]}"
SKIP_GATEWAY_BUILD=0
shift || true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-gateway-build) SKIP_GATEWAY_BUILD=1 ;;
    --local-gateway-build)
      echo "note: --local-gateway-build is the default on workPi; ignoring" >&2
      ;;
    *) echo "error: unknown arg: $1" >&2; exit 2 ;;
  esac
  shift || true
done

if [[ "${TAG}" == release-v* ]]; then
  echo "error: use release tag flow for ${TAG}; this script is for branch-* / dev-* CI tags" >&2
  exit 1
fi

cd "${REPO_ROOT}"
if [[ ! -f .env ]]; then
  echo "error: ${REPO_ROOT}/.env missing (copy .env.workpi on workPi)" >&2
  exit 1
fi

export CLAW_IMAGE_REGISTRY="${CLAW_IMAGE_REGISTRY:-acr}"
export CLAW_IMAGE_RELEASE_TAG="${TAG}"

# Drop sticky --release pin so compose uses claw-gateway-rs:local (arm64), not ACR amd64.
rm -f "${REPO_ROOT}/deploy/stack/.claw-image-release.env"

echo "==> workPi branch deploy: CI tag=${TAG}"
echo "    worker: ACR claw-code → e2b templates"
echo "    gateway: local arm64 (ACR amd64 cannot exec on aarch64)"
echo "    repo=${REPO_ROOT}"

echo "==> 1/3 e2b-worker-deploy --from-ci-image ${TAG}"
"${GATEWAY}" e2b-worker-deploy --from-ci-image "${TAG}"

if [[ "${SKIP_GATEWAY_BUILD}" -eq 0 ]]; then
  echo "==> 2/3 gateway build local (arm64)"
  "${GATEWAY}" build local
else
  echo "==> 2/3 skip gateway build (--skip-gateway-build)"
fi

echo "==> 3/3 gateway restart (new PG buildId + local gateway)"
"${GATEWAY}" restart

echo "==> done. verify:"
echo "    curl -sS http://127.0.0.1:\${GATEWAY_HOST_PORT:-18088}/healthz | jq ."
echo "    ${GATEWAY} check"
echo "    expect e2b buildId from ${TAG}; gateway image claw-gateway-rs:local"
