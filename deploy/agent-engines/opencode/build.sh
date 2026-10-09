#!/usr/bin/env bash
# Build the opencode Agent engine raw artifact. Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
VERSION="${OPENCODE_VERSION:-1.18.34}"
ARCH="${TARGETARCH:-amd64}"
OUTPUT="${1:-${ROOT}/deploy/agent-engines/dist/opencode-${VERSION}-${ARCH}.tar.gz}"
IMAGE="claw-agent-opencode-build:${VERSION}-${ARCH}-$$"
STAGE="$(mktemp -d)"
CID=""
cleanup() {
  if [[ -n "$CID" ]]; then
    docker rm -f "$CID" >/dev/null 2>&1 || true
  fi
  docker rmi "$IMAGE" >/dev/null 2>&1 || true
  rm -rf "$STAGE"
}
trap cleanup EXIT

docker build \
  --platform "linux/${ARCH}" \
  --build-arg "TARGETARCH=${ARCH}" \
  --build-arg "OPENCODE_VERSION=${VERSION}" \
  --build-arg "NPM_REGISTRY=${NPM_REGISTRY:-https://registry.npmjs.org}" \
  -t "$IMAGE" \
  -f "$ROOT/deploy/agent-engines/opencode/Containerfile" \
  "$ROOT"
CID="$(docker create "$IMAGE")"
mkdir -p "$STAGE/usr/local/lib/neuro-engines/opencode/bin"
docker cp "${CID}:/engine/package/bin/opencode" \
  "$STAGE/usr/local/lib/neuro-engines/opencode/bin/opencode"
chmod 0755 "$STAGE/usr/local/lib/neuro-engines/opencode/bin/opencode"
mkdir -p "$(dirname "$OUTPUT")"
tar -C "$STAGE" -czf "$OUTPUT" usr
echo "$OUTPUT"
