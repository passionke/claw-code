#!/usr/bin/env bash
# HTTP smoke: concurrent reads during solve_async; workDir is canonical ds_home.
# - Read paths stay fast while an async solve runs (no ds_lock starvation on GET CLAUDE.md).
# - Successful solve: workDir should be .../ds_{id} (not a separate sessions/ tree).
# - `CLAW_SOLVE_ISOLATION` defaults to podman_pool (needs podman + CLAW_PODMAN_IMAGE, or set docker_pool + CLAW_DOCKER_*).
#
# Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
WORK_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/claw-gateway-plan.XXXXXX")"
REGISTRY="$REPO_ROOT/rust/crates/http-gateway-rs/datasources.example.yaml"
BIN="$RUST_DIR/target/debug/http-gateway-rs"
CLAW_BIN="${CLAW_BIN:-$RUST_DIR/target/debug/claw}"

cleanup() {
  if [[ -n "${GATEWAY_PID:-}" ]] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
    kill "$GATEWAY_PID" 2>/dev/null || true
    wait "$GATEWAY_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK_ROOT"
}
trap cleanup EXIT

cd "$RUST_DIR"
if [[ "${SKIP_GATEWAY_BUILD:-}" != "1" ]]; then
  echo "[plan] cargo build -p http-gateway-rs -p rusty-claude-cli (set SKIP_GATEWAY_BUILD=1 to skip)..."
  cargo build -p http-gateway-rs -p rusty-claude-cli
fi

export CLAW_HTTP_ADDR="127.0.0.1:$PORT"
export CLAW_WORK_ROOT="$WORK_ROOT"
export CLAW_DS_REGISTRY="$REGISTRY"
export CLAW_BIN
# Match product default: container pool (see .env.example). Use docker_pool + CLAW_DOCKER_* if you run Docker workers.
export CLAW_SOLVE_ISOLATION="${CLAW_SOLVE_ISOLATION:-podman_pool}"
export CLAW_PODMAN_IMAGE="${CLAW_PODMAN_IMAGE:-claw-gateway-worker:local}"
export CLAW_PROJECTS_GIT_URL="${CLAW_PROJECTS_GIT_URL:-git@github.com:passionke/claw-code-projects.git}"
export CLAW_PROJECTS_GIT_BRANCH="${CLAW_PROJECTS_GIT_BRANCH:-main}"
export CLAW_PROJECTS_GIT_AUTHOR="${CLAW_PROJECTS_GIT_AUTHOR:-kejiqing <kejiqing@local>}"

GW_LOG="$WORK_ROOT/gateway.log"
"$BIN" >>"$GW_LOG" 2>&1 &
GATEWAY_PID=$!

BASE="http://127.0.0.1:$PORT"
CURL_MAX=25
for _ in $(seq 1 80); do
  if curl -sf --max-time "$CURL_MAX" "$BASE/healthz" >/dev/null; then
    break
  fi
  sleep 0.05
done
if ! curl -sf --max-time "$CURL_MAX" "$BASE/healthz" >/dev/null; then
  echo "gateway did not become ready on $BASE (see $GW_LOG)" >&2
  exit 1
fi

curl -sf --max-time "$CURL_MAX" -X POST "$BASE/v1/init" -H 'Content-Type: application/json' -d '{"dsId":10}' >/dev/null

echo "[plan] concurrent GET /v1/project/claude/10 while solve_async worker runs..."
SECONDS=0
ASYNC_OUT="$WORK_ROOT/solve_async.json"
rm -f "$ASYNC_OUT"
curl -sf --max-time "$CURL_MAX" -X POST "$BASE/v1/solve_async" \
  -H 'Content-Type: application/json' \
  -d '{"dsId":10,"userPrompt":"say hi in one word","timeoutSeconds":120}' \
  -o "$ASYNC_OUT" &
CURL_ASYNC_PID=$!

CURL_PIDS=("$CURL_ASYNC_PID")
for i in $(seq 1 30); do
  (curl -sf --max-time "$CURL_MAX" "$BASE/v1/project/claude/10" >/dev/null && : >"$WORK_ROOT/read_ok_$i") &
  CURL_PIDS+=($!)
