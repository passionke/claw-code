#!/usr/bin/env bash
# Verify e2b applies nasConfig bind into sandbox guest. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

API_KEY="${CLAW_E2B_API_KEY:-${E2B_API_KEY:-}}"
API_URL="${CLAW_E2B_API_URL:-http://10.8.0.1:3000}"
SANDBOX_URL="${CLAW_E2B_SANDBOX_URL:-http://10.8.0.1:3002}"
DOMAIN="${CLAW_E2B_DOMAIN:-supone.top}"
TEMPLATE="${CLAW_E2B_TEMPLATE:-claw-worker}"
CLUSTER_ID="${CLAW_CLUSTER_ID:-local-dev}"
# Bind source on the **e2b host** (`10.8.0.1`). Prefer CLAW_E2B_NAS_HOST_MOUNT; else e2b /health nas.hostMountRoot.
E2B_NAS_ROOT="${CLAW_E2B_NAS_HOST_MOUNT:-}"
if [[ -z "${E2B_NAS_ROOT}" ]]; then
  E2B_NAS_ROOT="$(curl -sS -m 10 "${API_URL%/}/health" | python3 -c "import json,sys; print((json.load(sys.stdin).get('nas') or {}).get('hostMountRoot',''))")"
fi
[[ -n "${E2B_NAS_ROOT}" ]] || E2B_NAS_ROOT="/mnt/nas0"

fail() { echo "verify-e2b-nas-inject: $*" >&2; exit 1; }

[[ -n "${API_KEY}" ]] || fail "set CLAW_E2B_API_KEY in .env"
command -v python3 >/dev/null || fail "python3 required"

HEALTH="$(curl -sS -m 10 "${API_URL%/}/health")" || fail "GET ${API_URL}/health failed"
NAS_READY="$(echo "${HEALTH}" | python3 -c "import json,sys; print((json.load(sys.stdin).get('nas') or {}).get('ready', False))")"
[[ "${NAS_READY}" == "True" ]] || fail "e2b nas not ready: $(echo "${HEALTH}" | python3 -c "import json,sys; print(json.load(sys.stdin).get('nas'))")"
echo "==> e2b health nas: $(echo "${HEALTH}" | python3 -c "import json,sys; print(json.load(sys.stdin).get('nas'))")"

resolve_nas_api_url() {
  if [[ -n "${CLAW_E2B_NAS_API_URL:-}" ]]; then
    printf '%s' "${CLAW_E2B_NAS_API_URL%/}"
    return 0
  fi
  bash "${ROOT_DIR}/deploy/stack/lib/e2b-nas-api-up.sh" --reuse --json 2>/dev/null \
    | python3 -c "import json,sys; print(json.load(sys.stdin).get('baseUrl','').rstrip('/'))"
}

NAS_API_URL="$(resolve_nas_api_url)"
[[ -n "${NAS_API_URL}" ]] || fail "claw-nas-api baseUrl missing; run: ./deploy/stack/gateway.sh nas-api-up"
curl -fsS -m 15 "${NAS_API_URL}/healthz" >/dev/null \
  || fail "claw-nas-api not healthy at ${NAS_API_URL}/healthz"
echo "==> claw-nas-api: ${NAS_API_URL}"

VENV="${ROOT_DIR}/.cache/e2b-venv"
if ! "${VENV}/bin/python" -c "import e2b_code_interpreter" 2>/dev/null; then
  python3 -m venv "${VENV}"
  "${VENV}/bin/pip" install -q e2b-code-interpreter
fi

WORKER="wrk_verify_nas_inject"
PROJ="${CLAW_E2B_E2E_PROJ_ID:-1}"

