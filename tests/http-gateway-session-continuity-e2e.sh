#!/usr/bin/env bash
# E2E: session SQLite + same workDir on continuation + 400 for unknown sessionId.
# Uses in-process gateway (no container pool). Requires OPENAI_* if you want solve to succeed;
# without keys, healthz / init / 400 path still validate wiring.
#
# Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
WORK_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/claw-gateway-sess-e2e.XXXXXX")"
SESSION_DB="${WORK_ROOT}/gateway-sessions.sqlite"
REGISTRY="$REPO_ROOT/rust/crates/http-gateway-rs/datasources.example.yaml"
BIN="${BIN:-$RUST_DIR/target/release/http-gateway-rs}"
CLAW_BIN="${CLAW_BIN:-$RUST_DIR/target/release/claw}"

cleanup() {
  if [[ -n "${GATEWAY_PID:-}" ]] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
    kill "$GATEWAY_PID" 2>/dev/null || true
    wait "$GATEWAY_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK_ROOT"
}
trap cleanup EXIT

if [[ ! -x "$BIN" ]]; then
  echo "missing $BIN — run: cd $RUST_DIR && cargo build --release -p http-gateway-rs -p rusty-claude-cli" >&2
  exit 1
fi
if [[ ! -x "$CLAW_BIN" ]]; then
  echo "missing $CLAW_BIN — same cargo build as above" >&2
  exit 1
fi

export CLAW_HTTP_ADDR="127.0.0.1:$PORT"
export CLAW_WORK_ROOT="$WORK_ROOT"
export CLAW_GATEWAY_SESSION_DB="$SESSION_DB"
export CLAW_DS_REGISTRY="$REGISTRY"
export CLAW_BIN
export CLAW_SOLVE_ISOLATION=inprocess
export CLAW_PROJECTS_GIT_URL="${CLAW_PROJECTS_GIT_URL:-git@github.com:passionke/claw-code-projects.git}"
export CLAW_PROJECTS_GIT_BRANCH="${CLAW_PROJECTS_GIT_BRANCH:-main}"
export CLAW_PROJECTS_GIT_AUTHOR="${CLAW_PROJECTS_GIT_AUTHOR:-kejiqing <kejiqing@local>}"

GW_LOG="$WORK_ROOT/gateway.log"
"$BIN" >>"$GW_LOG" 2>&1 &
GATEWAY_PID=$!

BASE="http://127.0.0.1:$PORT"
CURL_MAX=30
for _ in $(seq 1 100); do
  if curl -sf --max-time "$CURL_MAX" "$BASE/healthz" >/dev/null; then
    break
  fi
  sleep 0.05
done
if ! curl -sf --max-time "$CURL_MAX" "$BASE/healthz" >/dev/null; then
  echo "gateway did not become ready on $BASE (see $GW_LOG)" >&2
  exit 1
fi

HZ="$(curl -sf --max-time "$CURL_MAX" "$BASE/healthz")"
python3 -c 'import json,sys; o=json.loads(sys.argv[1]); assert o.get("ok") is True, o; p=o.get("sessionDbPath",""); assert "gateway-sessions.sqlite" in p, o' "$HZ"
echo "[e2e] healthz ok; sessionDbPath under work root"

curl -sf --max-time "$CURL_MAX" -X POST "$BASE/v1/init" -H 'Content-Type: application/json' -d '{"dsId":1}' >/dev/null
echo "[e2e] init ds 1 ok"

HTTP_UNKNOWN="$(curl -sS -o "$WORK_ROOT/body400.json" -w '%{http_code}' --max-time "$CURL_MAX" -X POST "$BASE/v1/solve" \
  -H 'Content-Type: application/json' \
  -d '{"dsId":1,"userPrompt":"x","sessionId":"definitely-not-in-sqlite-zzzz"}' || true)"
if [[ "$HTTP_UNKNOWN" != "400" ]]; then
  echo "FAIL: unknown sessionId expected HTTP 400, got $HTTP_UNKNOWN body=$(cat "$WORK_ROOT/body400.json")" >&2
  exit 1
fi
python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert "detail" in d' "$WORK_ROOT/body400.json"
echo "[e2e] unknown sessionId -> 400 ok"

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  echo "[e2e] OPENAI_API_KEY unset — skipping two-round solve / same workDir check"
  echo "OK — http-gateway-session-continuity-e2e (partial)"
  exit 0
fi

R1="$(curl -sf --max-time 180 -X POST "$BASE/v1/solve" \
  -H 'Content-Type: application/json' \
  -d '{"dsId":1,"userPrompt":"Reply with exactly one word: alpha","timeoutSeconds":120}')"
SID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["sessionId"])' "$R1")"
WD1="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["workDir"])' "$R1")"
echo "[e2e] round1 sessionId=$SID"

R2="$(curl -sf --max-time 180 -X POST "$BASE/v1/solve" \
  -H 'Content-Type: application/json' \
  -d "$(SID="$SID" python3 -c 'import json,os; print(json.dumps({"dsId":1,"userPrompt":"Reply with exactly one word: beta","sessionId":os.environ["SID"],"timeoutSeconds":120}))')")"
WD2="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["workDir"])' "$R2")"
if [[ "$WD1" != "$WD2" ]]; then
  echo "FAIL: continuation workDir mismatch:1=$WD1 2=$WD2" >&2
  exit 1
fi
echo "[e2e] same workDir on continuation: $WD1"

JSONL="${WD1}/.claw/gateway-solve-session.jsonl"
if [[ ! -f "$JSONL" ]]; then
  echo "FAIL: expected transcript at $JSONL" >&2
  exit 1
fi
echo "[e2e] transcript jsonl exists"

echo "OK — http-gateway-session-continuity-e2e"
