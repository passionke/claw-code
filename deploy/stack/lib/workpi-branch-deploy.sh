#!/usr/bin/env bash
# workPi: rebuild e2b templates from branch CI image + local arm64 gateway, then restart.
# Usage: ./deploy/stack/lib/workpi-branch-deploy.sh <branch-tag> [--skip-gateway-build]
# Example: ./deploy/stack/lib/workpi-branch-deploy.sh branch-feat-delegate-output-yield
# Author: kejiqing
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

echo "==> workPi branch deploy: CI tag=${TAG} (ACR claw-code + local gateway)"
echo "    repo=${REPO_ROOT}"

echo "==> 1/3 e2b-worker-deploy --from-ci-image ${TAG}"
"${GATEWAY}" e2b-worker-deploy --from-ci-image "${TAG}"

if [[ "${SKIP_GATEWAY_BUILD}" -eq 0 ]]; then
  echo "==> 2/3 gateway build local (arm64 http-gateway-rs + fan-in changes)"
  "${GATEWAY}" build local
else
  echo "==> 2/3 skip gateway build (--skip-gateway-build)"
fi

echo "==> 3/3 gateway restart (pick up new PG buildId + local gateway)"
"${GATEWAY}" restart

echo "==> done. verify:"
echo "    curl -sS http://127.0.0.1:\${GATEWAY_HOST_PORT:-18088}/healthz | jq ."
echo "    ${GATEWAY} check"
