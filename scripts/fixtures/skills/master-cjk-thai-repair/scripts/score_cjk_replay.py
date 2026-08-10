#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Score CJK mitigation vs control regression after observation_replay.

Author: kejiqing

--eval-set from build_cjk_eval_set.py
--replay-json: flexible shapes:
  { "results": [ { "itemId"|"sourceTurnId", "reportBody"|"message", "status" } ] }
  or { "turns": [...] } / list
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

CJK_RE = re.compile(r"[\u4E00-\u9FFF]")


def has_cjk(s: str) -> bool:
    return bool(CJK_RE.search(s or ""))


def load_replay_rows(path: Path) -> list[dict[str, Any]]:
    obj = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(obj, list):
        return [x for x in obj if isinstance(x, dict)]
    if not isinstance(obj, dict):
        return []
    for key in ("results", "items", "turns", "replayResults"):
        arr = obj.get(key)
        if isinstance(arr, list):
            return [x for x in arr if isinstance(x, dict)]
    # map itemId -> body
    if "byItemId" in obj and isinstance(obj["byItemId"], dict):
        rows = []
        for iid, v in obj["byItemId"].items():
            if isinstance(v, dict):
                rows.append({"itemId": iid, **v})
            else:
                rows.append({"itemId": iid, "reportBody": str(v)})
        return rows
    return []


def body_of(row: dict[str, Any]) -> str:
    for k in ("reportBody", "report_body", "message", "outputText", "body"):
        if row.get(k):
            return str(row[k])
    return ""


def status_of(row: dict[str, Any]) -> str:
    return str(row.get("status") or "").lower()


def index_replay(rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for r in rows:
        for key in ("itemId", "sourceTurnId", "turnId", "source_turn_id"):
            v = r.get(key)
            if v:
                out[str(v)] = r
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--eval-set", required=True)
    ap.add_argument("--replay-json", required=True)
    ap.add_argument("--out-dir", required=True)
    args = ap.parse_args()

    eval_set = json.loads(Path(args.eval_set).read_text(encoding="utf-8"))
    inv_items = (eval_set.get("inventoryJson") or {}).get("items") or []
    replay_idx = index_replay(load_replay_rows(Path(args.replay_json)))

    def match_row(item: dict[str, Any]) -> dict[str, Any] | None:
        for k in (item.get("itemId"), item.get("sourceTurnId")):
            if k and str(k) in replay_idx:
                return replay_idx[str(k)]
        return None

    cjk_items = [x for x in inv_items if x.get("bucket") == "cjk"]
    ctrl_items = [x for x in inv_items if x.get("bucket") == "control"]

    def bucket_stats(items: list[dict[str, Any]]) -> dict[str, Any]:
        base_cjk = 0
        after_cjk = 0
        missing = 0
        empty_after = 0
        failed_after = 0
        paired = 0
        for it in items:
            base = str(it.get("baselineReportBody") or "")
            if has_cjk(base):
                base_cjk += 1
            rep = match_row(it)
            if rep is None:
                missing += 1
                continue
            paired += 1
            body = body_of(rep)
            st = status_of(rep)
            if st in {"failed", "error", "cancelled"}:
                failed_after += 1
            if not body.strip():
                empty_after += 1
            if has_cjk(body):
                after_cjk += 1
        n = len(items)
        return {
            "n": n,
            "paired": paired,
            "missingReplay": missing,
            "baselineCjkHits": base_cjk,
            "afterCjkHits": after_cjk,
            "baselineCjkRate": round(base_cjk / n, 4) if n else None,
            "afterCjkRate": round(after_cjk / n, 4) if n else None,
            "emptyAfter": empty_after,
            "failedAfter": failed_after,
        }

    cjk_stats = bucket_stats(cjk_items)
    ctrl_stats = bucket_stats(ctrl_items)

    mitigated = False
    if cjk_stats["n"] > 0 and cjk_stats["paired"] > 0:
        # Prefer hit-count drop; also accept rate drop when paired==n
        mitigated = cjk_stats["afterCjkHits"] < cjk_stats["baselineCjkHits"]

    ctrl_ok = True
    if ctrl_stats["n"] > 0:
        # no large new CJK: after hits should not exceed baseline by >1
        # (control baseline usually 0 CJK)
        ctrl_base = sum(
            1
            for it in ctrl_items
            if has_cjk(str(it.get("baselineReportBody") or ""))
        )
        if ctrl_stats["afterCjkHits"] > ctrl_base + 1:
            ctrl_ok = False
        if ctrl_stats["emptyAfter"] > max(1, ctrl_stats["n"] // 5):
            ctrl_ok = False
        if ctrl_stats["failedAfter"] > max(1, ctrl_stats["n"] // 5):
            ctrl_ok = False
        if ctrl_stats["missingReplay"] > ctrl_stats["n"] // 2:
            ctrl_ok = False

    promote = bool(mitigated and ctrl_ok and cjk_stats["n"] > 0)

    score = {
        "promote_recommended": promote,
        "mitigated": mitigated,
        "controlOk": ctrl_ok,
        "cjk": cjk_stats,
        "control": ctrl_stats,
        "apprenticeId": eval_set.get("apprenticeId"),
        "bizdate": eval_set.get("bizdate"),
    }

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "score.json").write_text(
        json.dumps(score, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    lines = [
        f"# CJK score · 学徒{eval_set.get('apprenticeId')} · {eval_set.get('bizdate')}",
        "",
        f"- promote_recommended: **{promote}**",
        f"- mitigated: {mitigated}",
        f"- controlOk: {ctrl_ok}",
        "",
        "## CJK 病例",
        f"- n={cjk_stats['n']} paired={cjk_stats['paired']}",
        f"- baseline CJK hits: {cjk_stats['baselineCjkHits']} → after: {cjk_stats['afterCjkHits']}",
        f"- rate: {cjk_stats['baselineCjkRate']} → {cjk_stats['afterCjkRate']}",
        "",
        "## 对照",
        f"- n={ctrl_stats['n']} paired={ctrl_stats['paired']}",
        f"- after CJK hits: {ctrl_stats['afterCjkHits']} (empty={ctrl_stats['emptyAfter']} fail={ctrl_stats['failedAfter']})",
        "",
    ]
    (out_dir / "score.md").write_text("\n".join(lines), encoding="utf-8")
    print(json.dumps(score, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
