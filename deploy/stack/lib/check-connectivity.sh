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

if ! claw_ensure_host_pool_running "${PODMAN_DIR}"; then
  echo "error: host pool not running — Admin solve_async will 503" >&2
  exit 1
fi

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
if ! podman exec "${GW_CTN}" curl -fsS --max-time 5 \
  "http://host.containers.internal:${POOL_HTTP_PORT}/healthz/live-report" \
  >/tmp/claw_pool_health.json; then
  echo "error: gateway cannot reach pool HTTP — same failure as Admin 503" >&2
  echo "hint: ./deploy/stack/gateway.sh pool-up" >&2
  exit 1
fi
python3 -c 'import json; d=json.load(open("/tmp/claw_pool_health.json")); print("pool live-report ok", d.get("ok", d))' 2>/dev/null || cat /tmp/claw_pool_health.json
echo

if claw_pool_daemon_on_host; then
  base="$(claw_pool_http_base_url "${PODMAN_DIR}")" || exit 1
  echo "[2b/5] host pool HTTP (127.0.0.1:${CLAW_POOL_HTTP_PORT:-9944})"
  claw_assert_host_pool_http_ready "${PODMAN_DIR}/.claw-pool-rpc" || exit 1
  echo "host pool HTTP ok"
  echo "[2c/5] pool HTTP from gateway-rs container (${base})"
  claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}" || exit 1
  echo "gateway → pool HTTP ok"
  echo
else
  echo "error: compose pool sidecar removed in pool v1; set CLAW_POOL_HOST_DAEMON=1 or run ./deploy/stack/gateway.sh pool-up" >&2
  exit 1
fi

echo "[3/5] solve_async smoke (extraSession from ds 1 project config when defined)"
if claw_pool_daemon_on_host; then
  claw_assert_host_pool_rpc_ready "${PODMAN_DIR}/.claw-pool-rpc" || {
    echo "error: refuse solve_async smoke — host pool RPC not ready" >&2
    exit 1
  }
  claw_wait_gateway_pool_rpc_ready "${PODMAN_DIR}" || exit 1
fi
claw_wait_gateway_claw_tap_ready || exit 1
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
TURN_ID="$(printf "%s" "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["turnId"])')"

echo "[3b/5] poll solve_async until succeeded (same bar as Admin)"
for _ in $(seq 1 120); do
  sleep 2
  TASK_POLL="$(curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/tasks/${TASK_ID}")"
  TASK_ST="$(printf '%s' "${TASK_POLL}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["status"])')"
  echo "poll status=${TASK_ST}"
  if [[ "${TASK_ST}" == "succeeded" || "${TASK_ST}" == "failed" ]]; then
    if [[ "${TASK_ST}" != "succeeded" ]]; then
      printf '%s\n' "${TASK_POLL}" | python3 -m json.tool >&2
      echo "error: connectivity solve_async failed taskId=${TASK_ID} turnId=${TURN_ID}" >&2
      exit 1
    fi
    printf '%s\n' "${TASK_POLL}" | python3 -m json.tool
    break
  fi
done
if [[ "${TASK_ST:-}" != "succeeded" ]]; then
  echo "error: timeout waiting solve_async taskId=${TASK_ID} turnId=${TURN_ID}" >&2
  exit 1
fi

echo "[3c/5] turn tools API ↔ PG transcript (when solve used tools)"
SESSION_ID="$(printf '%s' "${TASK_POLL}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sessionId"])')"
TOOLS_CHAIN_JSON="$(mktemp)"
TIMELINE_JSON="$(mktemp)"
curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/sessions/${SESSION_ID}/turns/${TURN_ID}/timeline?ds_id=1" \
  -o "${TIMELINE_JSON}" || true
curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/sessions/${SESSION_ID}/turns/${TURN_ID}/tools?ds_id=1" \
  -o "${TOOLS_CHAIN_JSON}"
python3 - "${TIMELINE_JSON}" "${TOOLS_CHAIN_JSON}" <<'PY'
import json, sys

timeline_path, tools_path = sys.argv[1], sys.argv[2]
tool_segments = 0
try:
    tl = json.load(open(timeline_path))
    for lane in (tl.get("timeline") or {}).get("lanes") or []:
        lid = (lane.get("id") or "").lower()
        if "tool" in lid:
            tool_segments += len(lane.get("segments") or [])
except (OSError, json.JSONDecodeError, TypeError):
    tool_segments = 0

tools_body = json.load(open(tools_path))
tools = tools_body.get("tools") or []
print(f"timeline_tool_segments={tool_segments} tools_api_count={len(tools)}")
if tool_segments > 0 and len(tools) == 0:
    raise SystemExit(
        "regression: timeline shows tool calls but GET .../tools returned empty "
        "(PG transcript not wired to tools API)"
    )
if tool_segments > 0:
    print("turn tools chain ok (timeline + tools API agree)")
else:
    print("skip tools chain assert (solve had no tool lane — ping-style smoke)")
PY

TASK_POLL_JSON="$(mktemp)"
curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/tasks/${TASK_ID}" -o "${TASK_POLL_JSON}"
python3 - "${TIMELINE_JSON}" "${TASK_POLL_JSON}" <<'PY'
import json, sys
timeline_path, task_path = sys.argv[1], sys.argv[2]
progress_segments = 0
try:
    tl = json.load(open(timeline_path))
    for lane in (tl.get("timeline") or {}).get("lanes") or []:
        lid = (lane.get("id") or "").lower()
        if "progress" in lid or "report" in lid:
            progress_segments += len(lane.get("segments") or [])
except (OSError, json.JSONDecodeError, TypeError):
    progress_segments = 0
task = json.load(open(task_path))
hist = task.get("progressHistory") or []
todos = task.get("todos") or []
print(f"timeline_progress_segments={progress_segments} progressHistory={len(hist)} todos={len(todos)}")
if progress_segments > 0 and len(hist) == 0:
    raise SystemExit(
        "regression: timeline shows progress but GET /v1/tasks progressHistory empty "
        "(PG progress not wired to task status API)"
    )
if progress_segments > 0:
    print("task progress chain ok (timeline + progressHistory agree)")
else:
    print("skip progressHistory assert (solve had no progress lane)")
PY

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
