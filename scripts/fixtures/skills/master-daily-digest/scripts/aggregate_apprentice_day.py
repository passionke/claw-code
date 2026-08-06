#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Aggregate digest inputs: Dod table, store 30d revisit, turn latency.

Author: kejiqing

Accepts either --in-dir from fetch_digest_inputs.py, or legacy --sessions-json.
"""
from __future__ import annotations

import argparse
import json
import math
import re
import statistics
from collections import Counter
from pathlib import Path
from typing import Any


def load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def items_of(pack: dict) -> list[dict]:
    return [x for x in (pack.get("items") or []) if isinstance(x, dict)]


def store_of_turn(t: dict) -> tuple[str | None, str]:
    es = t.get("extraSession") or t.get("extra_session") or {}
    if not isinstance(es, dict):
        return None, ""
    sid = es.get("store_id") or es.get("storeId")
    name = str(es.get("store_name") or es.get("storeName") or "")
    return (str(sid) if sid else None), name


def turn_duration_ms(t: dict) -> int | None:
    c = t.get("createdAtMs") or t.get("created_at_ms")
    f = t.get("finishedAtMs") or t.get("finished_at_ms")
    if c is None or f is None:
        return None
    try:
        c_i, f_i = int(c), int(f)
    except (TypeError, ValueError):
        return None
    if f_i < c_i:
        return None
    return f_i - c_i


def percentile(sorted_vals: list[float], p: float) -> float:
    if not sorted_vals:
        return float("nan")
    if len(sorted_vals) == 1:
        return float(sorted_vals[0])
    k = (len(sorted_vals) - 1) * p
    f = math.floor(k)
    c = math.ceil(k)
    if f == c:
        return float(sorted_vals[int(k)])
    return float(sorted_vals[f] * (c - k) + sorted_vals[c] * (k - f))


def window_stats(pack: dict) -> dict:
    items = items_of(pack)
    sessions = len(items)
    turns_all: list[dict] = []
    for it in items:
        turns_all.extend([t for t in (it.get("turns") or []) if isinstance(t, dict)])
    status = Counter(str(t.get("status") or "").lower() for t in turns_all)
    fb = Counter(str(t.get("feedback") or "").lower() for t in turns_all if t.get("feedback"))
    durs = [d for d in (turn_duration_ms(t) for t in turns_all) if d is not None]
    stores: dict[str, str] = {}
    for t in turns_all:
        sid, name = store_of_turn(t)
        if sid:
            stores[sid] = name or stores.get(sid, "")
    long_sessions = sum(1 for it in items if len(it.get("turns") or []) >= 3)
    return {
        "sessions": sessions,
        "turns": len(turns_all),
        "avg_turns_per_session": round(len(turns_all) / sessions, 2) if sessions else 0.0,
        "status": dict(status),
        "feedback": dict(fb),
        "ge3_sessions": long_sessions,
        "stores": stores,
        "durations_ms": durs,
    }


def fmt_ms(ms: float) -> str:
    if ms != ms:  # nan
        return "n/a"
    sec = ms / 1000.0
    if sec < 60:
        return f"{sec:.1f}s"
    return f"{sec/60:.1f}min ({sec:.0f}s)"


def duration_block(durs: list[int]) -> list[str]:
    lines = []
    if not durs:
        lines.append("- 无可用 finishedAtMs/createdAtMs")
        return lines
    s = sorted(float(x) for x in durs)
    lines.append(f"- 样本轮次: {len(s)}")
    lines.append(f"- 平均耗时: {fmt_ms(statistics.mean(s))}")
    lines.append(f"- 中位 P50: {fmt_ms(statistics.median(s))}")
    lines.append(f"- P90: {fmt_ms(percentile(s, 0.90))}")
    lines.append(f"- 最大: {fmt_ms(max(s))}")
    return lines


_TOKEN = re.compile(r"[\u4e00-\u9fff]{2,}|[A-Za-z][A-Za-z0-9_-]{2,}")


def top_terms(texts: list[str], n: int = 12) -> list[tuple[str, int]]:
    c: Counter[str] = Counter()
    stop = {"请问", "一下", "什么", "怎么", "如何", "这个", "那个", "我们", "今天", "昨天", "可以", "需要", "帮我"}
    for t in texts:
        for m in _TOKEN.findall(t or ""):
            if m not in stop:
                c[m] += 1
    return c.most_common(n)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bizdate", required=True)
    ap.add_argument("--apprentice-id", type=int, required=True)
    ap.add_argument("--in-dir", default="")
    ap.add_argument("--sessions-json", default="")
    ap.add_argument("--turns-json", default="")
    ap.add_argument("--out", default="-")
    args = ap.parse_args()

    lines: list[str] = []
    lines.append(f"# 聚合统计 · 学徒{args.apprentice_id} · {args.bizdate}")
    lines.append("")

    if args.in_dir:
        d = Path(args.in_dir)
        meta = load(d / "meta.json") if (d / "meta.json").exists() else {}
        day = load(d / "day.json")
        prev = load(d / "prev.json")
        look = load(d / "lookback30.json")
        st_d = window_stats(day)
        st_p = window_stats(prev)
        st_l = window_stats(look)

        prev_bd = meta.get("prevBizdate") or "前日"
        lines.append("## 昨日 vs 前日（客观量级）")
        lines.append("")
        lines.append("| 指标 | 前日 " + str(prev_bd) + " | 昨日 " + args.bizdate + " |")
        lines.append("|------|----|----|")
        lines.append(f"| 会话总数 | **{st_p['sessions']}** | **{st_d['sessions']}** |")
        lines.append(f"| 总轮次 | **{st_p['turns']}** | **{st_d['turns']}** |")
        lines.append(f"| 平均轮次/会话 | {st_p['avg_turns_per_session']} | {st_d['avg_turns_per_session']} |")
        lines.append(f"| status 分布 | `{st_p['status']}` | `{st_d['status']}` |")
        lines.append(f"| feedback | `{st_p['feedback']}` | `{st_d['feedback']}` |")
        lines.append(f"| ≥3 轮会话 | {st_p['ge3_sessions']} | {st_d['ge3_sessions']} |")
        lines.append(f"| 去重 store 数 | {len(st_p['stores'])} | {len(st_d['stores'])} |")
        lines.append("")

        stores_d = st_d["stores"]
        stores_l = set(st_l["stores"])
        revisit = sorted(set(stores_d) & stores_l)
        fresh = sorted(set(stores_d) - stores_l)
        lines.append("## 门店复访（store_id，相对近 30 天 lookback `[D-30,D)`）")
        lines.append("")
        lines.append(f"- 分析日来访去重 store: **{len(stores_d)}**")
        lines.append(f"- lookback 窗口会话数: {look.get('sessionCount')}（去重 store {len(stores_l)}）")
        lines.append(f"- 复访 store: **{len(revisit)}**")
        lines.append(f"- 新访 store: **{len(fresh)}**")
        if stores_d:
            lines.append(f"- 复访率: **{100.0 * len(revisit) / len(stores_d):.1f}%**")
        lines.append("")
        lines.append("### 复访名单")
        for sid in revisit[:40]:
            lines.append(f"- `{sid}` {stores_d.get(sid) or st_l['stores'].get(sid, '')}")
        if not revisit:
            lines.append("- （无）")
        lines.append("")
        lines.append("### 新访名单")
        for sid in fresh[:40]:
            lines.append(f"- `{sid}` {stores_d.get(sid, '')}")
        if not fresh:
            lines.append("- （无）")
        lines.append("")

        lines.append("## 耗时（每轮 finishedAtMs − createdAtMs）")
        lines.append("")
        lines.append("### 昨日")
        lines.extend(duration_block(st_d["durations_ms"]))
        lines.append("")
        lines.append("### 前日")
        lines.extend(duration_block(st_p["durations_ms"]))
        lines.append("")

        # preview terms from day
        previews = []
        for it in items_of(day):
            s = it.get("session") or {}
            previews.append(str(s.get("previewPrompt") or ""))
            for t in it.get("turns") or []:
                previews.append(str(t.get("userPrompt") or ""))
        lines.append("## 预览词频（粗）")
        for term, cnt in top_terms(previews):
            lines.append(f"- {term}: {cnt}")
        lines.append("")
        lines.append("## 说明")
        lines.append("- 需求满足率 / 拒答分类需模型按 turns 正文判定（v2：有数据或操作指引算满足）。")
        lines.append("- store 取自 turns.extraSession.store_id；缺字段的会话不计入复访分母分子。")
    else:
        # legacy single-file path
        lines.append("(legacy mode: provide --in-dir for Dod/revisit/latency)")
        if args.sessions_json:
            obj = load(Path(args.sessions_json))
            sess = obj.get("sessions") if isinstance(obj, dict) else obj
            lines.append(f"- sessions: {len(sess or [])}")

    text = "\n".join(lines) + "\n"
    if args.out == "-":
        print(text, end="")
    else:
        Path(args.out).write_text(text, encoding="utf-8")
        print(f"wrote {args.out}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
