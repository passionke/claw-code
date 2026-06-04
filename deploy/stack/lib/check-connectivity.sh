#!/usr/bin/env bash
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE} — copy ${REPO_ROOT}/.env.example" >&2
  exit 1
fi

# shellcheck disable=SC1090
source "${ENV_FILE}"
# shellcheck disable=SC1091
source "${LIB_DIR}/compose-include.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"

PLAYGROUND_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
POOL_HTTP_PORT="${CLAW_POOL_HTTP_PORT:-9944}"
GW_CTN="${CLAW_GATEWAY_CONTAINER:-claw-gateway-rs}"

echo "[1/5] gateway healthz"
curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/healthz" >/tmp/claw_gateway_healthz.json
python3 -c '
import json, sys
d = json.load(open("/tmp/claw_gateway_healthz.json"))
tag = d.get("deployImageTag")
ref = d.get("deployImageRef")
if tag:
    print(f"deployImageTag={tag} deployImageRef={ref or ""}")
else:
    print("(healthz missing deployImageTag — rebuild gateway-rs image and recreate container)")
' 2>/dev/null || true
cat /tmp/claw_gateway_healthz.json
echo

echo "[2/5] pool HTTP from gateway-rs container (host.containers.internal:${POOL_HTTP_PORT})"
podman exec "${GW_CTN}" curl -fsS --max-time 5 \
  "http://host.containers.internal:${POOL_HTTP_PORT}/healthz/live-report" \
  >/tmp/claw_pool_health.json
python3 -c 'import json; d=json.load(open("/tmp/claw_pool_health.json")); print("pool live-report ok", d.get("ok", d))' 2>/dev/null || cat /tmp/claw_pool_health.json
echo

if claw_pool_daemon_on_host; then
  echo "[2b/5] host pool RPC (127.0.0.1:${CLAW_POOL_DAEMON_PORT:-9943}) — required for solve_async"
  claw_assert_host_pool_rpc_ready "${PODMAN_DIR}/.claw-pool-rpc" || exit 1
  echo "host pool RPC ok"
  echo
else
  # shellcheck disable=SC1091
  source "${LIB_DIR}/pool-sidecar-health.sh"
  echo "[2b/5] compose pool sidecar (gateway → claw-pool-daemon:${CLAW_POOL_DAEMON_PORT:-9943})"
  claw_assert_gateway_pool_rpc_reachable || exit 1
  echo "compose pool RPC ok"
  echo
fi

echo "[3/5] solve_async smoke (extraSession from ds 1 project config when defined)"
if claw_pool_daemon_on_host; then
  claw_assert_host_pool_rpc_ready "${PODMAN_DIR}/.claw-pool-rpc" || {
    echo "error: refuse solve_async smoke — host pool RPC not ready" >&2
    exit 1
  }
fi
SOLVE_BODY="$(python3 <<PY
import json
import urllib.request

port = ${GATEWAY_PORT}
cfg = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/v1/project/config/1", timeout=10))
fields = [f for f in (cfg.get("extraSessionFieldsJson") or []) if isinstance(f, str) and f.strip()]
body = {"dsId": 1, "userPrompt": "connectivity check"}
if fields:
    body["extraSession"] = {f: "" for f in fields}
print(json.dumps(body, ensure_ascii=False))
PY
)"
echo "POST body: ${SOLVE_BODY}"
TASK_JSON="$(curl -fsS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d "${SOLVE_BODY}")"
echo "${TASK_JSON}"
TASK_ID="$(printf "%s" "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["taskId"])')"

echo "[4/5] verify MCP list is available"
MCP_JSON="$(curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/mcp/injected/1")"
echo "${MCP_JSON}"
MCP_JSON="${MCP_JSON}" python3 -c '
import json
import os

obj = json.loads(os.environ["MCP_JSON"])
servers = obj.get("mcpReport", {}).get("servers", [])
if not isinstance(servers, list):
    raise SystemExit("mcpReport.servers missing")
print(f"mcp servers visible: {len(servers)}")
'

echo "[5/5] gateway-playground UI"
for _ in $(seq 1 20); do
  if curl -fsS "http://127.0.0.1:${PLAYGROUND_PORT}/__config__" >/tmp/claw_playground_config.json 2>/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:${PLAYGROUND_PORT}/__config__" >/tmp/claw_playground_config.json
python3 -c 'import json; c=json.load(open("/tmp/claw_playground_config.json")); assert c.get("defaultGatewayBase"); print("playground ok, defaultGatewayBase=", c["defaultGatewayBase"])'
# SPA white screen when hashed bundles 404 → index.html (text/html served as application/javascript).
curl -fsSL "http://127.0.0.1:${PLAYGROUND_PORT}/admin/" -o /tmp/claw_playground_html.txt
js_path="$(grep -oE '/admin/assets/index-[^"]+\.js' /tmp/claw_playground_html.txt | head -1)"
if [[ -z "${js_path}" ]]; then
  echo "error: playground /admin/ missing script src in index.html" >&2
  exit 1
fi
curl -fsS "http://127.0.0.1:${PLAYGROUND_PORT}${js_path}" -o /tmp/claw_playground_main.js
if head -c 20 /tmp/claw_playground_main.js | grep -q '<!DOCTYPE'; then
  echo "error: ${js_path} returned HTML (playground image missing admin assets — pull CI claw-gateway-playground and recreate container)" >&2
  exit 1
fi
echo "playground admin assets ok (${js_path})"

echo "Connectivity check passed. taskId=${TASK_ID}"
