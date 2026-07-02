#!/usr/bin/env bash
# Verify claw-vscode: list-extensions + syntax check + Machine chat settings. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

CONTAINER="${CLAW_OVS_CONTAINER:-claw-openvscode-server}"
EXT_DIR="/opt/claw-extensions"
SD="/opt/claw-ovs/server-data"
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
PORT="${CLAW_OVS_HOST_PORT:-13000}"
SETTINGS_HOST="${ROOT_DIR}/deploy/stack/openvscode-settings.json"
EXT_VER="$(python3 -c "import json; print(json.load(open('${ROOT_DIR}/extensions/claw-vscode/package.json'))['version'])")"

fail() { echo "verify-claw-vscode: $*" >&2; exit 1; }

podman container exists "${CONTAINER}" 2>/dev/null || fail "container ${CONTAINER} not running"

echo "==> [1/5] list-extensions"
podman exec "${CONTAINER}" "${OVS_BIN}" \
  --list-extensions \
  --extensions-dir="${EXT_DIR}" \
  --server-data-dir="${SD}" 2>/dev/null \
  | grep -q '^claw\.claw-vscode$' || fail "claw.claw-vscode not listed (run: ./deploy/stack/lib/ovs-claw-restart.sh)"

echo "==> [2/5] extension.js syntax (OVS Node)"
podman exec "${CONTAINER}" /home/.openvscode-server/node --check \
  "${EXT_DIR}/claw.claw-vscode-${EXT_VER}/extension.js" \
  || fail "extension.js syntax error — JSDoc proj_* /home must not contain */ (closes comment)"

echo "==> [3/5] Machine settings (chat.agent.enabled)"
agent_enabled="$(podman exec "${CONTAINER}" /home/.openvscode-server/node -e "
const fs=require('fs');
const p='${SD}/Machine/settings.json';
const j=JSON.parse(fs.readFileSync(p,'utf8'));
const v=j['chat.agent.enabled'];
console.log(v===true?'true':String(v));
" 2>/dev/null)" || fail "cannot read Machine settings.json"
[[ "${agent_enabled}" == "true" ]] || fail "chat.agent.enabled=${agent_enabled} (must be true in deploy/stack/openvscode-settings.json)"

if [[ -f "${SETTINGS_HOST}" ]]; then
  python3 - "${SETTINGS_HOST}" <<'PY' || fail "host openvscode-settings.json must not set claw.projId"
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    cfg = json.load(f)
if "claw.projId" in cfg:
    raise SystemExit("claw.projId must not be in Machine settings")
PY
fi

echo "==> [4/5] OVS HTTP"
code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/ovs/" || true)"
[[ "${code}" == "200" ]] || fail "OVS HTTP ${code} (expected 200)"

echo "==> [5/5] Playground folder redirect (proj_1/home)"
PG_PORT="${PLAYGROUND_LISTEN_PORT:-18765}"
PG_USER="${PLAYGROUND_ADMIN_USER:-admin}"
PG_PASS="${PLAYGROUND_ADMIN_PASSWORD:-sunmi123}"
cookie="$(mktemp)"
trap 'rm -f "${cookie}"' EXIT
login_code="$(curl -sS -c "${cookie}" -o /dev/null -w '%{http_code}' \
  -X POST "http://127.0.0.1:${PG_PORT}/__admin_login__" \
  -H 'Content-Type: application/json' \
  -d "{\"user\":\"${PG_USER}\",\"password\":\"${PG_PASS}\",\"next\":\"/ovs?projId=1\"}" || echo 000)"
[[ "${login_code}" == "200" ]] || fail "playground login HTTP ${login_code}"
loc="$(curl -sS -b "${cookie}" -o /dev/null -w '%{redirect_url}' \
  "http://127.0.0.1:${PG_PORT}/ovs?projId=1" 2>/dev/null || true)"
echo "redirect: ${loc}"
echo "${loc}" | grep -q 'folder=%2Fhome%2Fworkspace%2Fproj_1%2Fhome' \
  || fail "Playground must 302 to :${PORT}/ovs/?folder=/home/workspace/proj_1/home (got: ${loc})"

echo "verify-claw-vscode: OK"
echo "Browser: http://127.0.0.1:18765/ovs?projId=1 (Playground login → redirect to proj_1/home)"
echo "Doc: docs/ovs-chat/EXTENSION-STABLE-DEPLOY.md"
