#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Capture biz_advice_report SSE and analyze delta timing. Author: kejiqing"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.parse
import urllib.request
from collections import Counter
from typing import Any


def parse_sse_blocks(raw: bytes) -> list[tuple[str | None, str | None]]:
    text = raw.decode("utf-8", errors="replace")
    events: list[tuple[str | None, str | None]] = []
    buf: list[str] = []
    for line in text.splitlines():
        if line.startswith("event:"):
            if buf:
                events.append(_flush_block(buf))
                buf = []
            buf.append(line)
        elif line.startswith("data:"):
            buf.append(line)
        elif line == "":
            if buf:
                events.append(_flush_block(buf))
                buf = []
    if buf:
        events.append(_flush_block(buf))
    return events


def _flush_block(buf: list[str]) -> tuple[str | None, str | None]:
    ev_name = None
    data_lines: list[str] = []
    for ln in buf:
        if ln.startswith("event:"):
            ev_name = ln[len("event:") :].strip()
        elif ln.startswith("data:"):
            data_lines.append(ln[len("data:") :].lstrip())
    return ev_name, "\n".join(data_lines) if data_lines else None


def analyze_deltas(records: list[dict[str, Any]]) -> dict[str, Any]:
    server_ms = Counter()
    text_lens: list[int] = []
    large = 0
    for r in records:
        sdm = r.get("serverDeltaMs")
        if sdm is not None:
            server_ms[int(sdm)] += 1
        tl = r.get("textLen")
        if tl is None and "text" in r:
            tl = len(str(r.get("text") or ""))
        if tl is not None:
            text_lens.append(int(tl))
            if int(tl) >= 200:
                large += 1
    max_same_server = max(server_ms.values()) if server_ms else 0
    hot = sorted([(k, v) for k, v in server_ms.items() if v >= 5], key=lambda x: -x[1])[:10]
    return {
        "delta_count": len(records),
        "max_same_server_delta_ms": max_same_server,
        "server_ms_buckets_ge5": hot,
        "large_delta_ge200": large,
        "text_len_max": max(text_lens) if text_lens else 0,
        "text_len_median": sorted(text_lens)[len(text_lens) // 2] if text_lens else 0,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gateway", default="http://127.0.0.1:18088")
    ap.add_argument("--session-id", required=True)
    ap.add_argument("--turn-id", required=True)
    ap.add_argument("--ds-id", type=int, default=1)
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    q = urllib.parse.urlencode(
        {
            "sessionId": args.session_id,
            "turnId": args.turn_id,
            "dsId": str(args.ds_id),
            "stream": "true",
        }
    )
    url = f"{args.gateway.rstrip('/')}/v1/biz_advice_report?{q}"
    req = urllib.request.Request(url, method="GET")
    t0 = time.monotonic()
    deltas: list[dict[str, Any]] = []
    try:
        with urllib.request.urlopen(req, timeout=args.timeout) as resp:
            body = resp.read()
    except Exception as e:
        print(json.dumps({"ok": False, "error": str(e)}, ensure_ascii=False), file=sys.stderr)
        return 1

    elapsed = time.monotonic() - t0
    events = parse_sse_blocks(body)
    for ev_name, data in events:
        if ev_name != "biz.report.delta" or not data:
            continue
        try:
            obj = json.loads(data)
        except json.JSONDecodeError:
            continue
        obj["_clientRecvMs"] = int((time.monotonic() - t0) * 1000)
        deltas.append(obj)

    report = {
        "ok": True,
        "url": url,
        "elapsed_sec": round(elapsed, 2),
        "analysis": analyze_deltas(deltas),
        "deltas_head": deltas[:5],
        "deltas_tail": deltas[-3:] if len(deltas) >= 3 else deltas,
    }
    out = json.dumps(report, ensure_ascii=False, indent=2)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(out)
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
