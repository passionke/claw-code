#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Fetch apprentice day/prev/lookback30 sessions+turns from gateway HTTP.

Author: kejiqing

For master-daily-digest metrics (store_id revisit, turn latency, Dod).
Uses public list/turns APIs (same as Admin). Prefer GATEWAY_BASE / CLAW_GATEWAY_BASE.
"""
from __future__ import annotations

import argparse
import json
import os
import urllib.parse
import urllib.request
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo


def http_get(url: str, timeout: int = 60) -> Any:
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def parse_bizdate(s: str) -> date:
    s = s.strip().replace("-", "")
    return date(int(s[0:4]), int(s[4:6]), int(s[6:8]))


def window_ms(d: date, tz: ZoneInfo) -> tuple[int, int]:
    start = datetime(d.year, d.month, d.day, tzinfo=tz)
    end = start + timedelta(days=1)
    return int(start.timestamp() * 1000), int(end.timestamp() * 1000)


def list_sessions(base: str, proj: int, from_ms: int, to_ms: int, limit: int = 100) -> list[dict]:
    out: list[dict] = []
    before_ms = None
    before_sid = None
    while True:
        params: dict[str, Any] = {
            "limit": limit,
            "updatedFromMs": from_ms,
            "updatedToMs": to_ms,
        }
        if before_ms is not None:
            params["beforeUpdatedAtMs"] = before_ms
            params["beforeSessionId"] = before_sid
        q = urllib.parse.urlencode(params)
        data = http_get(f"{base.rstrip('/')}/v1/projects/{proj}/sessions?{q}")
        batch = data.get("sessions") or []
        out.extend(batch)
        if not data.get("hasMore") or not batch:
            break
        last = batch[-1]
        before_ms = last["updatedAtMs"]
        before_sid = last["sessionId"]
        if len(out) >= 5000:
            break
    return out


def list_turns(base: str, proj: int, session_id: str) -> list[dict]:
    data = http_get(
        f"{base.rstrip('/')}/v1/sessions/{urllib.parse.quote(session_id)}/turns?projId={proj}"
    )
    return data.get("turns") or []


def pack_window(base: str, proj: int, from_ms: int, to_ms: int) -> dict:
    sessions = list_sessions(base, proj, from_ms, to_ms)
    items = []
    for s in sessions:
        sid = s["sessionId"]
        turns = list_turns(base, proj, sid)
        items.append({"session": s, "turns": turns})
    return {
        "projId": proj,
        "updatedFromMs": from_ms,
        "updatedToMs": to_ms,
        "sessionCount": len(sessions),
        "items": items,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gateway", default=os.environ.get("CLAW_GATEWAY_BASE") or os.environ.get("GATEWAY_BASE") or "")
    ap.add_argument("--apprentice-id", type=int, required=True)
    ap.add_argument("--bizdate", required=True, help="YYYYMMDD (= D, 分析日)")
    ap.add_argument("--tz", default="Asia/Bangkok")
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--lookback-days", type=int, default=30)
    args = ap.parse_args()
    if not args.gateway:
        raise SystemExit("need --gateway or CLAW_GATEWAY_BASE")

    tz = ZoneInfo(args.tz)
    d = parse_bizdate(args.bizdate)
    p = d - timedelta(days=1)
    l0 = d - timedelta(days=args.lookback_days)
    a_d, b_d = window_ms(d, tz)
    a_p, b_p = window_ms(p, tz)
    a_l, _ = window_ms(l0, tz)

    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    meta = {
        "apprenticeId": args.apprentice_id,
        "bizdate": args.bizdate,
        "prevBizdate": p.strftime("%Y%m%d"),
        "tz": args.tz,
        "lookbackDays": args.lookback_days,
        "windows": {
            "day": [a_d, b_d],
            "prev": [a_p, b_p],
            "lookback30": [a_l, a_d],
        },
        "gateway": args.gateway,
    }
    (out / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")

    print("fetch day...", flush=True)
    day = pack_window(args.gateway, args.apprentice_id, a_d, b_d)
    (out / "day.json").write_text(json.dumps(day, ensure_ascii=False), encoding="utf-8")
    print(f"day sessions={day['sessionCount']}", flush=True)

    print("fetch prev...", flush=True)
    prev = pack_window(args.gateway, args.apprentice_id, a_p, b_p)
    (out / "prev.json").write_text(json.dumps(prev, ensure_ascii=False), encoding="utf-8")
    print(f"prev sessions={prev['sessionCount']}", flush=True)

    print("fetch lookback30...", flush=True)
    look = pack_window(args.gateway, args.apprentice_id, a_l, a_d)
    (out / "lookback30.json").write_text(json.dumps(look, ensure_ascii=False), encoding="utf-8")
    print(f"lookback30 sessions={look['sessionCount']}", flush=True)
    print(f"wrote {out}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
