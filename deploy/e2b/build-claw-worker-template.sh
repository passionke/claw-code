#!/usr/bin/env bash
# Build e2b template claw-worker-v1 (E2B SDK, cn-beijing). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
fi

VENV="${CLAW_E2B_VENV:-${ROOT_DIR}/.venv-fc}"
PY="${VENV}/bin/python3"

if [[ ! -x "${PY}" ]]; then
  echo "==> create venv ${VENV}" >&2
  python3 -m venv "${VENV}"
  "${VENV}/bin/pip" install -q e2b==2.26.0 e2b-code-interpreter python-dotenv
fi

# Recommended production path: builder from Hangzhou ACR worker image → fc-e2b-registry Beijing.
# Requires CLAW_E2B_TEMPLATE_DEST_USERNAME/PASSWORD (FC-managed ACR EE push creds). See README.md.
export CLAW_E2B_TEMPLATE_BUILD_STRATEGY="${CLAW_E2B_TEMPLATE_BUILD_STRATEGY:-from_image}"
export CLAW_E2B_TEMPLATE_BUILD_MODE="${CLAW_E2B_TEMPLATE_BUILD_MODE:-builder}"
export CLAW_E2B_TEMPLATE_SKIP_CACHE="${CLAW_E2B_TEMPLATE_SKIP_CACHE:-0}"
export CLAW_E2B_TEMPLATE_FROM_IMAGE="${CLAW_E2B_TEMPLATE_FROM_IMAGE:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12}"
export CLAW_E2B_TEMPLATE_DEST_IMAGE_REF="${CLAW_E2B_TEMPLATE_DEST_IMAGE_REF:-fc-e2b-registry.cn-beijing.cr.aliyuncs.com/passionke/claw-worker:release-v1.6.12}"
export CLAW_E2B_TEMPLATE="${CLAW_E2B_TEMPLATE:-claw-worker-v1-prod}"

exec "${PY}" "${ROOT_DIR}/deploy/e2b/build-claw-worker-template.py"
