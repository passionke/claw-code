#!/usr/bin/env bash
# Bootstrap e2b core templates from an existing ACR/CI image tag (no local amd64 rustc).
# Used by Gateway Admin POST /v1/gateway/bootstrap/publish-templates. Author: kejiqing
#
# Runs inside gateway container: NO nested podman/docker (userns fails).
# Extracts claw/claude-tap from ACR via registry HTTP, then e2b debian+COPY.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
E2B_DIR="${ROOT_DIR}/deploy/e2b"
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/claw-region.sh"
claw_region_load
# shellcheck disable=SC1091
[[ -f "${ROOT_DIR}/.env" ]] && set -a && source "${ROOT_DIR}/.env" && set +a
claw_region_load
# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/release-images.sh"

TAG="${1:-${CLAW_IMAGE_RELEASE_TAG:-}}"
if [[ -z "${TAG}" ]]; then
  echo "usage: $0 <release-or-branch-tag>" >&2
  echo "hint: tag is produced by CI/ACR (e.g. release-v1.8.11); this script only publishes to e2b." >&2
  exit 2
fi

claw_apply_release_image_tag "${TAG}"
PREFIX="$(claw_image_registry_prefix_from_env)"
WORKER_IMAGE="${CLAW_E2B_WORKER_IMAGE:-${PREFIX}/claw-gateway-worker:${TAG}}"
RELAXED_IMAGE="${CLAW_E2B_WORKER_RELAXED_IMAGE:-${PREFIX}/claw-gateway-worker-relaxed:${TAG}}"
# Observe binary comes from claw-tap (not debian-bookworm-claw-observe — CI does not push that). Author: kejiqing
TAP_IMAGE="${CLAUDE_TAP_IMAGE:-${PREFIX}/claw-tap:latest}"

# Writable dirs even when repo is mounted :ro into gateway. Author: kejiqing
ART_ROOT="${CLAW_BOOTSTRAP_ARTIFACT_DIR:-/tmp/claw-bootstrap-${TAG}}"
mkdir -p "${ART_ROOT}/venv"
export CLAW_E2B_VENV="${CLAW_E2B_VENV:-${ART_ROOT}/venv}"
export HOME="${CLAW_BOOTSTRAP_HOME:-${ART_ROOT}/home}"
mkdir -p "${HOME}"

# e2bserver rejects claw-gateway-worker / claw-tap as "non-Debian" bases; scripts switch to
# debian:bookworm-slim + COPY binary (registry HTTP extract, no nested podman). Author: kejiqing
export CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image
export CLAW_E2B_WORKER_IMAGE="${WORKER_IMAGE}"
export CLAW_E2B_TEMPLATE_FROM_IMAGE="${WORKER_IMAGE}"
export CLAW_E2B_WORKER_SKIP_LOCAL_BUILD=1
export CLAW_E2B_WORKER_RELAXED_IMAGE="${RELAXED_IMAGE}"
export CLAW_E2B_WORKER_RELAXED_FROM_IMAGE=1
export CLAUDE_TAP_IMAGE="${TAP_IMAGE}"
export CLAW_E2B_OBSERVE_SKIP_LOCAL_BUILD=1
export CLAW_E2B_TEMPLATE_SKIP_VERIFY="${CLAW_E2B_TEMPLATE_SKIP_VERIFY:-1}"
export CLAW_IMAGE_RELEASE_TAG="${TAG}"

export E2B_API_KEY="${E2B_API_KEY:-${CLAW_E2B_API_KEY:-}}"
export E2B_API_URL="${E2B_API_URL:-${CLAW_E2B_API_URL:-}}"
export E2B_SANDBOX_URL="${E2B_SANDBOX_URL:-${CLAW_E2B_SANDBOX_URL:-}}"
export E2B_DOMAIN="${E2B_DOMAIN:-${CLAW_E2B_DOMAIN:-}}"

if [[ -z "${E2B_API_KEY}" || -z "${E2B_API_URL}" ]]; then
  echo "error: set CLAW_E2B_API_KEY and CLAW_E2B_API_URL" >&2
  exit 1
fi

# Prefer existing venv; else create under ART_ROOT (repo may be read-only). Author: kejiqing
VENV_DIR="${CLAW_E2B_VENV}"
PY="${VENV_DIR}/bin/python3"
ensure_venv() {
  local -a pip_extra=()
  if claw_region_is_china; then
    pip_extra=(-i https://pypi.tuna.tsinghua.edu.cn/simple --trusted-host pypi.tuna.tsinghua.edu.cn)
  fi
  if [[ -x "${PY}" ]] && "${PY}" -c "import e2b, psycopg" 2>/dev/null; then
    return 0
  fi
  echo "==> create ${VENV_DIR} (e2b SDK)" >&2
  python3 -m venv "${VENV_DIR}"
  "${PY}" -m pip install -q "${pip_extra[@]}" e2b==2.26.0 e2b-code-interpreter python-dotenv 'psycopg[binary]'
}
ensure_venv

PLATFORM="${CLAW_E2B_TEMPLATE_PLATFORM:-linux/amd64}"
echo "==> bootstrap templates from CI tag=${TAG}" >&2
echo "    worker_image=${WORKER_IMAGE} → debian+COPY claw (no nested podman)" >&2
echo "    relaxed_image=${RELAXED_IMAGE} → debian+COPY claw (no OVS bake)" >&2
echo "    tap_image=${TAP_IMAGE} → debian+COPY claude-tap" >&2
echo "    e2b=${E2B_API_URL} platform=${PLATFORM}" >&2

run_py() {
  echo "" >&2
  echo "======== $(date -Iseconds) $* ========" >&2
  "${PY}" "$@"
}

run_py "${E2B_DIR}/build-claw-worker-selfhosted.py"
run_py "${E2B_DIR}/build-claw-worker-relaxed-selfhosted.py"
run_py "${E2B_DIR}/build-claw-observe-selfhosted.py"

case "${CLAW_E2B_NAS_API:-1}" in
  0|false|no|off|FALSE|NO|OFF)
    echo "==> skip nas-api (CLAW_E2B_NAS_API=0)" >&2
    ;;
  *)
    run_py "${E2B_DIR}/build-claw-nas-api-selfhosted.py"
    ;;
esac

echo "" >&2
echo "OK: bootstrap templates published for tag=${TAG} on ${E2B_API_URL}" >&2
echo "note: relaxed from CI claw may lack built-in OVS; full OVS bake still needs host e2b-worker-deploy" >&2
