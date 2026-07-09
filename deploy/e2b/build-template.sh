#!/usr/bin/env bash
# Build claw-worker image for e2b sandbox template publish. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

WORKER_IMAGE="${CLAW_E2B_WORKER_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12}"
FC_LAYERED_IMAGE="${CLAW_E2B_LAYERED_IMAGE:-claw-worker-fc:local}"
CONTAINERFILE="${CLAW_E2B_WORKER_CONTAINERFILE:-deploy/stack/Containerfile.gateway-worker}"
FC_CONTAINERFILE="${CLAW_E2B_LAYERED_CONTAINERFILE:-deploy/e2b/Containerfile.claw-worker-e2b}"
FC_DIR="${ROOT_DIR}/deploy/e2b"

usage() {
  cat <<EOF
Usage: $0 [worker-image|fc-layered|template]

  worker-image  Build claw-gateway-worker (default CI image) locally
  fc-layered    Build e2b code-interpreter base + claw only (needs claw in deploy/e2b/)
  template      Run E2B Template.build (see build-claw-worker-template.sh)

Env: CLAW_E2B_WORKER_IMAGE, CLAW_E2B_LAYERED_IMAGE, CLAW_E2B_TEMPLATE, CLAW_E2B_TEMPLATE_DEST_* (see README.md)
EOF
}

cmd="${1:-worker-image}"
case "${cmd}" in
  worker-image)
    echo "==> build worker image from ${CONTAINERFILE}"
    podman build -f "${CONTAINERFILE}" -t "${WORKER_IMAGE}" .
    ;;
  fc-layered)
    if [[ ! -f "${FC_DIR}/claw" ]]; then
      echo "==> extract claw from ${WORKER_IMAGE}"
      podman pull "${WORKER_IMAGE}" 2>/dev/null || true
      cid="$(podman create "${WORKER_IMAGE}")"
      podman cp "${cid}:/usr/local/bin/claw" "${FC_DIR}/claw"
      podman rm -f "${cid}" >/dev/null
      chmod +x "${FC_DIR}/claw"
    fi
    echo "==> build e2b layered image ${FC_LAYERED_IMAGE}"
    podman build -f "${FC_CONTAINERFILE}" -t "${FC_LAYERED_IMAGE}" "${FC_DIR}"
    podman tag "${FC_LAYERED_IMAGE}" \
      "fc-e2b-registry.cn-beijing.cr.aliyuncs.com/passionke/claw-worker:release-v1.6.12"
    ;;
  template)
    exec "${ROOT_DIR}/deploy/e2b/build-claw-worker-template.sh"
    ;;
  -h|--help|help)
    usage
    exit 0
    ;;
  *)
    echo "unknown command: ${cmd}" >&2
    usage >&2
    exit 1
    ;;
esac

cat <<EOF

Next: publish e2b template (cn-beijing only)
  1. Set CLAW_E2B_TEMPLATE_DEST_USERNAME/PASSWORD (FC-managed fc-e2b-registry push creds)
  2. ./deploy/e2b/build-template.sh template
  3. e2b console: template NAS volume name = CLAW_E2B_NAS_VOLUME_NAME
  4. .env: CLAW_E2B_TEMPLATE=<template name>

See deploy/e2b/README.md

EOF
