#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build CJK-pollution cases + control sample from apprentice day.json.

Author: kejiqing

CJK case: user prompt has Thai chars AND assistant reportBody has CJK.
Control: non-case succeeded turns with body; sample max(5, ceil(10% eligible)).
"""
from __future__ import annotations

import argparse
import json
import math
import random
import re
from pathlib import Path
from typing import Any

THAI_RE = re.compile(r"[\u0E00-\u0E7F]")
CJK_RE = re.compile(r"[\u4E00-\u9FFF]")


def has_thai(s: str) -> bool:
    return bool(THAI_RE.search(s or ""))


def has_cjk(s: str) -> bool:
    return bool(CJK_RE.search(s or ""))


def load_day(path: Path) -> list[dict[str, Any]]:
    obj = json.loads(path.read_text(encoding="utf-8"))
    items = obj.get("items") if isinstance(obj, dict) else None
    if isinstance(items, list):
        return [x for x in items if isinstance(x, dict)]
    # flat sessions+turns alternate shapes
    return []


def iter_turns(items: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for it in items:
        sess = it.get("session") or {}
        sid = str(sess.get("sessionId") or sess.get("session_id") or "")
        for t in it.get("turns") or []:
            if not isinstance(t, dict):
                continue
            tid = str(t.get("turnId") or t.get("turn_id") or "")
            prompt = str(t.get("userPrompt") or t.get("user_prompt") or "")
            body = str(t.get("reportBody") or t.get("report_body") or "")
            status = str(t.get("status") or "").lower()
            rows.append(
                {
                    "sessionId": sid,
                    "turnId": tid,
                    "userPrompt": prompt,
                    "reportBody": body,
                    "status": status,
                }
            )
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bizdate", required=True)
    ap.add_argument("--apprentice-id", type=int, required=True)
    ap.add_argument("--day-json", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    items = load_day(Path(args.day_json))
    turns = iter_turns(items)

    cjk_cases: list[dict[str, Any]] = []
    eligible_control: list[dict[str, Any]] = []
    for row in turns:
        if not row["sessionId"] or not row["turnId"]:
            continue
        thai = has_thai(row["userPrompt"])
        cjk = has_cjk(row["reportBody"])
        if thai and cjk and row["reportBody"].strip():
            cjk_cases.append({**row, "bucket": "cjk"})
            continue
        ok_status = row["status"] in {"", "succeeded", "success", "ok", "completed"}
        if ok_status and row["reportBody"].strip() and not (thai and cjk):
            eligible_control.append({**row, "bucket": "control"})

    n_elig = len(eligible_control)
    n_ctrl = max(5, int(math.ceil(n_elig * 0.10))) if n_elig else 0
    n_ctrl = min(n_ctrl, n_elig)
    rng = random.Random(args.seed)
    control = rng.sample(eligible_control, n_ctrl) if n_ctrl else []

    inventory_items = []
    for i, row in enumerate(cjk_cases + control):
        inventory_items.append(
            {
                "itemId": f"{row['bucket']}_{i}_{row['turnId'][:12]}",
                "sourceSessionId": row["sessionId"],
                "sourceTurnId": row["turnId"],
                "bizdate": args.bizdate,
                "replay": True,
                "bucket": row["bucket"],
                "userPrompt": row["userPrompt"][:500],
                "baselineReportBody": row["reportBody"],
            }
        )

    out = {
        "apprenticeId": args.apprentice_id,
        "bizdate": args.bizdate,
        "cjkCount": len(cjk_cases),
        "controlCount": len(control),
        "eligibleControlPool": n_elig,
        "cases": cjk_cases,
        "control": control,
        "inventoryJson": {"items": inventory_items},
    }
    Path(args.out).write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print(
        json.dumps(
            {
                "wrote": args.out,
                "cjkCount": len(cjk_cases),
                "controlCount": len(control),
                "eligibleControlPool": n_elig,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
