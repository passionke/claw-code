#!/usr/bin/env bash
# One-time / upgrade seed: publish static sccache image to Nora (OCI).
# Internal builds then COPY --from that image — no GitHub in the hot path.
# Usage:
#   NEXUS_USER=nora NEXUS_PASSWORD=... bash deploy/stack/lib/seed-sccache-nora.sh
# Optional: SCCACHE_VERSION=v0.10.0 REGION=china|else
# Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NEXUS_PUSH_REGISTRY="${NEXUS_PUSH_REGISTRY:-nora.home.passionke.top}"
NEXUS_NS="${NEXUS_NS:-passionke}"
SCCACHE_VERSION="${SCCACHE_VERSION:-v0.10.0}"
REGION="${REGION:-china}"
: "${NEXUS_USER:?NEXUS_USER is required}"
: "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required}"

ARCH_TRIPLE="${SCCACHE_ARCH:-x86_64-unknown-linux-musl}"
TGZ_NAME="sccache-${SCCACHE_VERSION}-${ARCH_TRIPLE}.tar.gz"
if [[ "$REGION" == "china" ]]; then
  TGZ_URL="${SCCACHE_TGZ_URL:-https://ghfast.top/https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/${TGZ_NAME}}"
else
  TGZ_URL="${SCCACHE_TGZ_URL:-https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/${TGZ_NAME}}"
fi

IMAGE="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/sccache:${SCCACHE_VERSION}"
CTX="${ROOT}/deploy/stack/.build-ctx/sccache-seed"
rm -rf "${CTX}"
mkdir -p "${CTX}"
cp "${ROOT}/deploy/stack/Containerfile.sccache" "${CTX}/Containerfile"

echo "==> fetch ${TGZ_URL}"
curl --http1.1 -fL --connect-timeout 20 --max-time 180 --retry 3 --retry-delay 2 \
  -o "${CTX}/${TGZ_NAME}" "${TGZ_URL}"
tar xz -C "${CTX}" -f "${CTX}/${TGZ_NAME}"
cp "${CTX}/sccache-${SCCACHE_VERSION}-${ARCH_TRIPLE}/sccache" "${CTX}/sccache"
chmod +x "${CTX}/sccache"
"${CTX}/sccache" --version

echo "$NEXUS_PASSWORD" | docker login "$NEXUS_PUSH_REGISTRY" -u "$NEXUS_USER" --password-stdin
echo "==> build ${IMAGE}"
DOCKER_BUILDKIT="${DOCKER_BUILDKIT:-0}" docker build -f "${CTX}/Containerfile" -t "${IMAGE}" "${CTX}"
echo "==> push ${IMAGE}"
docker push "${IMAGE}"
echo "seeded ${IMAGE}"
