#!/usr/bin/env bash
# M4: interrupt resolve API. Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}/rust"
cargo test -p http-gateway-rs --lib agui:: 2>&1

GW="${CLAW_GATEWAY_BASE_URL:-http://127.0.0.1:8080}"
if ! curl -sf "${GW}/healthz" >/dev/null 2>&1; then
  echo "interrupts-e2e: lib tests ok (gateway not running for live resolve)"
  exit 0
fi

# Unknown interrupt → 404
CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST \
  "${GW}/v1/interrupts/test-interrupt-1/resolve" \
  -H 'Content-Type: application/json' \
  -d '{"decision":"allow_once"}')
test "${CODE}" = "404"

echo "interrupts-e2e: ok"
