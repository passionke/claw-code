#!/usr/bin/env bash
# Wait for Nora Gateway images for TAG, then up --release on neurogate host.
# Caller (home-neurogate-*.yml) already waited for Jenkins. Author: kejiqing
# TAG: formal vX.Y.Z | fast :latest (per-image pin after path-gated Jobs).
set -euo pipefail

TAG="${1:-}"
if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && "$TAG" != "latest" ]]; then
  echo "usage: $0 vX.Y.Z|latest" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
PREFIX="${CLAW_IMAGE_PREFIX:-nora.home.passionke.top/passionke}"
REGISTRY_HOST="${PREFIX%%/*}"
WAIT_SECS="${CLAW_NORA_IMAGE_WAIT_SECS:-3600}"
POLL_SECS="${CLAW_NORA_IMAGE_POLL_SECS:-30}"
DOCKER_CONFIG_JSON="${CLAW_DOCKER_CONFIG:-${ROOT}/deploy/stack/.host-secrets/docker-config.json}"
DOCKER_CFG_DIR=""
cleanup() {
  if [[ -n "${DOCKER_CFG_DIR}" && -d "${DOCKER_CFG_DIR}" ]]; then
    rm -rf "${DOCKER_CFG_DIR}"
  fi
}
trap cleanup EXIT

need_images=(
  "${PREFIX}/claw-code:${TAG}"
  "${PREFIX}/claw-gateway-playground:${TAG}"
)

echo "==> neurogate deploy tag=${TAG} prefix=${PREFIX} root=${ROOT}"

if [[ -f "$DOCKER_CONFIG_JSON" ]]; then
  DOCKER_CFG_DIR="$(mktemp -d)"
  cp "$DOCKER_CONFIG_JSON" "${DOCKER_CFG_DIR}/config.json"
  export DOCKER_CONFIG="${DOCKER_CFG_DIR}"
fi

image_ready() {
  local ref="$1"
  if command -v skopeo >/dev/null 2>&1 && [[ -f "$DOCKER_CONFIG_JSON" ]]; then
    skopeo inspect --authfile "$DOCKER_CONFIG_JSON" "docker://${ref}" >/dev/null 2>&1
    return $?
  fi
  docker manifest inspect "$ref" >/dev/null 2>&1
}

echo "==> wait Nora images (timeout=${WAIT_SECS}s) — Jenkins pack for this tag must already be done or in flight"
deadline=$((SECONDS + WAIT_SECS))
while true; do
  missing=()
  for img in "${need_images[@]}"; do
    if image_ready "$img"; then
      echo "  ready: ${img}"
    else
      missing+=("$img")
    fi
  done
  if [[ ${#missing[@]} -eq 0 ]]; then
    break
  fi
  if (( SECONDS >= deadline )); then
    echo "error: timed out waiting for Nora images (Jenkins gateway/playground Jobs):" >&2
    printf '  - %s\n' "${missing[@]}" >&2
    exit 1
  fi
  echo "  waiting (${#missing[@]} missing), sleep ${POLL_SECS}s …"
  sleep "$POLL_SECS"
done

cd "$ROOT"
if git remote get-url gitea >/dev/null 2>&1; then
  git fetch --tags --force gitea
  git fetch --force gitea || true
else
  git fetch --tags --force origin
  git fetch --force origin || true
fi
if [[ "$TAG" == "latest" ]]; then
  # Fast path: images pinned to :latest; stay on the Actions-checked-out commit. Author: kejiqing
  git log -1 --oneline || true
elif [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  git checkout -f "tags/${TAG}" 2>/dev/null || git checkout -f "${TAG}"
  git describe --tags --exact-match HEAD
fi

export CLAW_IMAGE_PREFIX="$PREFIX"

echo "==> gateway.sh up --release ${TAG}"
./deploy/stack/gateway.sh up --release "$TAG"
./deploy/stack/gateway.sh verify
echo "==> neurogate deploy OK tag=${TAG} registry=${REGISTRY_HOST}"
