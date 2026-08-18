#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Count biz.report.delta on router vs specialist SSE during one live solve. Author: kejiqing"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field

GW = os.environ.get("GW", "http://127.0.0.1:18088").rstrip("/")
ROUTER_PROJ = int(os.environ.get("ROUTER_PROJ", "99010"))
OPS_PROJ = int(os.environ.get("OPS_PROJ", "99012"))
PROMPT = os.environ.get(
    "PROMPT", "2026-08-10的销售额和订单量是多少？"
)
EXTRA = {
    "store_id": os.environ.get("STORE_ID", "S20240930134500007303"),
    "org_id": os.environ.get("ORG_ID", "O20240930134500007302"),
    "tenant_code": os.environ.get("TENANT_CODE", "GPOS"),
}
PG_URL = os.environ.get(
    "CLAW_GATEWAY_DATABASE_URL",
    "postgres://claw_gateway:clawGw9Dev_Pg@10.22.28.94:5433/claw_gateway",
)


def http_json(method: str, path: str, body: dict | None = None, timeout: float = 120) -> dict:
    data = None
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(f"{GW}{path}", data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def pg_query(sql: str) -> list[list[str]]:
    env = os.environ.copy()
    if PG_URL.startswith("postgres://"):
        env["PGPASSWORD"] = PG_URL.split("@")[0].split(":")[-1]
    host_part = PG_URL.split("@")[1]
    host, rest = host_part.split(":")
    port_db = rest.split("/")
    port = port_db[0]
    db = port_db[1] if len(port_db) > 1 else "claw_gateway"
    user = PG_URL.split("://")[1].split(":")[0]
    out = subprocess.run(
        [
            "psql",
            "-h",
            host,
            "-p",
            port,
            "-U",
            user,
            "-d",
            db,
            "-At",
            "-F",
            "\t",
            "-c",
            sql,
        ],
        capture_output=True,
        text=True,
        env=env,
        check=False,
    )
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "psql failed")
    rows: list[list[str]] = []
    for line in out.stdout.splitlines():
        if line.strip():
            rows.append(line.split("\t"))
    return rows


@dataclass
class SseCount:
    label: str
    deltas: list[str] = field(default_factory=list)
    events: list[str] = field(default_factory=list)
    error: str | None = None
    done_payload: dict | None = None

    def run(self, session_id: str, turn_id: str, proj_id: int, stop: threading.Event) -> None:
        q = urllib.parse.urlencode(
            {
                "sessionId": session_id,
                "turnId": turn_id,
                "projId": str(proj_id),
                "stream": "true",
            }
        )
        url = f"{GW}/v1/biz_advice_report?{q}"
        req = urllib.request.Request(url, headers={"Accept": "text/event-stream"})
        try:
            with urllib.request.urlopen(req, timeout=600) as resp:
                event = None
                data_lines: list[str] = []
                while not stop.is_set():
                    raw = resp.readline()
                    if not raw:
                        break
                    line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                    if line == "":
                        if event and data_lines:
                            payload = json.loads("\n".join(data_lines))
                            self.events.append(event)
                            if event == "biz.report.delta":
                                text = payload.get("text") or ""
                                self.deltas.append(text)
                            elif event == "biz.report.done":
                                self.done_payload = payload
                                return
                        event = None
                        data_lines = []
                        continue
                    if line.startswith("event:"):
                        event = line.split(":", 1)[1].strip()
                    elif line.startswith("data:"):
                        data_lines.append(line.split(":", 1)[1].strip())
        except Exception as e:  # noqa: BLE001
            if not stop.is_set():
                self.error = str(e)


def task_status(task_id: str) -> dict:
    return http_json("GET", f"/v1/tasks/{task_id}")


