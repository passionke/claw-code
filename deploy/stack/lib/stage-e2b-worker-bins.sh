#!/usr/bin/env bash
# Stage linux claw for strict e2b worker template (copy strategy; no ttyd). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${LIB_DIR}/e2b-worker-arch.sh"

CLAW_BIN="${ROOT_DIR}/deploy/stack/.linux-artifacts/release/claw"
OUT_DIR="${CLAW_E2B_TEMPLATE_COPY_DIR:-${ROOT_DIR}/deploy/stack/.e2b-worker-bins}"

WORKER_ARCH="$(claw_e2b_worker_linux_arch)"

stage_usage() {
  cat <<EOF
Usage: stage-e2b-worker-bins.sh

Writes claw only (linux/${WORKER_ARCH}) into:
  ${OUT_DIR}/

Strict solve worker has no ttyd; relaxed template uses build-claw-worker-relaxed-selfhosted.py.

Env:
  CLAW_E2B_WORKER_ARCH         amd64 | arm64 (default amd64 for self-hosted e2b)
  CLAW_E2B_TEMPLATE_COPY_DIR   output dir

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

echo "==> stage strict e2b worker claw (linux/${WORKER_ARCH}) → ${OUT_DIR}"
require_linux_elf "${CLAW_BIN}" claw

cp -f "${CLAW_BIN}" "${OUT_DIR}/claw"
chmod +x "${OUT_DIR}/claw"

rm -f "${OUT_DIR}/ttyd"

echo "OK: ${OUT_DIR}/claw (linux/${WORKER_ARCH}; strict, no ttyd)"