# e2b validates host paths on the NAS export tree; gateway prepares via claw-nas-api (not local CLAW_NAS_HOST_MOUNT).
for rel in \
  "${CLUSTER_ID}/proj_${PROJ}/workers/${WORKER}/.claw" \
  "${CLUSTER_ID}/proj_${PROJ}/home" \
  "tap-traces"; do
  echo "==> nas-api mkdir ${rel}"
  curl -fsS -m 30 -X POST "${NAS_API_URL}/v1/mkdir" \
    -H "Content-Type: application/json" \
    -d "{\"relPath\":\"${rel}\",\"parents\":true}" >/dev/null
done

export PROJ WORKER E2B_NAS_ROOT TEMPLATE API_KEY API_URL SANDBOX_URL DOMAIN CLUSTER_ID
BODY="$(python3 - <<'PY'
import json, os
proj = int(os.environ["PROJ"])
worker = os.environ["WORKER"]
cluster = os.environ["CLUSTER_ID"]
root = os.environ["E2B_NAS_ROOT"]
nas = {
    "userId": int(os.environ.get("CLAW_WORKER_UID", "1000")),
    "groupId": int(os.environ.get("CLAW_WORKER_GID", "1000")),
    "hostMountRoot": root,
    "mountPoints": [
        {"relPath": f"{cluster}/proj_{proj}/workers/{worker}", "mountDir": "/claw_host_root"},
        {"relPath": f"{cluster}/proj_{proj}/home", "mountDir": "/claw_ds"},
        {"relPath": "tap-traces", "mountDir": "/claw_tap_traces"},
    ],
}
print(json.dumps({
    "templateID": os.environ["TEMPLATE"],
    "timeout": 300,
    "metadata": {"verify": "nas-inject"},
    "nasConfig": nas,
    "secure": False,
}))
PY
)"

CREATE_HTTP="$(curl -sS -m 120 -w $'\n%{http_code}' -X POST "${API_URL%/}/sandboxes" \
  -H "X-API-Key: ${API_KEY}" -H "Content-Type: application/json" -d "${BODY}")"
CREATE_BODY="${CREATE_HTTP%$'\n'*}"
CREATE_CODE="${CREATE_HTTP##*$'\n'}"
if [[ "${CREATE_CODE}" != "200" && "${CREATE_CODE}" != "201" ]]; then
  fail "POST /sandboxes HTTP ${CREATE_CODE}: ${CREATE_BODY}"
fi
SID="$(echo "${CREATE_BODY}" | python3 -c "import json,sys; print(json.load(sys.stdin)['sandboxID'])")"
echo "==> created ${SID} hostMountRoot=${E2B_NAS_ROOT} cluster=${CLUSTER_ID}"

cleanup() {
  curl -fsS -m 10 -X DELETE "${API_URL%/}/sandboxes/${SID}" -H "X-API-Key: ${API_KEY}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

export SID
if "${VENV}/bin/python" - <<'PY'
import os, sys
from e2b_code_interpreter import Sandbox

s = Sandbox.connect(
    os.environ["SID"],
    api_key=os.environ["API_KEY"],
    domain=os.environ["DOMAIN"],
    api_url=os.environ["API_URL"],
    sandbox_url=os.environ["SANDBOX_URL"],
)
r = s.commands.run(
    "for d in /claw_host_root /claw_ds /claw_tap_traces; do "
    'mountpoint -q "$d" 2>/dev/null && echo "OK $d" || echo "MISS $d"; done',
    timeout=30,
)
out = (r.stdout or "").strip()
print(out)
sys.exit(1 if "MISS " in out else 0)
PY
then
  echo "verify-e2b-nas-inject: OK"
  exit 0
fi

HEALTH_ROOT="$(echo "${HEALTH}" | python3 -c "import json,sys; n=json.load(sys.stdin).get('nas') or {}; print(n.get('hostMountRoot',''))")"
fail "e2b ignored nasConfig bind (mountDirs MISS). health hostMountRoot=${HEALTH_ROOT}; tried ${E2B_NAS_ROOT}. See docs/e2b-nas-workspace.md §5 (direct bind only; no sync-nas-bind)."
