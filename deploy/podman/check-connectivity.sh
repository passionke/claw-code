#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "missing ${ENV_FILE} — copy ${REPO_ROOT}/.env.example" >&2
  exit 1
fi

# shellcheck disable=SC1090
source "${ENV_FILE}"

echo "[1/3] gateway healthz"
curl -fsS "http://127.0.0.1:${GATEWAY_HOST_PORT}/healthz" >/tmp/claw_gateway_healthz.json
cat /tmp/claw_gateway_healthz.json
echo

echo "[2/3] solve_async smoke"
TASK_JSON="$(curl -fsS -X POST "http://127.0.0.1:${GATEWAY_HOST_PORT}/v1/solve_async" \
  -H "Content-Type: application/json" \
  -d '{"dsId":1,"userPrompt":"connectivity check"}')"
echo "${TASK_JSON}"
TASK_ID="$(printf "%s" "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["taskId"])')"

echo "[3/3] verify MCP list is available"
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

echo "Connectivity check passed. taskId=${TASK_ID}"
