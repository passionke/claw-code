#!/usr/bin/env bash
# Dev: amd64 claw → stage → e2b Template.build strict + relaxed (+ PG). Author: kejiqing
#
# Modes for step 1 (obtain claw):
#   default            cross-compile linux/amd64 (native amd64 CI/host)
#   --from-ci-image    pull claw out of CI claw-code image (Mac arm64 / skip qemu)
#   --skip-compile     reuse deploy/stack/.linux-artifacts/release/claw
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
# shellcheck source=/dev/null
source "${LIB_DIR}/release-images.sh"

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
FORCE_COMPILE=0
FROM_CI_IMAGE=0
STRICT_ONLY=0
CI_IMAGE_TAG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-compile)
      SKIP_COMPILE=1
      shift
      ;;
    --from-ci-image)
      FROM_CI_IMAGE=1
      SKIP_COMPILE=1
      shift
      if [[ $# -gt 0 && "$1" != -* ]]; then
        CI_IMAGE_TAG="$1"
        shift
      fi
      ;;
    --force-compile)
      FORCE_COMPILE=1
      shift
      ;;
    --strict-only)
      STRICT_ONLY=1
      shift
      ;;
    --no-verify)
      SKIP_VERIFY=1
      shift
      ;;
    -h | --help)
      cat <<EOF
Usage: gateway.sh e2b-worker-deploy [options]

Obtain amd64 claw → stage → e2b Template.build for:
  1) claw-worker (strict) → PG e2bWorker.templateId
  2) claw-worker-relaxed (same claw + OVS) → PG e2bWorkerRelaxed.templateId

Obtain claw (pick one):
  (default)              Cross-compile linux/amd64 (use on amd64 CI/host)
  --from-ci-image [TAG]  Pull claw from CI claw-code image (Mac arm64; TAG e.g. release-v1.7.19)
  --skip-compile         Reuse deploy/stack/.linux-artifacts/release/claw (must be amd64 ELF)
  --force-compile        Allow qemu cross-compile on arm64 Mac (often SEGVs; prefer --from-ci-image)

Other:
  --strict-only          Skip claw-worker-relaxed (default builds both)
  --no-verify            Skip post-build sandbox smoke test

Env: CLAW_E2B_API_URL, CLAW_E2B_API_KEY, CLAW_E2B_TEMPLATE, CLAW_E2B_WORKER_ARCH=amd64
     CLAW_IMAGE_REGISTRY / CLAW_IMAGE_PREFIX (for --from-ci-image registry)
See deploy/e2b/WORKER-BUILD.md
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
export CLAW_CONTAINER_RUNTIME="${CLAW_CONTAINER_RUNTIME:-${CONTAINER_CLI}}"
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

# PEP 668: never pip into system python; reuse repo .venv-fc. Author: kejiqing
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

WORKER_ARCH="$(claw_e2b_worker_linux_arch)"
export CLAW_E2B_TEMPLATE_PLATFORM="${CLAW_E2B_TEMPLATE_PLATFORM:-$(claw_e2b_worker_linux_platform)}"
export CLAW_LINUX_COMPILE_PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-${CLAW_E2B_TEMPLATE_PLATFORM}}"

HOST_ARCH="$(uname -m)"
if [[ -n "${CLAW_E2B_DEV_WORKER_HOST:-}" ]]; then
  echo "==> dev worker node: ${CLAW_E2B_DEV_WORKER_HOST} (arch linux/${WORKER_ARCH})"
fi

# Pull /usr/local/bin/claw from CI claw-code image into linux-artifacts. Author: kejiqing
claw_e2b_fetch_claw_from_ci_image() {
  local tag="${1:?}"
  local dest="${ROOT_DIR}/deploy/stack/.linux-artifacts/release/claw"
  local platform
  platform="$(claw_e2b_worker_linux_platform)"
  claw_apply_release_image_tag "${tag}"
  local image="${GATEWAY_IMAGE}"
  echo "==> from-ci-image: ${image} (${platform}) → ${dest}"
  mkdir -p "$(dirname "${dest}")"
  "${CONTAINER_CLI}" pull --platform "${platform}" "${image}"
  local cid
  cid="$("${CONTAINER_CLI}" create --platform "${platform}" "${image}")"
  # shellcheck disable=SC2064
  trap "${CONTAINER_CLI} rm -f '${cid}' >/dev/null 2>&1 || true" RETURN
  "${CONTAINER_CLI}" cp "${cid}:/usr/local/bin/claw" "${dest}"
  "${CONTAINER_CLI}" rm -f "${cid}" >/dev/null
  trap - RETURN
  chmod +x "${dest}"
  local probe
  probe="$(file -b "${dest}")"
  echo "  claw: ${probe}"
  if ! claw_e2b_elf_arch_ok "${probe}" "${WORKER_ARCH}"; then
    echo "error: extracted claw is not linux/${WORKER_ARCH} (${probe})" >&2
    exit 1
  fi
}

