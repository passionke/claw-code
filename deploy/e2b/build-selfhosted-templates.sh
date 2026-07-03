#!/usr/bin/env bash
# Build all self-hosted e2b templates (local → e2b API on CLAW_E2B_API_URL). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
E2B_DIR="${ROOT_DIR}/deploy/e2b"
cd "${ROOT_DIR}"

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

Env (repo root .env, auto-loaded by python scripts):
  CLAW_E2B_CN=1              → docker.1ms.run debian base
  CLAW_E2B_API_URL           → e2bserver (e.g. http://192.168.9.250:3000)
  CLAW_E2B_API_KEY           → api_key from e2bserver config.toml
  CLAW_E2B_TEMPLATE_SKIP_CACHE=1  force fresh docker build on e2b host

Build logs: SDK streams e2bserver docker build; on 250 also check:
  journalctl -u e2bserver -f   OR   docker logs <e2b-builder>
EOF
}

PY="${ROOT_DIR}/.venv-fc/bin/python3"
ensure_venv() {
  if [[ ! -x "${PY}" ]]; then
    echo "==> create ${ROOT_DIR}/.venv-fc (e2b SDK)" >&2
    python3 -m venv "${ROOT_DIR}/.venv-fc"
    "${PY}" -m pip install -q e2b==2.26.0 e2b-code-interpreter python-dotenv 'psycopg[binary]'
  fi
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

if [[ "${SKIP_CACHE}" -eq 1 ]]; then
  export CLAW_E2B_TEMPLATE_SKIP_CACHE=1
fi

ensure_venv

# shellcheck disable=SC1091
[[ -f "${ROOT_DIR}/.env" ]] && set -a && source "${ROOT_DIR}/.env" && set +a

echo "==> e2b API: ${CLAW_E2B_API_URL:-unset}  CLAW_E2B_CN=${CLAW_E2B_CN:-unset}" >&2

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
    nas-api) run "${E2B_DIR}/build-claw-nas-api-selfhosted.py" ;;
    ovs)     run "${E2B_DIR}/build-claw-ovs-selfhosted.py" ;;
    observe) run "${E2B_DIR}/build-claw-observe-selfhosted.py" ;;
  esac
done

echo "" >&2
echo "OK: templates built on ${CLAW_E2B_API_URL:-e2b}" >&2
echo "next: ./deploy/stack/gateway.sh e2b-singletons-up --reuse" >&2
