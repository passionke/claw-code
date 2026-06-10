#!/usr/bin/env bash
# E2E helpers: project workerIsolationJson for admin-solve-e2e. Author: kejiqing
set -euo pipefail

claw_e2e_set_project_worker_isolation() {
  local port="${1:?port}"
  local proj_id="${2:?proj_id}"
  local mode="${3:?mode}"
  case "${mode}" in
    strict | relaxed) ;;
    *)
      echo "error: worker isolation mode must be strict or relaxed (got ${mode})" >&2
      return 1
      ;;
  esac
  echo "==> e2e set proj=${proj_id} workerIsolationJson.mode=${mode} (gateway :${port})" >&2
  GATEWAY_PORT="${port}" PROJ_ID="${proj_id}" MODE="${mode}" python3 <<'PY'
import json, os, sys, urllib.error, urllib.request

port = os.environ["GATEWAY_PORT"]
proj = int(os.environ["PROJ_ID"])
mode = os.environ["MODE"]
base = f"http://127.0.0.1:{port}"

def load(url):
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.load(r)

cfg = load(f"{base}/v1/project/config/{proj}")
body = {
    "contentRev": cfg.get("contentRev") or "",
    "rulesJson": cfg.get("rulesJson") or [],
    "mcpServersJson": cfg.get("mcpServersJson") or {},
    "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
    "skillsJson": cfg.get("skillsJson") or [],
    "allowedToolsJson": cfg.get("allowedToolsJson") or [],
    "claudeMd": cfg.get("claudeMd"),
    "gitSyncJson": cfg.get("gitSyncJson") or {},
    "solvePreflightJson": cfg.get("solvePreflightJson") or {},
    "solveOrchestrationJson": cfg.get("solveOrchestrationJson") or {},
    "extraSessionFieldsJson": cfg.get("extraSessionFieldsJson") or [],
    "promptLimitsJson": cfg.get("promptLimitsJson") or {},
    "workerIsolationJson": {"mode": mode},
}
req = urllib.request.Request(
    f"{base}/v1/project/config/{proj}",
    data=json.dumps(body, ensure_ascii=False).encode("utf-8"),
    method="PUT",
    headers={"Content-Type": "application/json"},
)
try:
    with urllib.request.urlopen(req, timeout=60):
        pass
except urllib.error.HTTPError as e:
    err = e.read().decode("utf-8", errors="replace")[:800]
    raise SystemExit(f"PUT project config HTTP {e.code}: {err}") from e

got = load(f"{base}/v1/project/config/{proj}")
got_mode = ((got.get("workerIsolationJson") or {}).get("mode") or "").strip()
if got_mode != mode:
    raise SystemExit(f"workerIsolationJson.mode={got_mode!r} expected {mode!r}")
print(f"ok proj={proj} workerIsolationJson.mode={mode}")
PY
}
