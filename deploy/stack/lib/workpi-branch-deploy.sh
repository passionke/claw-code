#!/usr/bin/env bash
# workPi: deploy from branch CI ACR images only (no local rust compile).
# Usage: ./deploy/stack/lib/workpi-branch-deploy.sh <branch-tag> [--local-gateway-build]
# Example: ./deploy/stack/lib/workpi-branch-deploy.sh branch-feat-router-finish
#
# Default path (fast):
#   1) e2b-worker-deploy --from-ci-image <tag>   # claw from ACR → e2b templates
#   2) gateway.sh up --release <tag>            # http-gateway-rs from ACR claw-code
#
# Opt-in `--local-gateway-build` keeps the old arm64 cargo path (slow; avoid).
# Author: kejiqing
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
GATEWAY="${REPO_ROOT}/deploy/stack/gateway.sh"

TAG="${1:?usage: $0 <branch-tag> [--local-gateway-build]}"
LOCAL_GATEWAY_BUILD=0
shift || true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --local-gateway-build) LOCAL_GATEWAY_BUILD=1 ;;
    --skip-gateway-build)
      echo "error: --skip-gateway-build removed; default already skips local compile" >&2
      echo "  use: $0 ${TAG}   # ACR up --release" >&2
      exit 2
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

echo "==> workPi branch deploy: CI tag=${TAG} (ACR only; no local package by default)"
echo "    repo=${REPO_ROOT}"
echo "    CLAW_IMAGE_REGISTRY=${CLAW_IMAGE_REGISTRY}"

echo "==> 1/2 e2b-worker-deploy --from-ci-image ${TAG}"
"${GATEWAY}" e2b-worker-deploy --from-ci-image "${TAG}"

if [[ "${LOCAL_GATEWAY_BUILD}" -eq 1 ]]; then
  echo "==> 2/2 local arm64 gateway build + restart (SLOW; prefer ACR)"
  "${GATEWAY}" build local
  "${GATEWAY}" restart
else
  echo "==> 2/2 gateway up --release ${TAG} (pull ACR claw-code; no cargo)"
  "${GATEWAY}" up --release "${TAG}"
fi

echo "==> done. verify:"
echo "    curl -sS http://127.0.0.1:\${GATEWAY_HOST_PORT:-18088}/healthz | jq ."
echo "    ${GATEWAY} check"
echo "    expect deployImageTag / image pin = ${TAG}"
