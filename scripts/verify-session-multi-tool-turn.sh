#!/usr/bin/env bash
# Verify assistant turns in gateway-solve-session.jsonl (multi tool_use per message).
# Author: kejiqing
set -euo pipefail

JSONL="${1:?usage: $0 path/to/.claw/gateway-solve-session.jsonl}"
python3 - "${JSONL}" <<'PY'
import json, sys
from pathlib import Path

path = Path(sys.argv[1])
lines = [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]
turn = 0
max_tools = 0
max_turn = 0
analysis_multi = 0
for o in lines:
    if o.get("type") != "message":
        continue
    msg = o.get("message") or {}
    if msg.get("role") != "assistant":
        continue
    uses = [b for b in (msg.get("blocks") or []) if b.get("type") == "tool_use"]
    if not uses:
        continue
    turn += 1
    names = [b.get("name") or "" for b in uses]
    n = len(uses)
    if n > max_tools:
        max_tools, max_turn = n, turn
    analysis = [x for x in names if x.endswith("mcp_question_then_analysis")]
    flag = " *** MULTI" if n >= 2 else ""
    if len(analysis) >= 2:
        analysis_multi += 1
        flag += " (analysis×" + str(len(analysis)) + ")"
    print(f"assistant_turn#{turn}: {n} tool_use -> {names}{flag}")

print()
print(f"assistant_tool_turns={turn} max_tool_use_in_one_turn={max_tools} (turn#{max_turn})")
print(f"turns_with_2plus_analysis={analysis_multi}")
ok = max_tools >= 2 and analysis_multi >= 1
print("VERIFY_PARALLEL_RULE:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
PY
