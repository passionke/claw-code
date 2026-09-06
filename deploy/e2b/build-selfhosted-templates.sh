#!/usr/bin/env bash
# Build all self-hosted e2b templates (local → e2b API on CLAW_E2B_API_URL). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
E2B_DIR="${ROOT_DIR}/deploy/e2b"
cd "${ROOT_DIR}"

# shellcheck source=/dev/null
source "${ROOT_DIR}/deploy/stack/lib/claw-region.sh"
claw_region_load

# shellcheck disable=SC1091
[[ -f "${ROOT_DIR}/.env" ]] && set -a && source "${ROOT_DIR}/.env" && set +a
claw_region_load

usage() {
  cat <<'EOF'
Usage: ./deploy/e2b/build-selfhosted-templates.sh [options] [targets...]

Targets (default: all):
  worker   claw-worker (+ relaxed alias)
  nas-api  claw-nas-api
  ovs      claw-ovs
  observe  claw-observe

Options:
  --skip-cache   pass CLAW_E2B_TEMPLATE_SKIP_CACHE=1
  --only NAME    same as single target

Env (repo root .env + machine region=china via deploy/stack/lib/claw-region.sh):
  region=china               → docker.1ms.run debian base + mirrors.aliyun.com apt
  CLAW_E2B_API_URL           → e2bserver (e.g. http://192.168.9.250:3000)
  CLAW_E2B_API_KEY           → api_key from e2bserver config.toml
  CLAW_E2B_TEMPLATE_SKIP_CACHE=1  force fresh docker build on e2b host

Build logs: SDK streams e2bserver docker build; on 250 also check:
  journalctl -u e2bserver -f   OR   docker logs <e2b-builder>
EOF
}

# Writable venv for e2b SDK on deploy host (override with CLAW_E2B_VENV).
VENV_DIR="${CLAW_E2B_VENV:-${ROOT_DIR}/.venv-fc}"
PY="${VENV_DIR}/bin/python3"
FC_PIP_TARGET="${VENV_DIR}/deps"

ensure_venv() {
  local -a pip_extra=()
  if claw_region_is_china; then
    pip_extra=(-i https://pypi.tuna.tsinghua.edu.cn/simple --trusted-host pypi.tuna.tsinghua.edu.cn)
  fi
  if [[ -x "${PY}" ]] && "${PY}" -c "import e2b, psycopg" 2>/dev/null; then
    return 0
  fi
  if [[ "${PY}" == "python3" ]] || [[ "${PY}" == "$(command -v python3)" ]]; then
    if python3 -c "import e2b, psycopg" 2>/dev/null; then
      return 0
    fi
  fi
  mkdir -p "$(dirname "${VENV_DIR}")"
  echo "==> create ${VENV_DIR} (e2b SDK)" >&2
  if python3 -m venv "${VENV_DIR}" 2>/dev/null; then
    "${PY}" -m pip install -q "${pip_extra[@]}" e2b==2.26.0 e2b-code-interpreter python-dotenv 'psycopg[binary]'
    return 0
  fi
  echo "==> venv unavailable; pip --target ${FC_PIP_TARGET}" >&2
  mkdir -p "${FC_PIP_TARGET}"
  python3 -m pip install -q "${pip_extra[@]}" --target "${FC_PIP_TARGET}" \
    e2b==2.26.0 e2b-code-interpreter python-dotenv 'psycopg[binary]'
  export PYTHONPATH="${FC_PIP_TARGET}${PYTHONPATH:+:${PYTHONPATH}}"
  PY="python3"
}

TARGETS=()
SKIP_CACHE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --skip-cache) SKIP_CACHE=1; shift ;;
    --only) TARGETS+=("$2"); shift 2 ;;
    worker|nas-api|ovs|observe) TARGETS+=("$1"); shift ;;
    *) echo "unknown arg: $1" >&2; usage >&2; exit 1 ;;
  esac
done
[[ ${#TARGETS[@]} -eq 0 ]] && TARGETS=(worker nas-api ovs observe)

nas_api_enabled() {
  case "${CLAW_E2B_NAS_API:-1}" in
    0|false|no|off|FALSE|NO|OFF) return 1 ;;
    *) return 0 ;;
  esac
}

if [[ "${SKIP_CACHE}" -eq 1 ]]; then
  export CLAW_E2B_TEMPLATE_SKIP_CACHE=1
fi

ensure_venv

echo "==> e2b API: ${CLAW_E2B_API_URL:-unset}  region=${region:-${REGION:-${CLAW_REGION:-unset}}}" >&2

run() {
  echo "" >&2
  echo "======== $(date -Iseconds) $* ========" >&2
  "${PY}" "$@"
}

for t in "${TARGETS[@]}"; do
  case "${t}" in
    worker)
      run "${E2B_DIR}/build-claw-worker-selfhosted.py"
      run "${E2B_DIR}/build-claw-worker-relaxed-selfhosted.py"
      ;;
    nas-api)
      if nas_api_enabled; then
        run "${E2B_DIR}/build-claw-nas-api-selfhosted.py"
      else
        echo "==> skip nas-api (CLAW_E2B_NAS_API=0)" >&2
      fi
      ;;
    ovs)     run "${E2B_DIR}/build-claw-ovs-selfhosted.py" ;;
    observe) run "${E2B_DIR}/build-claw-observe-selfhosted.py" ;;
  esac
done

echo "" >&2
echo "OK: templates built on ${CLAW_E2B_API_URL:-e2b}" >&2
echo "next: curl -X POST http://127.0.0.1:\${GATEWAY_HOST_PORT:-8088}/v1/gateway/global-settings/e2b-singletons/<nas-api|ovs|observe>/reset" >&2
