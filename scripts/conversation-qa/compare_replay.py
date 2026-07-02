#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Compare prod vs dev replay results (rule-based regression QA).

Author: kejiqing
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent

REFUSAL_CUES = (
    "ไม่สามารถ",
    "ไม่มีข้อมูล",
    "ขออภัย",
    "cannot",
    "unable",
    "抱歉",
    "无法",
    "ไม่สิ้นสุดรอบธุรกิจ",
    "ยังไม่มีข้อมูล",
    "wait until",
)
DATA_CUES = (
    "ยอดขาย",
    "sales",
    "revenue",
    "กำไร",
    "毛利",
    "profit",
    "margin",
    "%",
    "บาท",
    "thb",
    "฿",
    "table",
    "|",
    "รายได้",
)
TABLE_MARK = "|"


def detect_lang(text: str) -> str:
    if not text:
        return "other"
    th = len(re.findall(r"[\u0E00-\u0E7F]", text))
    zh = len(re.findall(r"[\u4e00-\u9fff]", text))
    en = len(re.findall(r"[a-zA-Z]", text))
    scores = {"th": th, "zh": zh, "en": en}
    best = max(scores, key=scores.get)
    return best if scores[best] > 0 else "other"


def lang_match(expected: str, report: str) -> bool:
    got = detect_lang(report)
    if expected == "other":
        return bool(report.strip())
    return got == expected or (expected == "en" and got in ("en", "other"))


def has_refusal(text: str) -> bool:
    low = text.lower()
    return any(cue in low or cue in text for cue in REFUSAL_CUES)


def has_data_signal(text: str) -> bool:
    low = text.lower()
    if re.search(r"\d[\d,.]*", text):
        return True
    return any(cue in low or cue in text for cue in DATA_CUES)


def token_set(text: str) -> set[str]:
    parts = re.findall(r"[\u0e00-\u0e7f]+|[\u4e00-\u9fff]+|[a-zA-Z]{3,}|\d+", text.lower())
    return set(parts)


def overlap_ratio(a: str, b: str) -> float:
    sa, sb = token_set(a), token_set(b)
    if not sa or not sb:
        return 0.0
    return len(sa & sb) / len(sa | sb)


def classify_row(row: dict[str, Any]) -> dict[str, Any]:
    prompt = str(row.get("userPrompt") or "")
    prod = str(row.get("prodReport") or row.get("prodReportPreview") or "")
    dev = str(row.get("devReport") or "")
    prod_ok = str(row.get("prodStatus") or "succeeded") == "succeeded"
    dev_ok = str(row.get("devStatus") or "") == "succeeded"
    lang = detect_lang(prompt)

    checks = {
        "dev_terminal_ok": dev_ok,
        "prod_terminal_ok": prod_ok,
        "dev_has_content": len(dev.strip()) >= 60,
        "prod_has_content": len(prod.strip()) >= 60,
        "dev_lang_match": lang_match(lang, dev) if dev_ok else False,
        "dev_has_data": has_data_signal(dev),
        "prod_has_data": has_data_signal(prod),
        "refusal_aligned": has_refusal(prod) == has_refusal(dev),
        "data_signal_aligned": has_data_signal(prod) == has_data_signal(dev),
    }
    ov = overlap_ratio(prod, dev)
    checks["token_overlap_ge_0.15"] = ov >= 0.15

    issues: list[str] = []
    if prod_ok and not dev_ok:
        issues.append("regression: dev failed while prod succeeded")
    if checks["prod_has_data"] and dev_ok and not checks["dev_has_data"]:
        issues.append("regression: prod had data signal, dev did not")
    if checks["prod_has_content"] and dev_ok and not checks["dev_has_content"]:
        issues.append("regression: dev answer too short")
    if prod_ok and dev_ok and checks["prod_has_data"] and has_refusal(dev) and not has_refusal(prod):
        issues.append("regression: dev refused but prod answered")
    if prod_ok and dev_ok and not has_refusal(prod) and has_refusal(dev):
        issues.append("regression: dev refused, prod did not")
    if dev_ok and not checks["dev_lang_match"]:
        issues.append("warning: dev language mismatch")

    verdict = "pass"
    if any(x.startswith("regression") for x in issues):
        verdict = "regression"
    elif issues:
        verdict = "warning"
    elif not dev_ok:
        verdict = "fail"

    passed = sum(1 for v in checks.values() if v)
    return {
        "verdict": verdict,
        "issues": issues,
        "checks": checks,
        "score": f"{passed}/{len(checks)}",
        "tokenOverlap": round(ov, 3),
        "promptLang": lang,
    }


