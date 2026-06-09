#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Measure chunk / render temporal density for biz_advice_report SSE.

Layers:
  L1 serverDeltaMs (gateway emit time, ms bucket)
  L2 recvMonoMs   (client read time while streaming this script)

Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import math
import sys
import time
import urllib.parse
import urllib.request
from collections import Counter
from typing import Any


def parse_sse_stream(resp: Any, t0: float) -> list[dict[str, Any]]:
    """Parse SSE incrementally; record recvMonoMs at each complete event."""
    events: list[dict[str, Any]] = []
    buf: list[str] = []
    while True:
        raw = resp.readline()
        if not raw:
            break
        line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
        if line == "":
            if not buf:
                continue
            ev_name, data = _flush_block(buf)
            buf = []
            if ev_name == "biz.report.delta" and data:
                try:
                    obj = json.loads(data)
                except json.JSONDecodeError:
                    continue
                obj["recvMonoMs"] = int((time.monotonic() - t0) * 1000)
                events.append(obj)
            continue
        buf.append(line)
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


def _times(events: list[dict[str, Any]], key: str) -> list[int]:
    out: list[int] = []
    for e in events:
        v = e.get(key)
        if v is not None:
            out.append(int(v))
    return out


def _text_lens(events: list[dict[str, Any]]) -> list[int]:
    out: list[int] = []
    for e in events:
        tl = e.get("textLen")
        if tl is None:
            tl = len(str(e.get("text") or ""))
        out.append(int(tl))
    return out


def _bucket_counts(times_ms: list[int], bucket_ms: int) -> Counter[int]:
    w = max(1, bucket_ms)
    c: Counter[int] = Counter()
    for t in times_ms:
        c[t // w] += 1
    return c


def _iat_stats(times_ms: list[int]) -> dict[str, float | int]:
    if len(times_ms) < 2:
        return {"count": max(0, len(times_ms) - 1)}
    iats = [times_ms[i] - times_ms[i - 1] for i in range(1, len(times_ms))]
    iats_sorted = sorted(iats)
    mean = sum(iats) / len(iats)
    var = sum((x - mean) ** 2 for x in iats) / len(iats)
    std = math.sqrt(var)
    return {
        "count": len(iats),
        "min_ms": iats_sorted[0],
        "median_ms": iats_sorted[len(iats_sorted) // 2],
        "p95_ms": iats_sorted[int(len(iats_sorted) * 0.95)],
        "max_ms": iats_sorted[-1],
        "mean_ms": round(mean, 3),
        "std_ms": round(std, 3),
        "cv": round(std / mean, 4) if mean > 0 else 0.0,
        "zero_ms_count": sum(1 for x in iats if x == 0),
    }


def _simultaneity_ratio(times_ms: list[int]) -> float:
    if len(times_ms) < 2:
        return 0.0
    same = sum(1 for i in range(1, len(times_ms)) if times_ms[i] == times_ms[i - 1])
    return round(same / (len(times_ms) - 1), 4)


def _burstiness(bucket: Counter[int], bucket_ms: int) -> float:
    if not bucket:
        return 0.0
    rates = [c / bucket_ms for c in bucket.values()]
    mx = max(rates)
    mean = sum(rates) / len(rates)
    return round(mx / mean, 4) if mean > 0 else 0.0


def _layer_density(events: list[dict[str, Any]], time_key: str, bucket_list: list[int]) -> dict[str, Any]:
    times = _times(events, time_key)
    if not times:
        return {"event_count": 0, "time_key": time_key}
    lens = _text_lens(events)
    span = times[-1] - times[0]
    chars = sum(lens)
    out: dict[str, Any] = {
        "time_key": time_key,
        "event_count": len(times),
        "span_ms": span,
        "chars_total": chars,
        "chars_per_sec": round(chars / (span / 1000.0), 2) if span > 0 else 0.0,
        "text_len_median": sorted(lens)[len(lens) // 2],
        "text_len_max": max(lens),
        "large_delta_ge200": sum(1 for x in lens if x >= 200),
        "simultaneity_ratio": _simultaneity_ratio(times),
        "iat": _iat_stats(times),
    }
    for w in bucket_list:
        bc = _bucket_counts(times, w)
        mx = max(bc.values()) if bc else 0
        hot = sorted(bc.items(), key=lambda x: -x[1])[:5]
        out[f"max_bucket_count_{w}ms"] = mx
        out[f"hot_buckets_{w}ms"] = [{"bucket": b, "count": c, "t_ms_lo": b * w} for b, c in hot]
        out[f"burstiness_{w}ms"] = _burstiness(bc, w)
    return out


def analyze_events(events: list[dict[str, Any]], bucket_ms: list[int]) -> dict[str, Any]:
    return {
        "delta_count": len(events),
        "L1_gateway_emit": _layer_density(events, "serverDeltaMs", bucket_ms),
        "L2_stream_reader_recv": _layer_density(events, "recvMonoMs", bucket_ms),
        "head": events[:3],
        "tail": events[-3:] if len(events) >= 3 else events,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="Measure SSE chunk temporal density")
    ap.add_argument("--gateway", default="http://127.0.0.1:18088")
    ap.add_argument("--session-id", required=True)
    ap.add_argument("--turn-id", required=True)
    ap.add_argument("--proj-id", type=int, default=1, dest="proj_id")
    ap.add_argument("--ds-id", type=int, default=None, dest="ds_id", help="legacy alias for --proj-id")
    ap.add_argument("--timeout", type=float, default=300.0)
    ap.add_argument("--bucket-ms", default="1,16", help="Comma-separated bucket widths")
    ap.add_argument("--out", default="")
    args = ap.parse_args()
    if args.ds_id is not None:
        args.proj_id = args.ds_id

    buckets = [int(x.strip()) for x in args.bucket_ms.split(",") if x.strip().isdigit()]
    if not buckets:
        buckets = [1, 16]

    q = urllib.parse.urlencode(
        {
            "sessionId": args.session_id,
            "turnId": args.turn_id,
            "projId": str(args.proj_id),
            "stream": "true",
        }
    )
    url = f"{args.gateway.rstrip('/')}/v1/biz_advice_report?{q}"
    req = urllib.request.Request(url, method="GET")
    t0 = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=args.timeout) as resp:
            events = parse_sse_stream(resp, t0)
    except Exception as e:
        print(json.dumps({"ok": False, "error": str(e), "url": url}, ensure_ascii=False), file=sys.stderr)
        return 1

    report = {
        "ok": True,
        "url": url,
        "wall_sec": round(time.monotonic() - t0, 3),
        "metrics": analyze_events(events, buckets),
    }
    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text)
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
