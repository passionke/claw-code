#!/usr/bin/env bash
# One-shot: 234 ECS self-hosted e2b + NFS workspace + gateway stack. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

NAS_SERVER="${NAS_BASE_URL:-10.8.0.11}"
NAS_EXPORT="${CLAW_E2B_NAS_EXPORT:-/}"
MOUNT_POINT="${CLAW_NAS_HOST_MOUNT:-/mnt/nas0}"

echo "==> mount self-hosted NFS ${NAS_SERVER}:/ -> ${MOUNT_POINT}"
mkdir -p "${MOUNT_POINT}"
if ! mountpoint -q "${MOUNT_POINT}" 2>/dev/null; then
  mount -t nfs -o vers=4.2,_netdev "${NAS_SERVER}:/" "${MOUNT_POINT}"
fi
touch "${MOUNT_POINT}/.claw-probe" && rm -f "${MOUNT_POINT}/.claw-probe"

echo "==> gateway stack (e2b templates: Admin init / 重打模板, not this script)"
./deploy/stack/gateway.sh up "$@"

echo "OK: self-hosted stack up (NAS=${MOUNT_POINT}, e2b=${E2B_API_URL})"
