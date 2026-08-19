#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Run solve_async + wait until hasReport=true,
then open biz_advice_report SSE stream immediately and dump ALL biz.report.delta events
with accurate client receive timestamps.

This avoids the “post-completion replay gap” (after completion, some deployments only
return start/done and no delta replay).

Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import time
import urllib.parse
import urllib.request
from pathlib import Path


def sse_stream_events(url: str, *, session_started_ms: float) -> list[dict]:
    """
    Minimal SSE reader:
      - timestamps each `biz.report.delta` when the event is fully assembled (blank line)
    """
    t0 = time.monotonic()
    deltas: list[dict] = []
    current_event = None
    data_lines: list[str] = []

    req = urllib.request.Request(url, method="GET")
    with urllib.request.urlopen(req, timeout=180) as resp:
        # Keep reading line-by-line to preserve receive timing.
        while True:
            line = resp.readline()
            if not line:
                break
            s = line.decode("utf-8", errors="replace").rstrip("\n")

            if s.startswith("event:"):
                current_event = s[len("event:") :].strip()
            elif s.startswith("data:"):
                data_lines.append(s[len("data:") :].lstrip())
            elif s == "":
                if current_event and data_lines:
                    data = "\n".join(data_lines)
                    if current_event == "biz.report.delta":
                        try:
                            obj = json.loads(data)
                        except json.JSONDecodeError:
                            obj = {"_parseError": True, "raw": data}
                        obj["_clientRecvMs"] = int((time.monotonic() - t0) * 1000)
                        deltas.append(obj)
                    elif current_event == "biz.report.done":
                        # We can stop after done.
                        break

                current_event = None
                data_lines = []

    return deltas


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gateway", default="http://127.0.0.1:18088")
    ap.add_argument("--proj-id", type=int, default=1)
    ap.add_argument("--store-id", default="S20241007172800004204")
    ap.add_argument("--question", default="最近我们的router 角色改造测试长输出，请保持流式自然。")
    ap.add_argument("--timeout-sec", type=float, default=300.0)
    ap.add_argument("--poll-sec", type=float, default=2.0)
    ap.add_argument("--dump-path", default="./client-deltas.json")
    args = ap.parse_args()

    base = args.gateway.rstrip("/")

    # init
    init_url = f"{base}/v1/init"
    req_init = urllib.request.Request(init_url, method="POST")
    req_init.add_header("Content-Type", "application/json")
    req_init.data = json.dumps({"projId": args.proj_id}).encode("utf-8")
    urllib.request.urlopen(req_init, timeout=30).read()

    body = {
        "projId": int(args.proj_id),
        "userPrompt": args.question,
        "extraSession": {
            "store_id": args.store_id,
            "org_id": "",
            "tenant_code": "GPOS",
            "solution_code": "restaurant",
            "biz_type": "BOSS_REPORT",
        },
    }
    solve_url = f"{base}/v1/solve_async"
    req_solve = urllib.request.Request(solve_url, method="POST")
    req_solve.add_header("Content-Type", "application/json")
    req_solve.data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    solve_obj = json.loads(urllib.request.urlopen(req_solve, timeout=60).read().decode("utf-8"))

    task_id = solve_obj["taskId"]
    session_id = solve_obj["sessionId"]
    turn_id = solve_obj["turnId"]
    print(json.dumps({"taskId": task_id, "sessionId": session_id, "turnId": turn_id}, ensure_ascii=False))

    # wait hasReport=true
    has_report = False
    deadline = time.monotonic() + args.timeout_sec
    while time.monotonic() < deadline:
        task_url = f"{base}/v1/tasks/{task_id}"
        task_obj = json.loads(urllib.request.urlopen(task_url, timeout=30).read().decode("utf-8"))
        status = task_obj.get("status")
        has_report = bool(task_obj.get("hasReport"))
        if has_report:
            break
        if status in ("succeeded", "failed", "cancelled"):
            break
        time.sleep(args.poll_sec)

    if not has_report:
        raise SystemExit("hasReport never became true in time")

    # open SSE immediately
    q = urllib.parse.urlencode(
        {"sessionId": session_id, "turnId": turn_id, "projId": str(args.proj_id), "stream": "true"}
    )
    sse_url = f"{base}/v1/biz_advice_report?{q}"
    print(json.dumps({"sseUrl": sse_url}, ensure_ascii=False))

    deltas = sse_stream_events(sse_url, session_started_ms=time.time())

    # normalize to align_sse_trunks expected shape
    simplified = []
    for r in deltas:
        seq = r.get("seq")
        server_delta_ms = r.get("serverDeltaMs")
        text = r.get("text")
        tl = r.get("textLen")
        if tl is None and text is not None:
            tl = len(str(text))
        simplified.append(
            {
                "seq": seq,
                "serverDeltaMs": server_delta_ms,
                "clientDeltaMs": r.get("_clientRecvMs"),
                "textLen": tl,
                "text": text,
                "emitSeq": r.get("emitSeq"),
            }
        )

    out = Path(args.dump_path)
    out.write_text(json.dumps(simplified, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"dumped": str(out), "deltaCount": len(simplified)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