def render_markdown(payload: dict[str, Any], evaluations: list[dict[str, Any]]) -> str:
    results = payload.get("results") or []
    verdicts = Counter(e["verdict"] for e in evaluations)
    lines = [
        "# Prompt regression compare (prod p10 vs dev p27)",
        "",
        f"- Sessions: {payload.get('sessionCount')}",
        f"- Turns: {payload.get('turnCount')}",
        f"- Pass: {verdicts.get('pass', 0)}",
        f"- Warning: {verdicts.get('warning', 0)}",
        f"- Regression: {verdicts.get('regression', 0)}",
        f"- Fail: {verdicts.get('fail', 0)}",
        "",
        "## Regressions / warnings",
        "",
    ]
    for row, ev in zip(results, evaluations):
        if ev["verdict"] in ("regression", "warning", "fail"):
            lines.append(
                f"### [{ev['verdict']}] {row.get('questionAt')} "
                f"{row.get('storeName')} turn {row.get('turnIndex')}/{row.get('turnCount')}"
            )
            lines.append(f"- **Q:** {row.get('userPrompt')}")
            lines.append(f"- **Issues:** {', '.join(ev['issues']) or ev['verdict']}")
            lines.append(f"- **Score:** {ev['score']} overlap={ev['tokenOverlap']}")
            prod = str(row.get("prodReport") or row.get("prodReportPreview") or "")[:400]
            dev = str(row.get("devReport") or "")[:400]
            lines.append(f"- **Prod:** {prod.replace(chr(10), ' ')}")
            lines.append(f"- **Dev:** {dev.replace(chr(10), ' ')}")
            lines.append("")
    if not any(e["verdict"] != "pass" for e in evaluations):
        lines.append("_No regressions flagged by heuristics._")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--replay",
        type=Path,
        default=ROOT / "cases" / "proj10_partial25_replay_p27.json",
    )
    parser.add_argument(
        "--out-json",
        type=Path,
        default=None,
        help="default: <replay-stem>_compare.json",
    )
    parser.add_argument(
        "--out-md",
        type=Path,
        default=None,
        help="default: <replay-stem>_compare.md",
    )
    args = parser.parse_args()

    if not args.replay.is_file():
        print(f"missing replay file: {args.replay}", file=sys.stderr)
        return 1

    with args.replay.open(encoding="utf-8") as f:
        payload = json.load(f)

    evaluations = [classify_row(r) for r in payload.get("results") or []]
    out_json = args.out_json or args.replay.with_name(f"{args.replay.stem}_compare.json")
    out_md = args.out_md or args.replay.with_name(f"{args.replay.stem}_compare.md")

    report = {
        "comparedAtMs": payload.get("recoveredAtMs") or payload.get("replayedAtMs"),
        "replay": str(args.replay),
        "summary": dict(Counter(e["verdict"] for e in evaluations)),
        "turnCount": len(evaluations),
        "items": [
            {**row, "evaluation": ev}
            for row, ev in zip(payload.get("results") or [], evaluations)
        ],
    }
    with out_json.open("w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=2)
    with out_md.open("w", encoding="utf-8") as f:
        f.write(render_markdown(payload, evaluations))

    s = report["summary"]
    print(f"wrote {out_json}")
    print(f"wrote {out_md}")
    print(
        f"turns={len(evaluations)} pass={s.get('pass',0)} "
        f"warning={s.get('warning',0)} regression={s.get('regression',0)} fail={s.get('fail',0)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
