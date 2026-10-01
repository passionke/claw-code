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
TAP_IMAGE="${CLAUDE_TAP_IMAGE:-${PREFIX}/claw-tap:v0.0.18}"

# Writable dirs even when repo is mounted :ro into gateway. Author: kejiqing
ART_ROOT="${CLAW_BOOTSTRAP_ARTIFACT_DIR:-/tmp/claw-bootstrap-${TAG}}"
mkdir -p "${ART_ROOT}/venv"
export CLAW_E2B_VENV="${CLAW_E2B_VENV:-${ART_ROOT}/venv}"
export HOME="${CLAW_BOOTSTRAP_HOME:-${ART_ROOT}/home}"
mkdir -p "${HOME}"

# e2bserver rejects claw-gateway-worker / claw-tap as "non-Debian" bases; scripts switch to
# debian:bookworm-slim + COPY binary (registry HTTP extract, no nested podman).
# Author: kejiqing
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

# One path: prefer image-baked system python3 (Containerfile installs requirements-e2b-sdk.txt).
# Venv + pip is fallback only — never the Admin-button happy path. Author: kejiqing
VENV_DIR="${CLAW_E2B_VENV}"
SDK_REQ="${E2B_DIR}/requirements-e2b-sdk.txt"
PY=""
export PIP_CACHE_DIR="${ART_ROOT}/pip-cache"
mkdir -p "${PIP_CACHE_DIR}"

sdk_pins_ok() {
  local py="$1"
  "${py}" - "${SDK_REQ}" <<'PY'
import importlib.metadata as metadata
import sys

req = open(sys.argv[1], encoding="utf-8")
wanted = {}
for raw in req:
    line = raw.strip()
    if not line or line.startswith("#") or "==" not in line:
        continue
    name, ver = line.split("==", 1)
    name = name.split("[", 1)[0].replace("_", "-").lower()
    wanted[name] = ver.strip()
# Publish scripts need these even if an older image omitted them from the bake.
for name in ("e2b", "e2b-code-interpreter", "python-dotenv", "psycopg"):
    if name not in wanted:
        raise SystemExit(f"missing pin for {name} in requirements")
for name, ver in wanted.items():
    got = metadata.version(name)
    if got != ver:
        raise SystemExit(f"{name} {got} != {ver}")
import dotenv  # noqa: F401
import e2b  # noqa: F401
import psycopg  # noqa: F401
PY
}

pip_index_args() {
  # REGION=china | CLAW_E2B_CN=1 (pre/252 compose) → CN PyPI. Author: kejiqing
  if claw_region_is_china || [[ "${CLAW_E2B_CN:-}" == "1" ]]; then
    echo "==> pip index: mirrors.aliyun.com (china)" >&2
    printf '%s\n' -i https://mirrors.aliyun.com/pypi/simple --trusted-host mirrors.aliyun.com
  else
    echo "==> pip index: pypi.org (default)" >&2
  fi
}

ensure_venv() {
  local sys_py
  sys_py="$(command -v python3 || true)"
  if [[ -n "${sys_py}" ]] && sdk_pins_ok "${sys_py}" 2>/dev/null; then
    PY="${sys_py}"
    echo "==> using system python3 (image-baked SDK): ${PY}" >&2
    return 0
  fi

  local -a pip_extra=()
  local line
  while IFS= read -r line; do
    [[ -n "${line}" ]] && pip_extra+=("${line}")
  done < <(pip_index_args)

  PY="${VENV_DIR}/bin/python3"
  if [[ ! -x "${PY}" ]]; then
    echo "==> create ${VENV_DIR} (e2b SDK fallback venv)" >&2
    python3 -m venv "${VENV_DIR}"
  fi
  if sdk_pins_ok "${PY}" 2>/dev/null; then
    echo "==> reusing venv SDK: ${PY}" >&2
    return 0
  fi

  echo "==> install pinned e2b SDK into venv from ${SDK_REQ}" >&2
  local attempt=1
  while true; do
    if "${PY}" -m pip install -q "${pip_extra[@]}" -r "${SDK_REQ}"; then
      break
    fi
    if [[ "${attempt}" -ge 3 ]]; then
      echo "error: pip install failed after ${attempt} attempts (check REGION/CLAW_E2B_CN pip mirror)" >&2
      return 1
    fi
    echo "==> pip retry ${attempt}/3" >&2
    attempt=$((attempt + 1))
    sleep $((attempt * 2))
  done
  sdk_pins_ok "${PY}"
}
ensure_venv
[[ -n "${PY}" && -x "${PY}" ]] || {
  echo "error: no usable python for e2b bootstrap" >&2
  exit 1
}

PLATFORM="${CLAW_E2B_TEMPLATE_PLATFORM:-linux/amd64}"
echo "==> bootstrap templates from CI tag=${TAG}" >&2
echo "    worker_image=${WORKER_IMAGE} → debian+COPY claw (no nested podman)" >&2
echo "    relaxed_image=${RELAXED_IMAGE} → debian+COPY claw + tools (no OVS)" >&2
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

# neuro-harness engine workers (feat/neuro-harness): images from .github/workflows/neuro-harness-worker.yaml.
# Author: kejiqing
export CLAW_E2B_WORKER_OPENCODE_IMAGE="${CLAW_E2B_WORKER_OPENCODE_IMAGE:-${PREFIX}/claw-gateway-worker-opencode:${TAG}}"
export CLAW_E2B_WORKER_APPSERVER_IMAGE="${CLAW_E2B_WORKER_APPSERVER_IMAGE:-${PREFIX}/claw-gateway-worker-appserver:${TAG}}"
echo "    opencode_image=${CLAW_E2B_WORKER_OPENCODE_IMAGE} appserver_image=${CLAW_E2B_WORKER_APPSERVER_IMAGE}" >&2
run_py "${E2B_DIR}/build-claw-worker-opencode-selfhosted.py"
run_py "${E2B_DIR}/build-claw-worker-appserver-selfhosted.py"

echo "" >&2
echo "OK: bootstrap templates published for tag=${TAG} on ${E2B_API_URL}" >&2
