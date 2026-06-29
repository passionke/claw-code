#!/usr/bin/env bash
# One-shot: 234 ECS self-hosted e2b + NFS workspace + gateway stack. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

NAS_SERVER="${NAS_BASE_URL:-10.8.0.8}"
NAS_EXPORT="${CLAW_FC_NAS_EXPORT:-/}"
MOUNT_POINT="${CLAW_NAS_HOST_MOUNT:-/mnt/nas0}"

echo "==> mount self-hosted NFS ${NAS_SERVER}:/ -> ${MOUNT_POINT}"
mkdir -p "${MOUNT_POINT}"
if ! mountpoint -q "${MOUNT_POINT}" 2>/dev/null; then
  mount -t nfs -o vers=4.2,_netdev "${NAS_SERVER}:/" "${MOUNT_POINT}"
fi
touch "${MOUNT_POINT}/.claw-probe" && rm -f "${MOUNT_POINT}/.claw-probe"

echo "==> python e2b SDK"
if ! python3 -c "import e2b" 2>/dev/null; then
  python3 -m pip install -q e2b e2b-code-interpreter
fi

echo "==> build claw-worker template on e2bserver (FROM CI worker image tag)"
export CLAW_E2B_TEMPLATE_BUILD_STRATEGY="${CLAW_E2B_TEMPLATE_BUILD_STRATEGY:-from_image}"
export CLAW_FC_WORKER_IMAGE="${CLAW_FC_WORKER_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.14}"
export E2B_API_KEY="${CLAW_FC_API_KEY:-e2b_53ae1fed82754c17ad8077fbc8bcdd90}"
export E2B_API_URL="${CLAW_FC_API_URL:-http://10.8.0.1:3000}"
export E2B_SANDBOX_URL="${CLAW_E2B_SANDBOX_URL:-http://10.8.0.1:3002}"
export E2B_DOMAIN="${CLAW_FC_DOMAIN:-supone.top}"
python3 ./deploy/fc-sandbox/build-claw-worker-selfhosted.py

echo "==> gateway stack"
./deploy/stack/gateway.sh up "$@"

echo "OK: self-hosted stack up (NAS=${MOUNT_POINT}, e2b=${E2B_API_URL})"
