#!/usr/bin/env bash
# M3: stack smoke — bridge mock mode + gateway health (when running). Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Bridge mock SSE (no gateway required)
export CLAW_AGUI_MOCK=1
BRIDGE_PORT="${CLAW_AGUI_BRIDGE_PORT:-18090}"
BRIDGE_ADDR="127.0.0.1:${BRIDGE_PORT}"
export CLAW_AGUI_BRIDGE_ADDR="${BRIDGE_ADDR}"

cd "${ROOT}/rust"
cargo build -q -p ag-ui-claw-bridge
"${ROOT}/rust/target/debug/ag-ui-claw-bridge" &
BRIDGE_PID=$!
trap 'kill "${BRIDGE_PID}" 2>/dev/null || true' EXIT

for _ in $(seq 1 30); do
  if curl -sf "http://${BRIDGE_ADDR}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

OUT=$(curl -sf -N -X POST "http://${BRIDGE_ADDR}/v1/agent/run" \
  -H 'Content-Type: application/json' \
  -d '{"threadId":"e2e-t1","runId":"e2e-r1","messages":[{"role":"user","content":"hi"}],"tools":[],"forwardedProps":{"dsId":1}}' \
  | head -20)
echo "${OUT}" | grep -q 'RUN_STARTED' || { echo "missing RUN_STARTED"; exit 1; }
echo "${OUT}" | grep -q 'RUN_FINISHED' || { echo "missing RUN_FINISHED"; exit 1; }

GW="${CLAW_GATEWAY_BASE_URL:-http://127.0.0.1:8080}"
if curl -sf "${GW}/healthz" >/dev/null 2>&1; then
  echo "gateway healthz: ok"
else
  echo "gateway not running (skip live gateway check)"
fi

echo "e2e-claw-web-stack: ok"
