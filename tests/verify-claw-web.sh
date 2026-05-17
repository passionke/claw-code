#!/usr/bin/env bash
# Claw Web unified verifier — deploy后不变异即可跑（见 docs/contracts/VERIFY-CHECKLIST.md）
# Usage: ./tests/verify-claw-web.sh [--tier smoke|full|all|ui]
# Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TIER="${CLAW_VERIFY_TIER:-all}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tier) TIER="${2:?}"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [--tier smoke|full|all|ui]"
      exit 0
      ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

GW_PORT="${GATEWAY_HOST_PORT:-8088}"
if [[ -f "${ROOT}/.env" ]]; then
  # shellcheck disable=SC1090
  val="$(grep -E '^GATEWAY_HOST_PORT=' "${ROOT}/.env" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '"' | tr -d "'")"
  [[ -n "${val}" ]] && GW_PORT="${val}"
fi
export CLAW_GATEWAY_BASE_URL="${CLAW_GATEWAY_BASE_URL:-http://127.0.0.1:${GW_PORT}}"
BRIDGE_PORT="${CLAW_AGUI_BRIDGE_HOST_PORT:-8090}"
BRIDGE_ADDR="127.0.0.1:${BRIDGE_PORT}"

pass() { echo "  ok: $*"; }
fail() { echo "  FAIL: $*" >&2; exit 1; }

section() { echo ""; echo "== $* =="; }

gateway_up() {
  curl -sf "${CLAW_GATEWAY_BASE_URL}/healthz" >/dev/null 2>&1
}

start_bridge() {
  local mock="${1:-0}"
  export CLAW_AGUI_BRIDGE_ADDR="${BRIDGE_ADDR}"
  if [[ "${mock}" == "1" ]]; then
    export CLAW_AGUI_MOCK=1
  else
    unset CLAW_AGUI_MOCK
  fi
  cd "${ROOT}/rust"
  cargo build -q -p ag-ui-claw-bridge
  "${ROOT}/rust/target/debug/ag-ui-claw-bridge" &
  BRIDGE_PID=$!
  for _ in $(seq 1 40); do
    curl -sf "http://${BRIDGE_ADDR}/healthz" >/dev/null 2>&1 && return 0
    sleep 0.15
  done
  fail "bridge did not start on ${BRIDGE_ADDR}"
}

stop_bridge() {
  [[ -n "${BRIDGE_PID:-}" ]] && kill "${BRIDGE_PID}" 2>/dev/null || true
}

run_smoke() {
  section "L0 contracts"
  "${ROOT}/tests/contracts-m0.sh"
  pass "contracts"

  section "Rust unit (agui + auth + bridge)"
  cd "${ROOT}/rust"
  cargo test -q -p ag-ui-claw-bridge -p http-gateway-rs --lib -- agui auth_audit 2>&1
  cargo test -q -p ag-ui-claw-bridge --test mock_gateway_e2e 2>&1
  pass "cargo test"

  section "L1 bridge mock SSE"
  start_bridge 1
  trap 'stop_bridge' EXIT
  OUT=$(curl -sf -N -X POST "http://${BRIDGE_ADDR}/v1/agent/run" \
    -H 'Content-Type: application/json' \
    -d '{"threadId":"v-t1","runId":"v-r1","messages":[{"role":"user","content":"hi"}],"tools":[],"forwardedProps":{"dsId":1}}' \
    | head -30)
  echo "${OUT}" | grep -q 'RUN_STARTED' || fail "L1 missing RUN_STARTED"
  echo "${OUT}" | grep -q 'RUN_FINISHED' || fail "L1 missing RUN_FINISHED"
  pass "L1 mock SSE"
  stop_bridge
  trap - EXIT
}

