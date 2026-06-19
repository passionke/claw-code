#!/usr/bin/env bash
# Build claw-worker image for FC sandbox template publish. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

IMAGE="${CLAW_FC_WORKER_IMAGE:-claw-gateway-worker:fc}"
CONTAINERFILE="${CLAW_FC_WORKER_CONTAINERFILE:-deploy/stack/Containerfile.gateway-worker}"

echo "==> build worker image ${IMAGE} from ${CONTAINERFILE}"
podman build -f "${CONTAINERFILE}" -t "${IMAGE}" .

cat <<EOF

Next (FC console, cn-beijing only):
  1. Push ${IMAGE} to ACR / FC-accessible registry
  2. Create sandbox function template (e.g. claw-worker-v1) from that image
  3. Configure VPC + NAS dynamic mount (see deploy/fc-sandbox/README.md)
  4. Set CLAW_FC_TEMPLATE=claw-worker-v1 in repo root .env

EOF
