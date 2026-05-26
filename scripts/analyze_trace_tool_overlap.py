#!/usr/bin/env python3
# Analyze session trace NDJSON for overlapping SQLBot MCP tool windows. Author: kejiqing
"""Usage: analyze_trace_tool_overlap.py PATH_TO_TRACE.ndjson [--tool-substr sqlbot]"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def load_windows(path: Path, tool_substr: str) -> list[dict]:
    windows: list[dict] = []
    open_by_id: dict[str, dict] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        event = json.loads(line)
        name = event.get("name") or ""
        if name not in ("tool_execution_started", "tool_execution_finished"):
            continue
        attrs = event.get("attributes") or {}
        tool_name = attrs.get("tool_name") or ""
        if tool_substr and tool_substr not in tool_name:
            continue
        tool_use_id = (
            attrs.get("tool_use_id")
            or attrs.get("trace_tool_use_id")
            or attrs.get("toolUseId")
            or ""
        )
        ts = event.get("timestamp_ms")
        if ts is None:
            continue
        if name == "tool_execution_started":
            row = {
                "id": tool_use_id,
                "turn": attrs.get("turn_id"),
                "tool": tool_name,
                "start": ts,
                "end": None,
            }
            windows.append(row)
            if tool_use_id:
                open_by_id[tool_use_id] = row
        else:
            row = open_by_id.pop(tool_use_id, None)
            if row is None:
                for candidate in reversed(windows):
                    if candidate["end"] is None and candidate["tool"] == tool_name:
                        row = candidate
                        break
            if row is not None:
                row["end"] = ts
    return [w for w in windows if w["end"] is not None]


def count_overlaps(windows: list[dict]) -> tuple[int, list[tuple[dict, dict]]]:
    pairs: list[tuple[dict, dict]] = []
    for i, left in enumerate(windows):
        for right in windows[i + 1 :]:
            if left["start"] < right["end"] and right["start"] < left["end"]:
                pairs.append((left, right))
    return len(pairs), pairs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--tool-substr", default="sqlbot")
    args = parser.parse_args()
    if not args.trace.is_file():
        print(f"trace not found: {args.trace}", file=sys.stderr)
        return 1

    windows = load_windows(args.trace, args.tool_substr)
    overlaps, pairs = count_overlaps(windows)

    by_turn: dict[str, int] = {}
    for w in windows:
        turn = str(w.get("turn") or "?")
        by_turn[turn] = by_turn.get(turn, 0) + 1

    print(f"trace: {args.trace}")
    print(f"tools matching {args.tool_substr!r}: {len(windows)} completed windows")
    print(f"per-turn counts: {dict(sorted(by_turn.items()))}")
    print(f"overlapping pairs: {overlaps}")
    for left, right in pairs[:10]:
        print(
            f"  overlap: {left.get('turn')} {left['tool'][-40:]} "
            f"[{left['start']},{left['end']}] vs {right.get('turn')} "
            f"[{right['start']},{right['end']}]"
        )
    if overlaps > 0:
        print("VERDICT: parallel MCP execution detected (time windows overlap)")
        return 0
    print("VERDICT: no overlapping windows (serial or one tool per turn)")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