done
for pid in "${CURL_PIDS[@]}"; do
  wait "$pid"
done
if [[ ! -s "$ASYNC_OUT" ]]; then
  echo "FAIL: solve_async returned no body (see $GW_LOG)" >&2
  exit 1
fi
TASK_JSON="$(cat "$ASYNC_OUT")"
TASK_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["taskId"])' "$TASK_JSON")"
READ_ELAPSED="$SECONDS"
OK_READS="$(find "$WORK_ROOT" -maxdepth 1 -name 'read_ok_*' 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$OK_READS" != "30" ]]; then
  echo "FAIL: only $OK_READS/30 parallel reads succeeded (see $GW_LOG)" >&2
  exit 1
fi
if [[ "$READ_ELAPSED" -gt 12 ]]; then
  echo "FAIL: parallel reads took ${READ_ELAPSED}s (expected quick; possible lock contention)" >&2
  exit 1
fi
echo "[plan] 30 parallel reads finished in ${READ_ELAPSED}s"

for _ in $(seq 1 180); do
  ST="$(curl -sf --max-time "$CURL_MAX" "$BASE/v1/tasks/$TASK_ID" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')"
  if [[ "$ST" != "running" && "$ST" != "queued" ]]; then
    break
  fi
  sleep 0.2
done

RECORD="$(curl -sf --max-time "$CURL_MAX" "$BASE/v1/tasks/$TASK_ID")"
ST="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' <<<"$RECORD")"
echo "[plan] async task status: $ST"

if [[ "$ST" == "succeeded" ]]; then
  WD="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["workDir"])' <<<"$RECORD")"
  if [[ "$WD" != *"/ds_10"* ]] && [[ "$WD" != *"\\ds_10"* ]]; then
    echo "FAIL: expected workDir to contain ds_10 (canonical ds_home), got: $WD" >&2
    exit 1
  fi
  echo "[plan] workDir is ds_home: $WD"
else
  echo "[plan] solve did not succeed (often missing model API keys); skipping workDir shape assert"
fi

echo "[plan] GET /v1/skills/10 list + GET /v1/skills/10/plan_skill..."
mkdir -p "$WORK_ROOT/ds_10/home/skills/plan_skill"
printf '%s\n' '---' 'name: plan_skill' '---' 'skill body line' >"$WORK_ROOT/ds_10/home/skills/plan_skill/SKILL.md"
SK_JSON="$(curl -sf --max-time "$CURL_MAX" "$BASE/v1/skills/10")"
python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("ds_id")==10; assert isinstance(d.get("skills"),list); assert any(x.get("skill_name")=="plan_skill" for x in d["skills"])' <<<"$SK_JSON"
ONE_JSON="$(curl -sf --max-time "$CURL_MAX" "$BASE/v1/skills/10/plan_skill")"
python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("skill_name")=="plan_skill"; assert "skill body line" in d.get("skill_content","")' <<<"$ONE_JSON"
if curl -sf --max-time "$CURL_MAX" "$BASE/v1/skills/10/missing_skill_xyz" >/dev/null 2>&1; then
  echo "FAIL: expected 404 for missing skill" >&2
  exit 1
fi
HTTP_CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time "$CURL_MAX" "$BASE/v1/skills/10/missing_skill_xyz")"
if [[ "$HTTP_CODE" != "404" ]]; then
  echo "FAIL: missing skill should return 404, got $HTTP_CODE" >&2
  exit 1
fi

sleep 0.3
if [[ -d "$WORK_ROOT/sessions" ]]; then
  LEFT="$(find "$WORK_ROOT/sessions" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l | tr -d ' ')"
  if [[ "$LEFT" != "0" ]]; then
    echo "FAIL: unexpected files under $WORK_ROOT/sessions (count=$LEFT)" >&2
    exit 1
  fi
fi
echo "[plan] no stray work_root/sessions content"

echo "OK — http-gateway-plan-scenarios"
