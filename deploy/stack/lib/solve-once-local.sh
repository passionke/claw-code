#!/usr/bin/env bash
# Host-side one-turn solve (no podman worker / no gateway.sh build).
# Same runtime as pool `claw gateway-solve-once`; for ds_1 tuning see TUNING-LOG.md.
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
# Export all vars from .env so `claw` child sees OPENAI_* / CLAW_* (Author: kejiqing).
set -a
# shellcheck disable=SC1090
source "${REPO_ROOT}/.env"
set +a

# LLM: pool worker uses host.docker.internal; on macOS host use 127.0.0.1 or LOCAL_OPENAI_BASE_URL.
if [[ -n "${LOCAL_OPENAI_BASE_URL:-}" ]]; then
  export OPENAI_BASE_URL="${LOCAL_OPENAI_BASE_URL}"
elif [[ -n "${LOCAL_USE_UPSTREAM_OPENAI:-}" && -n "${UPSTREAM_OPENAI_BASE_URL:-}" ]]; then
  export OPENAI_BASE_URL="${UPSTREAM_OPENAI_BASE_URL}"
elif [[ "${OPENAI_BASE_URL:-}" == *host.docker.internal* ]]; then
  OPENAI_BASE_URL="${OPENAI_BASE_URL//host.docker.internal/127.0.0.1}"
  export OPENAI_BASE_URL
fi

DS_ID="${DS_ID:-1}"
STORE_ID="${STORE_ID:-S20241007172800004204}"
QUESTION="${QUESTION:-最近生意怎样}"
TIMEOUT_SEC="${TIMEOUT_SEC:-600}"
MAX_ITERATIONS="${MAX_ITERATIONS:-64}"
SKIP_BUILD="${SKIP_BUILD:-0}"

# Workspace tree (same mount as stack pool when using deploy/stack).
WORK_ROOT="${CLAW_LOCAL_WORK_ROOT:-${REPO_ROOT}/deploy/stack/claw-workspace}"
DS_BASE="${WORK_ROOT}/ds_${DS_ID}"
SESSION_ID="${SESSION_ID:-local-$(date +%Y%m%d-%H%M%S)}"
SESSION_HOME="${DS_BASE}/sessions/${SESSION_ID}"

# MCP reachable from macOS host (not host.containers.internal).
LOCAL_MCP_URL="${LOCAL_MCP_URL:-http://127.0.0.1:8001/mcp-streamable}"
SETTINGS_SOURCE="${SETTINGS_SOURCE:-}"

CLAW_BIN="${CLAW_BIN:-${REPO_ROOT}/rust/target/release/claw}"
if [[ ! -x "${CLAW_BIN}" ]]; then
  CLAW_BIN="${REPO_ROOT}/rust/target/debug/claw"
fi

allowed_tools_csv="${CLAW_ALLOWED_TOOLS:-bash,mcp__sqlbot-streamable__*,report_progress}"

echo "==> local gateway-solve-once (host, no container)"
echo "    work_root=${WORK_ROOT}"
echo "    session=${SESSION_HOME}"
echo "    question=${QUESTION}"
echo "    OPENAI_BASE_URL=${OPENAI_BASE_URL:-<unset>}"
echo "    LOCAL_MCP_URL=${LOCAL_MCP_URL}"

if [[ "${SKIP_BUILD}" != "1" ]]; then
  echo "==> cargo build -p rusty-claude-cli --bin claw --release (SKIP_BUILD=1 to skip)"
  (cd "${REPO_ROOT}/rust" && cargo build -p rusty-claude-cli --bin claw --release)
  CLAW_BIN="${REPO_ROOT}/rust/target/release/claw"
fi

if [[ ! -x "${CLAW_BIN}" ]]; then
  echo "claw binary not found: ${CLAW_BIN}" >&2
  exit 1
fi

if [[ ! -f "${DS_BASE}/home/CLAUDE.md" ]]; then
  echo "missing ${DS_BASE}/home/CLAUDE.md (provision ds first)" >&2
  exit 1
fi

echo "==> mirror home/CLAUDE.md -> ds_${DS_ID}/CLAUDE.md (pool bind uses root file)"
cp "${DS_BASE}/home/CLAUDE.md" "${DS_BASE}/CLAUDE.md"

mkdir -p "${SESSION_HOME}/.claw"

export WORK_ROOT DS_ID SESSION_ID LOCAL_MCP_URL SETTINGS_SOURCE SESSION_HOME
python3 <<'PY'
import json
import os
from pathlib import Path

