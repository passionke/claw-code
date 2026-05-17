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

echo "[1/3] gateway healthz"
curl -fsS "http://127.0.0.1:${GATEWAY_HOST_PORT}/healthz" >/tmp/claw_gateway_healthz.json
cat /tmp/claw_gateway_healthz.json
echo
python3 - <<'PY'
import json
import sys

h = json.load(open("/tmp/claw_gateway_healthz.json"))
if not h.get("poolRpcRemote"):
    print(
        "error: healthz poolRpcRemote is not true — gateway is not using host claw-pool-daemon.",
        "Run ./deploy/stack/gateway.sh down && ./deploy/stack/gateway.sh up",
        "Never start gateway without deploy/stack/.claw-pool-rpc/gateway.env (in-container podman is forbidden).",
        file=sys.stderr,
    )
    sys.exit(1)
if not h.get("poolRpcTcp"):
    print("error: healthz poolRpcTcp missing", file=sys.stderr)
    sys.exit(1)
print(f"pool RPC ok: {h.get('poolRpcTcp')}")
PY

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
