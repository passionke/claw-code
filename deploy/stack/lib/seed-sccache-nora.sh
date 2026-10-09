#!/usr/bin/env bash
# Publish sccache OCI to Nora after local seed. Author: kejiqing
# Usage:
#   NEXUS_USER=nora NEXUS_PASSWORD=... bash deploy/stack/lib/seed-sccache-nora.sh
# Optional: SCCACHE_VERSION=v0.10.0 REGION=china|else
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NEXUS_PUSH_REGISTRY="${NEXUS_PUSH_REGISTRY:-nora.home.passionke.top}"
NEXUS_NS="${NEXUS_NS:-passionke}"
SCCACHE_VERSION="${SCCACHE_VERSION:-v0.10.0}"
REGION="${REGION:-china}"
: "${NEXUS_USER:?NEXUS_USER is required}"
: "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required}"

export SCCACHE_VERSION REGION
bash "${ROOT}/deploy/stack/lib/seed-sccache-local.sh"

LOCAL_TAG="sccache:${SCCACHE_VERSION}"
IMAGE="${NEXUS_PUSH_REGISTRY}/${NEXUS_NS}/sccache:${SCCACHE_VERSION}"

echo "$NEXUS_PASSWORD" | docker login "$NEXUS_PUSH_REGISTRY" -u "$NEXUS_USER" --password-stdin
docker tag "${LOCAL_TAG}" "${IMAGE}"
echo "==> push ${IMAGE}"
docker push "${IMAGE}"
echo "seeded ${IMAGE}"
