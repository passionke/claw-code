#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Export prod conversation manifest (per-turn) for prompt regression QA.

Author: kejiqing

ONLY_READ against prod gateway. Pulls session list + turns, emits a flat manifest
with question time, extraSession params, store/org, session id, and turn index.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sys
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
DEFAULT_GATEWAY = os.environ.get("GATEWAY", "http://10.200.2.171:18088").rstrip("/")
DEFAULT_PROJ_ID = int(os.environ.get("PROJ_ID", "10"))
TZ = timezone(timedelta(hours=8))


def http_json(url: str, timeout: int = 60) -> Any:
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def bizdate_to_range_ms(bizdate: str) -> tuple[int, int]:
    d = datetime.strptime(bizdate, "%Y%m%d").replace(tzinfo=TZ)
    start = int(d.timestamp() * 1000)
    end = int((d + timedelta(days=1)).timestamp() * 1000) - 1
    return start, end


def ms_to_local(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=TZ).strftime("%Y-%m-%d %H:%M:%S")


def fetch_sessions(gateway: str, proj_id: int, bizdate: str) -> list[dict[str, Any]]:
    start, end = bizdate_to_range_ms(bizdate)
    sessions: list[dict[str, Any]] = []
    before_ms: int | None = None
    before_sid: str | None = None
    while True:
        params = f"limit=100&updatedFromMs={start}&updatedToMs={end}"
        if before_ms is not None and before_sid:
            params += f"&beforeUpdatedAtMs={before_ms}&beforeSessionId={before_sid}"
        url = f"{gateway}/v1/projects/{proj_id}/sessions?{params}"
        data = http_json(url)
        batch = data.get("sessions") or []
        sessions.extend(batch)
        if not data.get("hasMore") or not batch:
            break
        last = batch[-1]
        before_ms = int(last["updatedAtMs"])
        before_sid = str(last["sessionId"])
    return sessions


def fetch_turns(gateway: str, proj_id: int, session_id: str) -> list[dict[str, Any]]:
    url = f"{gateway}/v1/sessions/{session_id}/turns?proj_id={proj_id}"
    return http_json(url).get("turns") or []


def turn_to_item(
    *,
    bizdate: str,
    session_id: str,
    turn_count: int,
    turn: dict[str, Any],
    turn_index: int,
) -> dict[str, Any]:
    es = turn.get("extraSession") or {}
    if not isinstance(es, dict):
        es = {}
    return {
        "bizdate": bizdate,
        "questionAt": ms_to_local(int(turn["createdAtMs"])),
        "questionAtMs": int(turn["createdAtMs"]),
        "storeId": str(es.get("store_id") or ""),
        "storeName": str(es.get("store_name") or ""),
        "orgId": str(es.get("org_id") or ""),
        "extraSession": {k: str(v) for k, v in es.items() if isinstance(v, str)},
        "sessionId": session_id,
        "turnIndex": turn_index,
        "turnCount": turn_count,
        "turnId": str(turn.get("turnId") or ""),
        "userPrompt": str(turn.get("userPrompt") or ""),
        "status": str(turn.get("status") or ""),
        "prodReportPreview": str(turn.get("reportBody") or "")[:300],
    }


def export_manifest(
    gateway: str,
    proj_id: int,
    bizdates: list[str],
) -> dict[str, Any]:
    items: list[dict[str, Any]] = []
    stats: dict[str, Any] = {}
    for bizdate in bizdates:
        sessions = fetch_sessions(gateway, proj_id, bizdate)
        turn_count = 0
        for s in sessions:
            sid = str(s["sessionId"])
            turns = fetch_turns(gateway, proj_id, sid)
            for i, t in enumerate(turns, 1):
                items.append(
                    turn_to_item(
                        bizdate=bizdate,
                        session_id=sid,
                        turn_count=len(turns),
                        turn=t,
                        turn_index=i,
                    )
                )
                turn_count += 1
        stats[bizdate] = {"sessions": len(sessions), "turns": turn_count}
    items.sort(key=lambda x: (x["questionAtMs"], x["sessionId"], x["turnIndex"]))
    return {
        "exportedAtMs": int(datetime.now(tz=TZ).timestamp() * 1000),
        "source": gateway,
        "projId": proj_id,
        "bizdates": bizdates,
        "stats": stats,
        "items": items,
    }


def write_csv(path: Path, items: list[dict[str, Any]]) -> None:
    fields = [
        "bizdate",
        "questionAt",
        "storeId",
        "storeName",
        "orgId",
        "sessionId",
        "turnIndex",
        "turnCount",
        "turnId",
        "userPrompt",
        "status",
    ]
    with path.open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        w.writeheader()
        for row in items:
            w.writerow(row)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bizdates",
        default="20260614,20260615",
        help="comma-separated yyyyMMdd (Asia/Shanghai day boundary)",
    )
    parser.add_argument("--proj-id", type=int, default=DEFAULT_PROJ_ID)
    parser.add_argument("--gateway", default=DEFAULT_GATEWAY)
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="manifest json path (default: cases/proj{N}_manifest_<dates>.json)",
    )
    parser.add_argument("--csv", action="store_true", help="also write .csv alongside json")
    args = parser.parse_args()

    bizdates = [d.strip() for d in args.bizdates.split(",") if d.strip()]
    if not bizdates:
        print("no bizdates", file=sys.stderr)
        return 1

    gateway = args.gateway.rstrip("/")
    out = args.out
    if out is None:
        tag = "_".join(bizdates) if len(bizdates) <= 3 else f"{bizdates[0]}_{bizdates[-1]}"
        out = ROOT / "cases" / f"proj{args.proj_id}_manifest_{tag}.json"

    try:
        manifest = export_manifest(gateway, args.proj_id, bizdates)
    except urllib.error.URLError as e:
        print(f"gateway unreachable: {e}", file=sys.stderr)
        return 1
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code}: {e.read().decode()[:500]}", file=sys.stderr)
        return 1

    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)

    print(f"wrote {out}")
    for bd, st in manifest["stats"].items():
        print(f"  {bd}: {st['sessions']} sessions, {st['turns']} turns")
    print(f"  total turns: {len(manifest['items'])}")

    if args.csv:
        csv_path = out.with_suffix(".csv")
        write_csv(csv_path, manifest["items"])
        print(f"wrote {csv_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
