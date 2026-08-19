#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Align “trunks” across:
  1) model gateway -> clawTap (raw HTTP chunk boundaries in CLAW_SSE_BURST_TRACE ndjson)
  2) clawTap -> browser (biz.report.delta events with clientDeltaMs/serverDeltaMs in browser dump)
Optionally:
  3) CLAW_SSE_DEBUG text log for frame payload preview within each chunk.

This script is intentionally boring (deterministic, order-based) so it produces evidence
instead of “hand waving”.

Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


@dataclass(frozen=True)
class NdJsonEvent:
    ev: str
    rawChunk: int | None
    deltaInChunk: int | None
    textLen: int | None
    monoMs: int | None
    wallMs: int | None
    bytes: int | None
    extra: dict[str, Any]


def _iter_ndjson_lines(path: Path) -> Iterable[dict[str, Any]]:
    with path.open("r", encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                # Keep going; logs might contain partial lines.
                continue


def parse_burst_ndjson(path: Path) -> tuple[
    dict[int, dict[str, Any]],  # http_chunk_by_rawChunk
    list[dict[str, Any]],  # ordered text_delta events
]:
    http_chunk_by_raw: dict[int, dict[str, Any]] = {}
    text_delta_events: list[dict[str, Any]] = []

    for obj in _iter_ndjson_lines(path):
        ev = str(obj.get("ev") or "")
        if not ev:
            continue
        mono_ms = obj.get("monoMs")
        wall_ms = obj.get("wallMs")

        if ev == "http_chunk":
            raw = obj.get("rawChunk")
            if not isinstance(raw, int):
                continue
            # Keep first (there should be only one, but be defensive)
            http_chunk_by_raw.setdefault(
                raw,
                {
                    "rawChunk": raw,
                    "bytes": obj.get("bytes"),
                    "monoMs": mono_ms,
                    "wallMs": wall_ms,
                },
            )
        elif ev == "text_delta":
            raw = obj.get("rawChunk")
            di = obj.get("deltaInChunk")
            tl = obj.get("textLen")
            if not isinstance(raw, int) or not isinstance(di, int):
                continue
            text_delta_events.append(
                {
                    "rawChunk": raw,
                    "deltaInChunk": di,
                    "textLen": tl if isinstance(tl, int) else None,
                    "monoMs": mono_ms if isinstance(mono_ms, int) else None,
                    "wallMs": wall_ms if isinstance(wall_ms, int) else None,
                }
            )
    return http_chunk_by_raw, text_delta_events


def parse_sse_debug_text(path: Path) -> dict[int, dict[str, Any]]:
    """
    Group CLAW_SSE_DEBUG lines by chunk_index.

    Returns:
      by_rawChunk[N] = {
        "rawChunk": N,
        "stream_chunk_received_elapsed_ms": ...,
        "frame_data_count": ...,
        "frame_payload_previews": [ ... ]  (may be truncated previews)
      }
    """
    chunk_started = False
    current_raw: int | None = None
    by_raw: dict[int, dict[str, Any]] = {}

    # Example line (from Rust):
    # [sse-debug] provider=xxx model=yyy stage=stream_chunk_received chunk_index=3 bytes=123 elapsed_ms=45 request_id=...
    received_re = re.compile(
        r"stage=stream_chunk_received\s+chunk_index=(?P<idx>\d+)\s+bytes=(?P<bytes>\d+)\s+elapsed_ms=(?P<elapsed>\d+)"
    )
    frame_re = re.compile(r"stage=stream_frame_data\s+payload=(?P<payload>.*)$")

    with path.open("r", encoding="utf-8", errors="replace") as f:
        for raw_line in f:
            line = raw_line.strip()
            if not line.startswith("[sse-debug]"):
                continue

            m = received_re.search(line)
            if m:
                current_raw = int(m.group("idx"))
                by_raw.setdefault(
                    current_raw,
                    {
                        "rawChunk": current_raw,
                        "stream_chunk_received_elapsed_ms": int(m.group("elapsed")),
                        "frame_data_count": 0,
                        "frame_payload_previews": [],
                    },
                )
                chunk_started = True
                continue

            if not chunk_started or current_raw is None:
                continue

            mf = frame_re.search(line)
            if mf:
                payload = mf.group("payload")
                entry = by_raw.setdefault(
                    current_raw,
                    {
                        "rawChunk": current_raw,
                        "stream_chunk_received_elapsed_ms": None,
                        "frame_data_count": 0,
                        "frame_payload_previews": [],
                    },
                )
                entry["frame_data_count"] = int(entry["frame_data_count"]) + 1
                entry["frame_payload_previews"].append(payload)
                continue

    return by_raw


def load_browser_delta_log(path: Path) -> list[dict[str, Any]]:
    data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
    if isinstance(data, list):
        return data
    # allow {"turnId": "...", "deltas": [...]}
    if isinstance(data, dict):
        for k in ("deltas", "deltaLog", "items", "records"):
            v = data.get(k)
            if isinstance(v, list):
                return v
    raise ValueError("Unsupported browser delta json format")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--burst-ndjson", required=True, help="CLAW_SSE_BURST_LOG_FILE (ndjson)")
    ap.add_argument("--browser-delta-json", required=True, help="Export of window.__bizReportDeltaLogByTurn[turnId]")
    ap.add_argument("--sse-debug-log", default="", help="CLAW_SSE_LOG_FILE (optional)")
    ap.add_argument("--out", default="", help="Output JSON path")
    args = ap.parse_args()

    burst_path = Path(args.burst_ndjson)
    browser_path = Path(args.browser_delta_json)
    debug_path = Path(args.sse_debug_log) if args.sse_debug_log else None

    http_by_raw, text_delta_events = parse_burst_ndjson(burst_path)
    browser_records = load_browser_delta_log(browser_path)
    def _seq_key(r: dict[str, Any]) -> int:
        v = r.get("seq")
        try:
            return int(v) if v is not None else 0
        except Exception:
            return 0

    browser_records_sorted = sorted(browser_records, key=_seq_key)

    # Order-based mapping: assume biz.report.delta.seq order matches text_delta event order.
    if len(browser_records_sorted) != len(text_delta_events):
        print(
            json.dumps(
                {
                    "warn": "length_mismatch",
                    "browserLen": len(browser_records_sorted),
                    "providerTextDeltaLen": len(text_delta_events),
                    "note": "Mapping will be done on the common prefix by index.",
                },
                ensure_ascii=False,
                indent=2,
            )
        )

    n = min(len(browser_records_sorted), len(text_delta_events))
    mapped = []
    len_mismatches = 0
    for i in range(n):
        b = browser_records_sorted[i]
        p = text_delta_events[i]
        b_text_len = b.get("textLen")
        p_text_len = p.get("textLen")
        if isinstance(b_text_len, int) and isinstance(p_text_len, int) and b_text_len != p_text_len:
            len_mismatches += 1
        mapped.append(
            {
                "globalDeltaIndex": i,
                "rawChunk": p["rawChunk"],
                "deltaInChunk": p["deltaInChunk"],
                "providerMonoMs": p.get("monoMs"),
                "browserSeq": b.get("seq"),
                "browserClientDeltaMs": b.get("clientDeltaMs"),
                "browserServerDeltaMs": b.get("serverDeltaMs"),
                "providerTextLen": p_text_len,
                "browserTextLen": b_text_len,
                "browserText": b.get("text"),  # exists after our UI instrumentation
            }
        )

    by_raw_mapped: dict[int, list[dict[str, Any]]] = {}
    for r in mapped:
        rc = r["rawChunk"]
        by_raw_mapped.setdefault(rc, []).append(r)

    debug_by_raw: dict[int, dict[str, Any]] = {}
    if debug_path and debug_path.exists():
        debug_by_raw = parse_sse_debug_text(debug_path)

    report = {
        "ok": True,
        "burstNdjson": str(burst_path),
        "browserDeltaJson": str(browser_path),
        "sseDebugLog": str(debug_path) if debug_path else "",
        "lengths": {
            "browserLen": len(browser_records_sorted),
            "providerTextDeltaLen": len(text_delta_events),
            "mappedPrefixLen": n,
        },
        "stats": {
            "mappedLenMismatchesByTextLen": len_mismatches,
            "rawChunkCountMapped": len(by_raw_mapped),
        },
        "trunks": [],
    }

    for rawChunk in sorted(by_raw_mapped.keys()):
        rows = by_raw_mapped[rawChunk]
        first = rows[0]
        last = rows[-1]
        http = http_by_raw.get(rawChunk, {})

        entry = {
            "rawChunk": rawChunk,
            "deltaCount": len(rows),
            "providerFirstMonoMs": http.get("monoMs") if isinstance(http.get("monoMs"), int) else first.get("providerMonoMs"),
            "providerFirstWallMs": http.get("wallMs") if isinstance(http.get("wallMs"), int) else None,
            "providerFirstBytes": http.get("bytes"),
            "browserFirstSeq": first.get("browserSeq"),
            "browserFirstClientDeltaMs": first.get("browserClientDeltaMs"),
            "browserFirstServerDeltaMs": first.get("browserServerDeltaMs"),
            "browserLastSeq": last.get("browserSeq"),
            "browserLastClientDeltaMs": last.get("browserClientDeltaMs"),
            "mappedDeltaIndexRange": {"start": rows[0]["globalDeltaIndex"], "end": rows[-1]["globalDeltaIndex"]},
            "serverElapsedMsFromSseDebug": None,
            "sseDebugFrameDataCount": None,
            "sseDebugFramePayloadPreviewsHead": [],
        }

        dbg = debug_by_raw.get(rawChunk)
        if dbg:
            entry["serverElapsedMsFromSseDebug"] = dbg.get("stream_chunk_received_elapsed_ms")
            entry["sseDebugFrameDataCount"] = dbg.get("frame_data_count")
            previews = dbg.get("frame_payload_previews") or []
            entry["sseDebugFramePayloadPreviewsHead"] = previews[:3]

        report["trunks"].append(entry)

    # Optional: include full mapped deltas for deeper inspection
    report["mappedDeltasPrefix"] = mapped[: min(2000, len(mapped))]

    out = json.dumps(report, ensure_ascii=False, indent=2)
    if args.out:
        Path(args.out).write_text(out, encoding="utf-8")
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

