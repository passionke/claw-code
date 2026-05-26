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

PLAYGROUND_PORT="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"

echo "[1/4] gateway healthz"
curl -fsS "http://127.0.0.1:${GATEWAY_HOST_PORT}/healthz" >/tmp/claw_gateway_healthz.json
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

echo "[2/4] solve_async smoke"
TASK_JSON="$(curl -fsS -X POST "http://127.0.0.1:${GATEWAY_HOST_PORT}/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d '{"dsId":1,"userPrompt":"connectivity check"}')"
echo "${TASK_JSON}"
TASK_ID="$(printf "%s" "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["taskId"])')"

echo "[3/4] verify MCP list is available"
MCP_JSON="$(curl -fsS "http://127.0.0.1:${GATEWAY_HOST_PORT}/v1/mcp/injected/1")"
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

echo "[4/4] gateway-playground UI"
for _ in $(seq 1 20); do
  if curl -fsS "http://127.0.0.1:${PLAYGROUND_PORT}/__config__" >/tmp/claw_playground_config.json 2>/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:${PLAYGROUND_PORT}/__config__" >/tmp/claw_playground_config.json
python3 -c 'import json; c=json.load(open("/tmp/claw_playground_config.json")); assert c.get("defaultGatewayBase"); print("playground ok, defaultGatewayBase=", c["defaultGatewayBase"])'

echo "Connectivity check passed. taskId=${TASK_ID}"
