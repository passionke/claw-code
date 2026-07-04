#!/usr/bin/env bash
# Stage linux claw + ttyd for e2b worker template (copy strategy). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${LIB_DIR}/e2b-worker-arch.sh"

CLAW_BIN="${ROOT_DIR}/deploy/stack/.linux-artifacts/release/claw"
OUT_DIR="${CLAW_E2B_TEMPLATE_COPY_DIR:-${ROOT_DIR}/deploy/stack/.e2b-worker-bins}"
TTYD_VERSION="${CLAW_TTYD_VERSION:-1.7.7}"

WORKER_ARCH="$(claw_e2b_worker_linux_arch)"
TTYD_ASSET="$(claw_e2b_ttyd_asset_name)"
TTYD_URL="https://github.com/tsl0922/ttyd/releases/download/${TTYD_VERSION}/ttyd.${TTYD_ASSET}"

stage_usage() {
  cat <<EOF
Usage: stage-e2b-worker-bins.sh

Writes claw + ttyd (linux/${WORKER_ARCH}) into:
  ${OUT_DIR}/

Env:
  CLAW_E2B_WORKER_ARCH         amd64 | arm64 (default amd64 for self-hosted e2b)
  CLAW_E2B_TEMPLATE_COPY_DIR   output dir
  CLAW_TTYD_VERSION            default ${TTYD_VERSION}

Prereq: deploy/stack/.linux-artifacts/release/claw (see gateway.sh e2b-worker-deploy)
EOF
}

require_linux_elf() {
  local path="$1"
  local label="$2"
  if [[ ! -f "${path}" ]]; then
    echo "error: missing ${label} at ${path}" >&2
    return 1
  fi
  local probe
  probe="$(file -b "${path}")"
  echo "  ${label}: ${probe}"
  if ! claw_e2b_elf_arch_ok "${probe}" "${WORKER_ARCH}"; then
    echo "error: ${label} is not linux/${WORKER_ARCH} ELF (${probe})" >&2
    echo "hint: ./deploy/stack/gateway.sh e2b-worker-deploy  (CLAW_E2B_WORKER_ARCH=${WORKER_ARCH})" >&2
    return 1
  fi
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  stage_usage
  exit 0
fi

mkdir -p "${OUT_DIR}"

echo "==> stage e2b worker bins (linux/${WORKER_ARCH}) → ${OUT_DIR}"
require_linux_elf "${CLAW_BIN}" claw

cp -f "${CLAW_BIN}" "${OUT_DIR}/claw"
chmod +x "${OUT_DIR}/claw"

ttyd_dest="${OUT_DIR}/ttyd"
if [[ -f "${ttyd_dest}" ]]; then
  if require_linux_elf "${ttyd_dest}" ttyd 2>/dev/null; then
    echo "  ttyd: reuse existing ${ttyd_dest}"
  else
    rm -f "${ttyd_dest}"
  fi
fi
if [[ ! -f "${ttyd_dest}" ]]; then
  echo "  ttyd: curl ${TTYD_URL}"
  curl -fsSL -o "${ttyd_dest}" "${TTYD_URL}"
  chmod +x "${ttyd_dest}"
  require_linux_elf "${ttyd_dest}" ttyd
fi

echo "OK: ${OUT_DIR}/claw ${OUT_DIR}/ttyd (linux/${WORKER_ARCH})"
