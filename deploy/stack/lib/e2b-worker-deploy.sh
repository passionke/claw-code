#!/usr/bin/env bash
# Dev: local linux compile → stage claw+ttyd → e2b Template.build (no CI/ACR). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STACK_DIR="$(cd "${LIB_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${STACK_DIR}/../.." && pwd)"
cd "${ROOT_DIR}"

# shellcheck source=/dev/null
source "${LIB_DIR}/compose-include.sh"
# shellcheck source=/dev/null
source "${LIB_DIR}/claw-step-timing.sh"
# shellcheck source=/dev/null
source "${LIB_DIR}/e2b-worker-arch.sh"

if [[ -f "${ROOT_DIR}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/.env"
  set +a
  # shellcheck source=/dev/null
  source "${LIB_DIR}/env-profile.sh"
  claw_apply_deploy_profile || exit 1
fi

SKIP_COMPILE=0
SKIP_VERIFY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-compile)
      SKIP_COMPILE=1
      shift
      ;;
    --no-verify)
      SKIP_VERIFY=1
      shift
      ;;
    -h | --help)
      cat <<EOF
Usage: gateway.sh e2b-worker-deploy [options]

Local dev: cross-compile linux/amd64 claw + stage ttyd → e2b worker template (copy).
Build persists settings_json.e2bWorker.templateId to PG. Gateway startup/renewal reconciles workers.
See deploy/e2b/WORKER-BUILD.md

Options:
  --skip-compile   Reuse deploy/stack/.linux-artifacts/release/claw (must match target arch)
  --no-verify      Skip post-build sandbox smoke test

Env (from .env):
  CLAW_E2B_API_URL, CLAW_E2B_API_KEY, CLAW_E2B_TEMPLATE (default claw-worker)
  CLAW_E2B_WORKER_ARCH       amd64（自托管默认；e2b 节点全是 x86_64）
  CLAW_E2B_DEV_WORKER_HOST   optional hint (e.g. 10.8.0.2); logged only
  CLAW_E2B_TEMPLATE_COPY_DIR default deploy/stack/.e2b-worker-bins

After gateway-only Rust changes: pack-deploy.
After claw binary in e2b sandboxes: this command.
EOF
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 2
      ;;
  esac
done

CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1
export CLAW_E2B_TEMPLATE_BUILD_STRATEGY=copy
export CLAW_E2B_TEMPLATE_COPY_DIR="${CLAW_E2B_TEMPLATE_COPY_DIR:-${ROOT_DIR}/deploy/stack/.e2b-worker-bins}"

export E2B_API_KEY="${E2B_API_KEY:-${CLAW_E2B_API_KEY:-}}"
export E2B_API_URL="${E2B_API_URL:-${CLAW_E2B_API_URL:-}}"
export E2B_SANDBOX_URL="${E2B_SANDBOX_URL:-${CLAW_E2B_SANDBOX_URL:-}}"
export E2B_DOMAIN="${E2B_DOMAIN:-${CLAW_E2B_DOMAIN:-}}"

if [[ -z "${E2B_API_KEY}" || -z "${E2B_API_URL}" ]]; then
  echo "error: set CLAW_E2B_API_KEY and CLAW_E2B_API_URL in .env" >&2
  exit 1
fi

if [[ -n "${CLAW_E2B_DEV_WORKER_HOST:-}" ]]; then
  echo "==> dev worker node: ${CLAW_E2B_DEV_WORKER_HOST} (arch linux/${WORKER_ARCH})"
fi

# PEP 668: never pip into system python; reuse repo .venv-fc (same as build-selfhosted-templates.sh). Author: kejiqing
E2B_PY="${ROOT_DIR}/.venv-fc/bin/python3"
claw_ensure_e2b_python_venv() {
  if [[ -x "${E2B_PY}" ]] && "${E2B_PY}" -c "import e2b" 2>/dev/null; then
    return 0
  fi
  if [[ ! -x "${E2B_PY}" ]]; then
    echo "==> create ${ROOT_DIR}/.venv-fc (e2b SDK)" >&2
    if ! python3 -m venv "${ROOT_DIR}/.venv-fc" 2>/dev/null; then
      echo "error: python3 -m venv failed; install python3-venv (apt install python3-venv)" >&2
      exit 1
    fi
  fi
  echo "==> install e2b python SDK in .venv-fc" >&2
  "${E2B_PY}" -m pip install -q e2b==2.26.0 e2b-code-interpreter python-dotenv 'psycopg[binary]'
}
claw_ensure_e2b_python_venv

CLAW_TIMING_LABEL="e2b-worker-deploy"
claw_timing_init

if [[ "${SKIP_COMPILE}" -eq 0 ]]; then
  claw_step_begin "1/3 linux compile (platform=${CLAW_LINUX_COMPILE_PLATFORM})"
  CN_FLAG=0
  if [[ "${CLAW_USE_CN_CRATES_MIRROR:-0}" == "1" || "${CLAW_USE_CN_RUST_MIRROR:-0}" == "1" ]]; then
    CN_FLAG=1
  fi
  if [[ "${CLAW_USE_DOCKER_IO:-}" == "1" ]] || [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
    REG="docker.io"
  else
    REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
    REG="${REG%/}"
  fi
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/linux-compile.sh"
  # shellcheck source=/dev/null
  source "${ROOT_DIR}/deploy/stack/lib/rust-compile-image.sh"
  COMPILE_IMAGE="$(claw_ensure_rust_compile_image "${ROOT_DIR}" "${CONTAINER_CLI}" "${REG}")"
  claw_linux_compile_release "${ROOT_DIR}" "${CONTAINER_CLI}" "${COMPILE_IMAGE}" "${CN_FLAG}"
else
  claw_step_begin "1/3 skip compile (reuse .linux-artifacts/release/claw)"
fi

claw_step_begin "2/3 stage claw + ttyd → ${CLAW_E2B_TEMPLATE_COPY_DIR}"
"${LIB_DIR}/stage-e2b-worker-bins.sh"

claw_step_begin "3/3 e2b Template.build (copy, alias=${CLAW_E2B_TEMPLATE:-claw-worker}, platform=${CLAW_E2B_TEMPLATE_PLATFORM})"
if [[ "${SKIP_VERIFY}" -eq 1 ]]; then
  export CLAW_E2B_TEMPLATE_SKIP_VERIFY=1
fi
"${E2B_PY}" "${ROOT_DIR}/deploy/e2b/build-claw-worker-selfhosted.py"

claw_timing_summary
echo "==> e2b worker template ready (${CLAW_E2B_TEMPLATE:-claw-worker}, linux/${WORKER_ARCH})"
echo "    new sandboxes use updated claw; existing sandboxes keep old binary"
