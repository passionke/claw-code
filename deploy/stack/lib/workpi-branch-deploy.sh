#!/usr/bin/env bash
# workPi: e2b shell register + local arm64 gateway.
# ONE path. Author: kejiqing. Do not invent forks.
# Usage: ./deploy/stack/lib/workpi-branch-deploy.sh <branch-tag> [--skip-gateway-build]
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

cd "${REPO_ROOT}"
if [[ ! -f .env ]]; then
  echo "error: ${REPO_ROOT}/.env missing" >&2
  exit 1
fi

export CLAW_IMAGE_REGISTRY="${CLAW_IMAGE_REGISTRY:-acr}"
export CLAW_IMAGE_RELEASE_TAG="${TAG}"
export RELEASE_TAG="${TAG}"
rm -f "${REPO_ROOT}/deploy/stack/.claw-image-release.env"

echo "==> workPi: e2b-register shell + local gateway (tag=${TAG})"
echo "==> 1/3 publish.sh e2b-register"
chmod +x "${REPO_ROOT}/deploy/pack/publish.sh"
"${REPO_ROOT}/deploy/pack/publish.sh" e2b-register

if [[ "${SKIP_GATEWAY_BUILD}" -eq 0 ]]; then
  echo "==> 2/3 gateway build local (arm64)"
  "${GATEWAY}" build local
else
  echo "==> 2/3 skip gateway build"
fi

echo "==> 3/3 gateway restart"
"${GATEWAY}" restart
echo "==> done. Apply CLI pins in Admin → Worker CLI 版本 if needed."