work_root = Path(os.environ["WORK_ROOT"])
ds_id = os.environ["DS_ID"]
session_home = Path(os.environ["SESSION_HOME"])
local_mcp_url = os.environ["LOCAL_MCP_URL"]
settings_source = os.environ.get("SETTINGS_SOURCE", "").strip()

def load_settings_template() -> dict:
    if settings_source:
        p = Path(settings_source)
        if not p.is_file():
            raise SystemExit(f"SETTINGS_SOURCE not found: {p}")
        return json.loads(p.read_text(encoding="utf-8"))
    sessions = work_root / f"ds_{ds_id}" / "sessions"
    candidates = sorted(
        sessions.glob("*/.claw/settings.json"),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    if candidates:
        return json.loads(candidates[0].read_text(encoding="utf-8"))
    return {
        "mcpServers": {
            "sqlbot-streamable": {
                "type": "http",
                "url": local_mcp_url,
                "headers": {"MCP-Protocol-Version": "2025-06-18"},
            }
        }
    }

settings = load_settings_template()
servers = settings.setdefault("mcpServers", {})
for name, cfg in list(servers.items()):
    if not isinstance(cfg, dict):
        continue
    url = cfg.get("url")
    if isinstance(url, str) and (
        "host.containers.internal" in url or "host.docker.internal" in url
    ):
        cfg["url"] = local_mcp_url
        print(f"    MCP {name!r} url -> {local_mcp_url}")

out = session_home / ".claw" / "settings.json"
out.write_text(json.dumps(settings, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(f"==> wrote {out}")
PY

export allowed_tools_csv
allowed_json="$(python3 -c '
import json, os
csv = os.environ["allowed_tools_csv"]
print(json.dumps([p.strip() for p in csv.split(",") if p.strip()]))
')"

export DS_ID QUESTION STORE_ID TIMEOUT_SEC MAX_ITERATIONS SESSION_ID allowed_json
python3 <<'PY' >"${SESSION_HOME}/gateway-solve-task.json"
import json, os
allowed = json.loads(os.environ["allowed_json"])
print(json.dumps({
    "requestId": os.environ["SESSION_ID"],
    "userPrompt": os.environ["QUESTION"],
    "timeoutSeconds": int(os.environ["TIMEOUT_SEC"]),
    "maxIterations": int(os.environ["MAX_ITERATIONS"]),
    "allowedTools": allowed,
    "extraSession": {
        "store_id": os.environ["STORE_ID"],
        "tenant_code": "GPOS",
        "solution_code": "restaurant",
        "biz_type": "BOSS_REPORT",
    },
}, ensure_ascii=False, indent=2))
PY

echo "==> claw gateway-solve-once (cwd=${SESSION_HOME})"
export CLAW_GATEWAY_WORK_ROOT="${WORK_ROOT}"
set +e
(
  cd "${SESSION_HOME}"
  "${CLAW_BIN}" gateway-solve-once --task-file gateway-solve-task.json \
    | tee "${SESSION_HOME}/gateway-solve-once.stdout.json"
)
solve_rc=$?
set -e

events_file="${SESSION_HOME}/.claw/progress-events.ndjson"
echo ""
echo "==> progress-events (${events_file})"
if [[ -f "${events_file}" ]]; then
  python3 - "${events_file}" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
lines = [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]
starts = [e for e in lines if e.get("kind") == "mcp_tool_started"]
print(f"    events={len(lines)}  mcp_starts={len(starts)}")
for i, e in enumerate(starts):
    gap = (e["tsMs"] - starts[i - 1]["tsMs"]) / 1000 if i else 0
    msg = e.get("message", "")[:60]
    print(f"    start[{i}] +{gap:.1f}s  {msg}")
completed = [e.get("message") for e in lines if e.get("kind") == "mcp_tool_completed"]
if completed:
    print(f"    last completed: {completed[-1]!r}")
PY
else
  echo "    (no progress-events.ndjson)"
fi

jsonl="${SESSION_HOME}/.claw/gateway-solve-session.jsonl"
if [[ -f "${jsonl}" ]]; then
  echo ""
  echo "==> MCP tool mentions in session jsonl"
  python3 - "${jsonl}" <<'PY'
import json, re, sys
from collections import Counter
from pathlib import Path
text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
tools = re.findall(r"mcp__sqlbot[^\s\"']+", text)
for name, c in Counter(tools).most_common(12):
    print(f"    {c}x {name}")
PY
fi

echo ""
echo "==> session artifacts: ${SESSION_HOME}"
echo "    task: gateway-solve-task.json"
echo "    stdout: gateway-solve-once.stdout.json"
if [[ "${solve_rc}" -ne 0 ]]; then
  echo "gateway-solve-once exited ${solve_rc}" >&2
  exit "${solve_rc}"
fi

echo "==> done"