run_full() {
  section "Gateway health"
  gateway_up || fail "gateway not at ${CLAW_GATEWAY_BASE_URL} — run ./deploy/stack/gateway.sh up"
  pass "gateway healthz"

  section "L2 event tap (dev seed)"
  if [[ "${CLAW_GATEWAY_DEV_AGUI:-}" != "1" ]]; then
    echo "  warn: CLAW_GATEWAY_DEV_AGUI!=1 — set in .env and recreate gateway for dev seed tests"
  fi
  SEED=$(curl -sf -X POST "${CLAW_GATEWAY_BASE_URL}/v1/dev/agui/seed-task" \
    -H 'Content-Type: application/json' \
    -d '{"dsId":1,"outputText":"verify seed"}' 2>/dev/null || true)
  if [[ -z "${SEED}" ]]; then
    fail "dev seed-task failed (need CLAW_GATEWAY_DEV_AGUI=1 on gateway)"
  fi
  TID=$(echo "${SEED}" | sed -n 's/.*"taskId":"\([^"]*\)".*/\1/p')
  [[ -n "${TID}" ]] || fail "parse taskId from seed"
  EVENTS=$(curl -sf "${CLAW_GATEWAY_BASE_URL}/v1/events/${TID}")
  echo "${EVENTS}" | grep -q 'solve.queued' || fail "missing solve.queued in tap"
  echo "${EVENTS}" | grep -q 'text.delta' || fail "missing text.delta in tap"
  echo "${EVENTS}" | grep -q 'solve.finished' || fail "missing solve.finished in tap"
  pass "L2 event tap NDJSON"

  section "L4 interrupt resolve"
  INT=$(curl -sf -X POST "${CLAW_GATEWAY_BASE_URL}/v1/dev/agui/seed-interrupt/${TID}" \
    -H 'Content-Type: application/json' \
    -d '{"reason":"permission"}')
  IID=$(echo "${INT}" | sed -n 's/.*"interruptId":"\([^"]*\)".*/\1/p')
  [[ -n "${IID}" ]] || fail "parse interruptId"
  curl -sf -X POST "${CLAW_GATEWAY_BASE_URL}/v1/interrupts/${IID}/resolve" \
    -H 'Content-Type: application/json' \
    -d '{"decision":"allow_once"}' >/dev/null
  pass "interrupt resolve"
  CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST \
    "${CLAW_GATEWAY_BASE_URL}/v1/interrupts/unknown-interrupt/resolve" \
    -H 'Content-Type: application/json' \
    -d '{"decision":"deny"}')
  [[ "${CODE}" == "404" ]] || fail "unknown interrupt expected 404 got ${CODE}"
  pass "interrupt 404"

  section "L2 bridge → gateway (live)"
  start_bridge 0
  trap 'stop_bridge' EXIT
  OUT=$(curl -sf -N -m 30 -X POST "http://${BRIDGE_ADDR}/v1/agent/run" \
    -H 'Content-Type: application/json' \
    -d "{\"threadId\":\"${TID}\",\"runId\":\"v-run-2\",\"messages\":[{\"role\":\"user\",\"content\":\"verify\"}],\"tools\":[],\"forwardedProps\":{\"dsId\":1}}" \
    | head -40)
  echo "${OUT}" | grep -q 'RUN_STARTED' || fail "live bridge missing RUN_STARTED"
  echo "${OUT}" | grep -qE 'RUN_FINISHED|TEXT_MESSAGE|mock|verify|hello' || fail "live bridge missing content/finish: ${OUT}"
  pass "L2 live bridge SSE"
  stop_bridge
  trap - EXIT

  section "L5 auth (cargo only unless gateway restarted with CLAW_GATEWAY_AUTH=1)"
  cd "${ROOT}/rust"
  cargo test -q -p http-gateway-rs --lib auth_audit 2>&1
  pass "L5 auth unit tests"
}

section "Claw Web verify (tier=${TIER})"
run_ui() {
  "${ROOT}/tests/verify-claw-web-ui.sh"
}

case "${TIER}" in
  smoke) run_smoke ;;
  full) run_full ;;
  all) run_smoke; run_full ;;
  ui) run_smoke; run_full; run_ui ;;
  *) fail "unknown tier: ${TIER}" ;;
esac

echo ""
echo "verify-claw-web: all checks passed (tier=${TIER})"
