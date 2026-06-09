#!/usr/bin/env python3
# Run one solve_async and collect wall clock + session artifacts. Author: kejiqing
import json
import os
import sys
import time
import urllib.error
import urllib.request

GATEWAY = os.environ.get("GATEWAY", "http://127.0.0.1:18088")
WORK_ROOT = os.environ.get(
    "WORK_ROOT",
    "/Users/sm4645/work/claw-code/deploy/stack/claw-workspace",
)
DS_ID = int(os.environ.get("DS_ID", "1"))
QUESTION = os.environ.get("QUESTION", "我最近生意咋样")
STORE_ID = os.environ.get("STORE_ID", "S20241007172800004204")
POLL_SEC = float(os.environ.get("POLL_SEC", "3"))
MAX_POLLS = int(os.environ.get("MAX_POLLS", "400"))
LABEL = os.environ.get("LABEL", "run")


def http(method: str, path: str, body=None, timeout=180):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"{GATEWAY}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode())


def main():
    created = http(
        "POST",
        "/v1/solve_async",
        {
            "projId": DS_ID,
            "userPrompt": QUESTION,
            "extraSession": {
                "store_id": STORE_ID,
                "org_id": os.environ.get("ORG_ID", ""),
                "tenant_code": "GPOS",
                "solution_code": "restaurant",
                "biz_type": "BOSS_REPORT",
            },
        },
    )
    task_id = created["taskId"]
    session_id = created["sessionId"]
    print(f"[{LABEL}] taskId={task_id} sessionId={session_id}", flush=True)

    start = time.time()
    last_sig = None
    final = None
    plan_at = None
    for _ in range(MAX_POLLS):
        task = http("GET", f"/v1/tasks/{task_id}")
        st = task.get("status")
        plan = task.get("planTitle")
        todos = task.get("todos") or []
        done = sum(1 for t in todos if t.get("status") == "done")
        desc = (task.get("currentTaskDesc") or "")[:80]
        sig = (st, plan, done, len(todos), desc)
        if sig != last_sig:
            elapsed = time.time() - start
            print(
                f"[{LABEL}] +{elapsed:5.1f}s status={st} plan={plan!r} todos={done}/{len(todos)} desc={desc!r}",
                flush=True,
            )
            if plan and plan_at is None:
                plan_at = elapsed
            last_sig = sig
        if st in ("succeeded", "failed", "cancelled"):
            final = task
            break
        time.sleep(POLL_SEC)

    wall = time.time() - start
    sh = f"{WORK_ROOT}/ds_{DS_ID}/sessions/{session_id}/.claw"
    result = {
        "label": LABEL,
        "taskId": task_id,
        "sessionId": session_id,
        "wallSec": round(wall, 1),
        "planVisibleSec": round(plan_at, 1) if plan_at else None,
        "status": final.get("status") if final else "timeout",
        "planTitle": final.get("planTitle") if final else None,
        "todoCount": len(final.get("todos") or []) if final else 0,
        "hasReport": final.get("hasReport") if final else False,
        "error": final.get("error") if final else None,
    }

    timings_path = f"{sh}/multi-agent-timings.json"
    try:
        with open(timings_path, encoding="utf-8") as f:
            timings = json.load(f)
        phases = []
        t0 = timings["phases"][0]["startedAtMs"] if timings.get("phases") else 0
        for p in timings.get("phases", []):
            phases.append(
                {
                    "phase": p.get("phase"),
                    "durationSec": round((p["endedAtMs"] - p["startedAtMs"]) / 1000, 1),
                    "offsetSec": round((p["startedAtMs"] - t0) / 1000, 1),
                    "detail": p.get("detail"),
                }
            )
        result["phases"] = phases
        result["phaseTotalSec"] = round((timings["phases"][-1]["endedAtMs"] - t0) / 1000, 1) if phases else None
    except FileNotFoundError:
        result["phases"] = None

    orch_path = f"{sh}/orchestration-events.ndjson"
    try:
        events = []
        with open(orch_path, encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    events.append(json.loads(line))
        result["orchestrationEvents"] = len(events)
        kinds = [e.get("kind") for e in events]
        result["queryStarted"] = kinds.count("query_started")
        result["queryDone"] = kinds.count("query_done")
        result["queryFailed"] = kinds.count("query_failed")
    except FileNotFoundError:
        pass

    tp_path = f"{sh}/task-progress.json"
    try:
        with open(tp_path, encoding="utf-8") as f:
            tp = json.load(f)
        todos = tp.get("todos") or []
        result["taskPhase"] = tp.get("phase")
        result["todosDone"] = sum(1 for t in todos if t.get("status") in ("done", "skipped"))
        result["todosTotal"] = len(todos)
    except FileNotFoundError:
        pass

    ar_dir = f"{sh}/analysis-results"
    try:
        ok_n = fail_n = 0
        for name in os.listdir(ar_dir):
            if not name.endswith(".json"):
                continue
            with open(os.path.join(ar_dir, name), encoding="utf-8") as f:
                row = json.load(f)
            if row.get("ok"):
                ok_n += 1
            else:
                fail_n += 1
        result["analysisOk"] = ok_n
        result["analysisFail"] = fail_n
    except FileNotFoundError:
        pass

    prog_path = f"{sh}/progress-events.ndjson"
    try:
        prog = []
        with open(prog_path, encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    prog.append(json.loads(line))
        mcp_starts = [e for e in prog if e.get("kind") == "mcp_tool_started"]
        result["progressEvents"] = len(prog)
        result["mcpToolStarts"] = len(mcp_starts)
        if mcp_starts:
            t0 = mcp_starts[0]["tsMs"]
            result["mcpSpanSec"] = round((mcp_starts[-1]["tsMs"] - t0) / 1000, 1)
    except FileNotFoundError:
        pass

    out_path = f"/tmp/claw-bench-{LABEL}-{task_id[:8]}.json"
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(result, f, ensure_ascii=False, indent=2)
    print(f"[{LABEL}] result_json={out_path}", flush=True)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
