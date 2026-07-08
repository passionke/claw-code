#!/usr/bin/env bash
# Pre-252 full e2b pipeline: templates → singletons → gateway up --release → verify. Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
# shellcheck source=stack-instance.sh
source "${LIB_DIR}/stack-instance.sh"
BUILD_TEMPLATES_SH="${REPO_ROOT}/deploy/e2b/build-selfhosted-templates.sh"

release=""
skip_templates=0
skip_singletons=0
skip_gateway=0
skip_verify=0
template_args=()
singleton_args=(--reuse)

usage() {
  cat <<'EOF'
Usage: ./deploy/stack/gateway.sh pre-252-e2b-up [options]

串联预发 252（外连 250 PG + e2b）完整路径：

  Phase 0  preflight   PG + e2b /health + Claw 模板存在性
  Phase 1  templates   claw-code 本机构建模板 → 250 e2bserver（claw-deploy 自动跳过）
  Phase 2  gateway      up --release <tag>（启动时 ensure e2b singletons）
  Phase 3  singletons  经 gateway API 再 ensure 一遍（幂等）
  Phase 4  verify      claw-stack-verify

Options:
  --release TAG       CI 镜像 tag（如 release-v1.6.18）；省略则只做到 singletons
  --skip-templates    模板已在 e2b 上
  --skip-singletons   仅构建模板
  --skip-gateway      不启动 gateway
  --skip-verify       跳过 verify
  --skip-cache        模板强制 fresh docker build
  --reset             重建 singleton sandbox（默认 --reuse）

分仓说明：
  claw-code（开发机）  Phase 0–2 或全量
  claw-deploy（252）   Phase 0,2–4（模板在开发机先跑 Phase 1）

示例：
  # 开发机（claw-code）
  ./deploy/stack/gateway.sh pre-252-e2b-up --skip-gateway --skip-cache

  # 252（claw-code 或 claw-deploy）
  ./deploy/stack/gateway.sh pre-252-e2b-up --release release-v1.6.18
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) release="$2"; shift 2 ;;
    --skip-templates) skip_templates=1; shift ;;
    --skip-singletons) skip_singletons=1; shift ;;
    --skip-gateway) skip_gateway=1; shift ;;
    --skip-verify) skip_verify=1; shift ;;
    --skip-cache) template_args+=(--skip-cache); shift ;;
    --reset) singleton_args=(--reset); shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown: $1" >&2; usage >&2; exit 1 ;;
  esac
done

[[ -f "${ENV_FILE}" ]] || { echo "error: missing ${ENV_FILE}" >&2; exit 1; }
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

REQUIRED_TEMPLATES=(claw-worker claw-nas-api claw-ovs claw-observe)

preflight_remote() {
  echo "==> Phase 0: preflight" >&2
  local pg_url="${CLAW_GATEWAY_DATABASE_URL:-}"
  local e2b_url="${CLAW_E2B_API_URL:-}"
  [[ -n "${pg_url}" ]] || { echo "error: CLAW_GATEWAY_DATABASE_URL unset" >&2; exit 1; }
  [[ -n "${e2b_url}" ]] || { echo "error: CLAW_E2B_API_URL unset" >&2; exit 1; }
  [[ -n "${CLAW_E2B_API_KEY:-}" ]] || { echo "error: CLAW_E2B_API_KEY unset" >&2; exit 1; }

  echo "    PG $(claw_redact_database_url "${pg_url}")" >&2
  python3 -c "
import sys
try:
    import psycopg
except ImportError:
    sys.exit(0)
url = sys.argv[1]
with psycopg.connect(url, connect_timeout=8) as conn:
    conn.execute('SELECT 1')
print('    PG: OK')
" "${pg_url}" 2>/dev/null || echo "    PG: skip (install psycopg to verify)" >&2

  local health
  health="$(curl -fsS -m 15 "${e2b_url%/}/health")" || {
    echo "error: GET ${e2b_url}/health failed" >&2
    exit 1
  }
  echo "    e2b: ${e2b_url}/health OK" >&2

  local missing
  missing="$(printf '%s' "${health}" | python3 -c "
import json, sys
required = sys.argv[1].split(',')
text = json.dumps(json.load(sys.stdin))
missing = [t for t in required if t not in text]
print(','.join(missing))
" "${REQUIRED_TEMPLATES[*]}")"

  if [[ -n "${missing}" ]]; then
    echo "    templates missing on e2b: ${missing}" >&2
    if [[ "${skip_templates}" -eq 1 ]]; then
      echo "error: --skip-templates but templates not on e2b; run build on claw-code first" >&2
      exit 1
    fi
    if [[ ! -f "${BUILD_TEMPLATES_SH}" ]]; then
      echo "error: claw-deploy cannot build templates; on dev machine run:" >&2
      echo "  cd ~/work/claw-code && ./deploy/e2b/build-selfhosted-templates.sh" >&2
      exit 1
    fi
    echo "    will build in Phase 1" >&2
  else
    echo "    templates: ${REQUIRED_TEMPLATES[*]} present" >&2
  fi
}

phase_templates() {
  if [[ "${skip_templates}" -eq 1 ]]; then
    echo "==> Phase 1: skip templates (--skip-templates)" >&2
    return 0
  fi
  if [[ ! -f "${BUILD_TEMPLATES_SH}" ]]; then
    echo "==> Phase 1: skip templates (claw-deploy; build on claw-code dev machine)" >&2
    return 0
  fi
  echo "==> Phase 1: build templates → ${CLAW_E2B_API_URL:-e2b}" >&2
  "${BUILD_TEMPLATES_SH}" "${template_args[@]}"
}

phase_gateway() {
  if [[ "${skip_gateway}" -eq 1 ]] || [[ -z "${release}" ]]; then
    if [[ -z "${release}" ]] && [[ "${skip_gateway}" -eq 0 ]]; then
      echo "==> Phase 2: skip gateway (no --release)" >&2
      echo "    next: ./deploy/stack/gateway.sh pre-252-e2b-up --release release-vX.Y.Z" >&2
    fi
    return 0
  fi
  echo "==> Phase 2: gateway up --release ${release}" >&2
  "${LIB_DIR}/up.sh" --release "${release}"
}

phase_singletons() {
  if [[ "${skip_singletons}" -eq 1 ]]; then
    echo "==> Phase 3: skip singletons (--skip-singletons)" >&2
    return 0
  fi
  if [[ "${skip_gateway}" -eq 1 ]] || [[ -z "${release}" ]]; then
    echo "==> Phase 3: skip singletons API (no gateway up; singletons run on gateway startup when --release)" >&2
    return 0
  fi
  echo "==> Phase 3: e2b singletons → PG (gateway API, idempotent)" >&2
  "${LIB_DIR}/e2b-singletons-up.sh" "${singleton_args[@]}"
}

phase_verify() {
  if [[ "${skip_verify}" -eq 1 ]] || [[ -z "${release}" ]] || [[ "${skip_gateway}" -eq 1 ]]; then
    return 0
  fi
  echo "==> Phase 4: verify" >&2
  "${LIB_DIR}/claw-stack-verify.sh"
}

preflight_remote
phase_templates
phase_gateway
phase_singletons
phase_verify

echo "" >&2
echo "OK — pre-252 e2b pipeline done" >&2
if [[ -z "${release}" ]] || [[ "${skip_gateway}" -eq 1 ]]; then
  echo "next: ./deploy/stack/gateway.sh pre-252-e2b-up --release release-vX.Y.Z" >&2
fi