# arm64 Mac + amd64 target: qemu rustc often SIGSEGVs — require CI image mode. Author: kejiqing
if [[ "${SKIP_COMPILE}" -eq 0 && "${FORCE_COMPILE}" -eq 0 && "${WORKER_ARCH}" == "amd64" ]]; then
  if [[ "${HOST_ARCH}" == "arm64" || "${HOST_ARCH}" == "aarch64" ]]; then
    echo "error: this host is ${HOST_ARCH}; e2b workers need linux/amd64 claw." >&2
    echo "hint: after claw-code-image CI, use:" >&2
    echo "  ./deploy/stack/gateway.sh e2b-worker-deploy --from-ci-image release-vX.Y.Z" >&2
    echo "hint: qemu cross-compile is unreliable here; pass --force-compile only to retry it." >&2
    exit 1
  fi
fi

CLAW_TIMING_LABEL="e2b-worker-deploy"
claw_timing_init

TOTAL_STEPS=4
if [[ "${STRICT_ONLY}" -eq 1 ]]; then
  TOTAL_STEPS=3
fi

if [[ "${FROM_CI_IMAGE}" -eq 1 ]]; then
  if [[ -z "${CI_IMAGE_TAG}" ]]; then
    CI_IMAGE_TAG="${CLAW_IMAGE_RELEASE_TAG:-}"
  fi
  if [[ -z "${CI_IMAGE_TAG}" ]]; then
    echo "error: --from-ci-image needs a tag (e.g. release-v1.7.19) or CLAW_IMAGE_RELEASE_TAG" >&2
    exit 1
  fi
  claw_step_begin "1/${TOTAL_STEPS} from-ci-image (${CI_IMAGE_TAG})"
  claw_e2b_fetch_claw_from_ci_image "${CI_IMAGE_TAG}"
elif [[ "${SKIP_COMPILE}" -eq 0 ]]; then
  claw_step_begin "1/${TOTAL_STEPS} linux compile (platform=${CLAW_LINUX_COMPILE_PLATFORM})"
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
  claw_step_begin "1/${TOTAL_STEPS} skip compile (reuse .linux-artifacts/release/claw)"
fi

claw_step_begin "2/${TOTAL_STEPS} stage claw → ${CLAW_E2B_TEMPLATE_COPY_DIR}"
"${LIB_DIR}/stage-e2b-worker-bins.sh"

if [[ "${SKIP_VERIFY}" -eq 1 ]]; then
  export CLAW_E2B_TEMPLATE_SKIP_VERIFY=1
fi

claw_step_begin "3/${TOTAL_STEPS} e2b Template.build strict (copy, alias=${CLAW_E2B_TEMPLATE:-claw-worker}, platform=${CLAW_E2B_TEMPLATE_PLATFORM})"
"${E2B_PY}" "${ROOT_DIR}/deploy/e2b/build-claw-worker-selfhosted.py"

if [[ "${STRICT_ONLY}" -eq 0 ]]; then
  # Same COPY_DIR claw as strict; relaxed script prefers CLAW_E2B_TEMPLATE_COPY_DIR. Author: kejiqing
  claw_step_begin "4/${TOTAL_STEPS} e2b Template.build relaxed (alias=${CLAW_E2B_WORKER_RELAXED_ALIAS:-claw-worker-relaxed})"
  "${E2B_PY}" "${ROOT_DIR}/deploy/e2b/build-claw-worker-relaxed-selfhosted.py"
fi

claw_timing_summary
if [[ "${STRICT_ONLY}" -eq 0 ]]; then
  echo "==> e2b worker templates ready (strict=${CLAW_E2B_TEMPLATE:-claw-worker} + relaxed=${CLAW_E2B_WORKER_RELAXED_ALIAS:-claw-worker-relaxed}, linux/${WORKER_ARCH})"
else
  echo "==> e2b worker template ready (strict=${CLAW_E2B_TEMPLATE:-claw-worker}, linux/${WORKER_ARCH}; --strict-only)"
fi
echo "    new sandboxes use updated claw; existing sandboxes keep old binary until reconcile/reset"