def find_specialist(router_session: str, router_turn: str) -> dict | None:
    rows = pg_query(
        f"""
        SELECT turn_id, session_id, proj_id, status,
               solve_timing_jsonb->'activeDelegate'->>'turnId',
               solve_timing_jsonb->'activeDelegate'->>'sessionId',
               solve_timing_jsonb->'activeDelegate'->>'projId'
        FROM gateway_turns
        WHERE turn_id = '{router_turn}';
        """
    )
    if rows:
        r = rows[0]
        if len(r) >= 7 and r[4] and r[5]:
            return {
                "turn_id": r[4],
                "session_id": r[5],
                "proj_id": int(r[6] or OPS_PROJ),
                "source": "activeDelegate",
                "status": r[3],
            }
    link = pg_query(
        f"""
        SELECT delegate_session_id, delegate_proj_id
        FROM gateway_delegate_session_link
        WHERE root_session_id = '{router_session}' OR parent_session_id = '{router_session}'
        ORDER BY created_at_ms DESC NULLS LAST
        LIMIT 5;
        """
    )
    if not link:
        return None
    delegate_sid = link[0][0]
    delegate_proj = int(link[0][1] or OPS_PROJ)
    turns = pg_query(
        f"""
        SELECT turn_id, status, created_at_ms
        FROM gateway_turns
        WHERE session_id = '{delegate_sid}' AND proj_id = {delegate_proj}
        ORDER BY created_at_ms DESC
        LIMIT 3;
        """
    )
    if not turns:
        return {
            "turn_id": None,
            "session_id": delegate_sid,
            "proj_id": delegate_proj,
            "source": "delegate_link_only",
            "status": None,
        }
    return {
        "turn_id": turns[0][0],
        "session_id": delegate_sid,
        "proj_id": delegate_proj,
        "source": "delegate_session_latest_turn",
        "status": turns[0][1],
    }


def main() -> int:
    print(f"GW={GW} ROUTER_PROJ={ROUTER_PROJ}")
    health = http_json("GET", "/healthz", timeout=10)
    print(f"health cluster={health.get('clawTapCluster', {}).get('clusterId')}")

    body = {
        "projId": ROUTER_PROJ,
        "userPrompt": PROMPT,
        "extraSession": EXTRA,
    }
    t0 = time.time()
    async_resp = http_json("POST", "/v1/solve_async", body, timeout=60)
    session_id = async_resp["sessionId"]
    turn_id = async_resp["turnId"]
    task_id = async_resp.get("taskId") or session_id
    print(f"solve_async session={session_id} turn={turn_id} task={task_id}")

    stop = threading.Event()
    router_sse = SseCount("router")
    router_thread = threading.Thread(
        target=router_sse.run,
        args=(session_id, turn_id, ROUTER_PROJ, stop),
        daemon=True,
    )
    router_thread.start()

    specialist_sse = SseCount("specialist")
    specialist_thread: threading.Thread | None = None
    specialist_meta: dict | None = None
    specialist_started = False

    terminal = None
    while time.time() - t0 < 600:
        st = task_status(task_id)
        status = st.get("status")
        if not specialist_started:
            specialist_meta = find_specialist(session_id, turn_id)
            if specialist_meta and specialist_meta.get("turn_id"):
                specialist_started = True
                specialist_sse = SseCount("specialist")
                specialist_thread = threading.Thread(
                    target=specialist_sse.run,
                    args=(
                        specialist_meta["session_id"],
                        specialist_meta["turn_id"],
                        specialist_meta["proj_id"],
                        stop,
                    ),
                    daemon=True,
                )
                specialist_thread.start()
                print(
                    "specialist_sse_subscribe",
                    json.dumps(specialist_meta, ensure_ascii=False),
                )
        if status in ("succeeded", "failed", "cancelled"):
            terminal = status
            break
        time.sleep(0.5)

    # let SSE drain a bit after terminal
    time.sleep(2)
    stop.set()
    router_thread.join(timeout=5)
    if specialist_thread:
        specialist_thread.join(timeout=5)

    result = {
        "prompt": PROMPT,
        "router": {
            "sessionId": session_id,
            "turnId": turn_id,
            "projId": ROUTER_PROJ,
            "terminalStatus": terminal,
            "sseEvents": router_sse.events,
            "deltaCount": len(router_sse.deltas),
            "deltaTextLens": [len(x) for x in router_sse.deltas],
            "deltaPreview": [x[:80] for x in router_sse.deltas[:5]],
            "doneDeltaCount": (router_sse.done_payload or {}).get("deltaCount"),
            "error": router_sse.error,
        },
        "specialist": {
            "meta": specialist_meta,
            "sseEvents": specialist_sse.events,
            "deltaCount": len(specialist_sse.deltas),
            "deltaTextLens": [len(x) for x in specialist_sse.deltas],
            "deltaPreview": [x[:80] for x in specialist_sse.deltas[:5]],
            "doneDeltaCount": (specialist_sse.done_payload or {}).get("deltaCount"),
            "error": specialist_sse.error,
        },
        "elapsedSec": round(time.time() - t0, 1),
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    out_path = os.environ.get(
        "OUT", f"/tmp/router-sse-delta-{int(time.time())}.json"
    )
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(result, f, ensure_ascii=False, indent=2)
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
