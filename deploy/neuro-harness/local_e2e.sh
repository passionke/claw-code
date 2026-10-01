#!/usr/bin/env bash
# Local two-turn e2e for neuro-harness bins against the spike mock LLM + mock MCP.
# Usage: deploy/neuro-harness/local_e2e.sh opencode|appserver
# Needs: mock_openai.py on $MOCK_LLM (default http://127.0.0.1:18080/v1), built bins in
# rust/target/debug, and NEURO_OPENCODE_BIN / NEURO_CODEX_ACP_BIN pointing at local engines.
# Author: kejiqing
set -euo pipefail

ENGINE="${1:?usage: $0 opencode|appserver}"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO/rust/target/debug/neuro-$ENGINE"
MOCK_LLM="${MOCK_LLM:-http://127.0.0.1:18080/v1}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/nh-e2e-$ENGINE.XXXXXX")"
SESS="$WORK/sess"
PCR="$WORK/project_home_def"
MCP_LOG="$WORK/mcp.ndjson"

mkdir -p "$SESS/.claw" "$PCR/.claw/skills/demo-skill"
echo "Always answer briefly." >"$PCR/CLAUDE.md"
printf -- '---\nname: demo-skill\ndescription: demo\n---\nDemo skill.\n' >"$PCR/.claw/skills/demo-skill/SKILL.md"
cat >"$SESS/.claw/settings.json" <<EOF
{"mcpServers":{"probe":{"command":"python3","args":["$REPO/deploy/neuro-harness/spike/mock_mcp.py"],"env":{"MOCK_MCP_LOG":"$MCP_LOG"}}}}
EOF

run_turn() {
  local turn="$1" prompt="$2"
  cat >"$SESS/gateway-solve-task.json" <<EOF
{"requestId":"req-$turn","turnId":"turn-$turn","sessionId":"sess-e2e","userPrompt":"$prompt",
 "extraSession":{"org_id":"o1"},"maxIterations":8,"timeoutSeconds":120}
EOF
  (cd "$SESS" && env HOME="$SESS" OPENAI_BASE_URL="$MOCK_LLM" OPENAI_API_KEY=claw-tap-cluster \
    CLAW_DEFAULT_MODEL=mock-model CLAW_PROJECT_CONFIG_ROOT="$PCR" \
    "$BIN" gateway-solve-once --task-file "$SESS/gateway-solve-task.json") \
    >"$WORK/turn$turn.stdout" 2>"$WORK/turn$turn.stderr" || true
  grep '^__CLAW_GATEWAY_STDOUT__' "$WORK/turn$turn.stdout" | sed 's/^__CLAW_GATEWAY_STDOUT__//' >"$WORK/turn$turn.events"
}

run_turn 1 "hello"
run_turn 2 "please call tool now"

python3 - "$WORK" "$ENGINE" <<'PY'
import json, sys, pathlib
work, engine = pathlib.Path(sys.argv[1]), sys.argv[2]
def events(n):
    return [json.loads(l) for l in (work / f"turn{n}.events").read_text().splitlines() if l.strip()]
fail = []
for n in (1, 2):
    evs = events(n)
    done = [e for e in evs if e["ev"] == "solve.done"]
    if len(done) != 1 or evs[-1]["ev"] != "solve.done":
        fail.append(f"turn{n}: expected exactly one trailing solve.done, got {[e['ev'] for e in evs]}")
        continue
    d = done[0]
    if d.get("clawExitCode") != 0:
        fail.append(f"turn{n}: solve failed: {d.get('error')}")
        continue
    out = d["outputJson"]
    print(f"turn{n}: completion={out['completionReason']} iterations={out['iterations']} usage={out['usage']} message={out['message'][:80]!r}")
    print(f"turn{n}: events={[e['ev'] + ('/' + e.get('kind', '') if e['ev'].startswith('tool') else '') for e in evs]}")
    if out["harnessEngine"] != engine:
        fail.append(f"turn{n}: harnessEngine={out['harnessEngine']}")
    if out["completionReason"] != "model_end_turn":
        fail.append(f"turn{n}: completionReason={out['completionReason']}")
evs2 = events(2)
starts = [e for e in evs2 if e["ev"] == "tool.start"]
if not starts or starts[0]["kind"] != "mcp":
    fail.append(f"turn2: expected an mcp tool.start, got {starts}")
done2 = [e for e in evs2 if e["ev"] == "solve.done"]
if done2 and done2[0].get("clawExitCode") == 0 and "TOOL_RESULT_SEEN" not in done2[0]["outputJson"]["message"]:
    fail.append("turn2: model did not see the tool result")
mcp = [json.loads(l) for l in (work / "mcp.ndjson").read_text().splitlines()] if (work / "mcp.ndjson").exists() else []
calls = [m for m in mcp if "_meta" in m]
if not calls:
    fail.append("mcp: no tools/call reached the real MCP server")
else:
    extra = calls[-1]["_meta"].get("extra_session", {})
    print(f"mcp: _meta keys={sorted(calls[-1]['_meta'])} extra_session={extra}")
    for k, v in {"org_id": "o1", "_claw_session_id": "sess-e2e", "_claw_turn_id": "turn-2"}.items():
        if extra.get(k) != v:
            fail.append(f"mcp: extra_session[{k}]={extra.get(k)!r}, want {v!r}")
roles = [json.loads(l)["message"]["role"] for l in (work / "sess/.claw/gateway-solve-session.jsonl").read_text().splitlines() if '"message"' in l]
print(f"transcript roles={roles}")
if roles[:2] != ["user", "assistant"] or roles.count("user") != 2 or "tool" not in roles:
    fail.append(f"transcript roles unexpected: {roles}")
state = json.loads((work / "sess/.neuro-harness/state.json").read_text())
print(f"state={state}")
if state["engine"] != engine:
    fail.append(f"state engine={state['engine']}")
print(f"work dir: {work}")
if fail:
    print("FAIL:\n  " + "\n  ".join(fail))
    sys.exit(1)
print("PASS")
PY
