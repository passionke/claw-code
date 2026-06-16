#!/usr/bin/env python3
# Score GPOS lang eval cases. Author: kejiqing
"""Score case JSON files and emit summary.json + summary.txt."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent

STORE_NAME = "ทองหล่อมินิมาร์ท"

# Agent planning / meta talk at the *opening* of the user-visible report (not markdown shape).
MONOLOGUE_PATTERNS: list[tuple[str, str]] = [
    (r"Now let me\b", "now_let_me"),
    (r"^Let me (load|check|query|run|start|first|look|fetch|analyze)\b", "let_me_plan"),
    (r"^I'll (start|check|query|run|first|look|need to|analyze)\b", "ill_plan"),
    (r"^I will (start|check|query|run|first|analyze)\b", "i_will_plan"),
    (r"^I need to (check|query|load|run|first|analyze)\b", "i_need_to"),
    (r"^用户问", "user_ask_meta"),
    (r"^让我(先|来|查|看|加载)", "rang_wo_plan"),
    (r"^我先", "wo_xian_plan"),
    (r"^接下来(我|让)", "jie_xia_lai_plan"),
    (r"^现在让我", "xian_zai_rang_wo"),
    (r"store_id", "store_id_leak"),
    (r"S20241007172800004204", "store_id_value"),
]

LEAD_CHARS = 600

LANG_MAP = {"zh": "Chinese", "en": "English", "th": "Thai"}

# Thai output must not leak Chinese template phrases or CJK paragraphs. Author: kejiqing
THAI_FORBIDDEN_CJK_PHRASES: tuple[str, ...] = (
    "数据显示",
    "建议考虑",
    "依据何在",
    "值得注意的是",
    "做什么",
    "怎么做",
    "门店",
    "客流",
    "套餐",
    "预期",
    "依据",
)
THAI_CJK_PARAGRAPH_MIN_RUN = 8
THAI_CJK_TOTAL_FAIL_THRESHOLD = 12

# Gateway terminal / queue labels (task_status.rs) — not user-facing progress stream.
_SYSTEM_PROGRESS_RE = re.compile(
    r"^(?:分析完成(?:（\d+/\d+）)?|任务失败|任务已取消|排队中（.+）|分析门店)$"
)


def count_scripts(text: str) -> dict[str, int]:
    cjk = thai = latin = 0
    for ch in text:
        if "\u4e00" <= ch <= "\u9fff":
            cjk += 1
        elif "\u0e00" <= ch <= "\u0e7f":
            thai += 1
        elif ch.isascii() and ch.isalpha():
            latin += 1
    return {"cjk": cjk, "thai": thai, "latin": latin}


def dominant_lang(counts: dict[str, int]) -> str:
    cjk, thai, latin = counts["cjk"], counts["thai"], counts["latin"]
    total = cjk + thai + latin
    if total == 0:
        return "empty"
    if cjk >= thai and cjk >= latin and cjk > 0:
        return "Chinese"
    if thai >= cjk and thai >= latin and thai > 0:
        return "Thai"
    if latin >= cjk and latin >= thai and latin > 0:
        return "English"
    return "mixed"


def strip_store_name(text: str, store_name: str) -> str:
    return text.replace(store_name, "")


def detect_lang(text: str, store_name: str = STORE_NAME) -> str:
    cleaned = strip_store_name(text or "", store_name)
    return dominant_lang(count_scripts(cleaned))


def progress_text_for_lang(case: dict) -> str:
    """User-visible progress lines (playground stream), not terminal currentTaskDesc."""
    hist = case.get("progressHistory") or []
    if hist:
        parts = [str(e.get("message") or "").strip() for e in hist if e.get("message")]
        return "\n".join(p for p in parts if p)
    raw = (case.get("progressDesc") or case.get("currentTaskDesc") or "").strip()
    if not raw or _SYSTEM_PROGRESS_RE.match(raw):
        return ""
    return raw


def mcp_question_messages(case: dict) -> list[str]:
    """Lines from mcp_tool_started events (echo MCP `question` or gateway fallback)."""
    hist = case.get("progressHistory") or []
    return [
        str(e.get("message") or "").strip()
        for e in hist
        if e.get("kind") == "mcp_tool_started" and (e.get("message") or "").strip()
    ]


def check_mcp_question_lang(case: dict, store_name: str = STORE_NAME) -> tuple[bool, str, str]:
    """Dominant script of MCP question lines should match turn language."""
    expected = LANG_MAP[case["lang"]]
    msgs = mcp_question_messages(case)
    if not msgs:
        return True, "n/a", "empty"
    combined = "\n".join(msgs)
    got = detect_lang(combined, store_name)
    ok = got == expected or (expected == "English" and got in ("English", "mixed"))
    note = "ok" if ok else f"{got}!={expected}"
    return ok, note, got


def message_lead(message: str) -> str:
    """First paragraph / opening slice — monologue usually appears here."""
    msg = (message or "").lstrip()
    if not msg:
        return ""
    para = msg.split("\n\n", 1)[0]
    return para[:LEAD_CHARS]


def check_monologue(message: str) -> tuple[bool, list[str]]:
    """Detect planning / self-talk in the report opening, not markdown layout."""
    issues: list[str] = []
    lead = message_lead(message)
    if not lead:
        return True, issues
    for pattern, tag in MONOLOGUE_PATTERNS:
        if re.search(pattern, lead, re.IGNORECASE | re.MULTILINE):
            issues.append(f"monologue:{tag}")
    return len(issues) == 0, issues


def cjk_runs(text: str) -> list[str]:
    return re.findall(r"[\u4e00-\u9fff]+", text or "")


def check_thai_cjk_leak(message: str, store_name: str = STORE_NAME) -> tuple[bool, list[str]]:
    """Thai turns: forbid Chinese template phrases and CJK paragraphs in final report."""
    cleaned = strip_store_name(message or "", store_name)
    issues: list[str] = []
    runs = cjk_runs(cleaned)
    if not runs:
        return True, issues
    total = sum(len(r) for r in runs)
    for phrase in THAI_FORBIDDEN_CJK_PHRASES:
        if phrase in cleaned:
            issues.append(f"th_cjk_phrase:{phrase}")
    for run in runs:
        if len(run) >= THAI_CJK_PARAGRAPH_MIN_RUN:
            issues.append(f"th_cjk_run:{run[:24]}{'…' if len(run) > 24 else ''}")
    if total >= THAI_CJK_TOTAL_FAIL_THRESHOLD:
        issues.append(f"th_cjk_total:{total}")
    return len(issues) == 0, issues


def check_invalid_handled(case: dict, message: str) -> tuple[bool, str]:
    """Q30 off-topic control — informational only; capability intro is acceptable. Author: kejiqing"""
    if case.get("questionId") != "Q30":
        return True, "n/a"
    msg = (message or "").strip()
    if not msg:
        return False, "empty_response"
    return True, "exempt_off_topic"


def score_case(case: dict, store_name: str = STORE_NAME) -> dict[str, Any]:
    expected = LANG_MAP[case["lang"]]
    message = case.get("message") or ""
    progress = progress_text_for_lang(case)

    msg_lang = detect_lang(message, store_name)
    prog_lang = detect_lang(progress, store_name) if progress.strip() else "empty"

    lang_msg_ok = msg_lang == expected or (expected == "English" and msg_lang in ("English", "mixed"))
    lang_prog_ok = (
        prog_lang in ("empty", expected)
        or (expected == "English" and prog_lang in ("English", "mixed"))
    )

    store_in_msg = store_name in message
    store_in_prog = store_name in progress
    store_name_kept = store_in_msg or store_in_prog

    monologue_clean, monologue_issues = check_monologue(message)
    invalid_ok, invalid_note = check_invalid_handled(case, message)
    mcp_q_ok, mcp_q_note, mcp_q_lang = check_mcp_question_lang(case, store_name)
    thai_cjk_clean, thai_cjk_issues = (
        check_thai_cjk_leak(message, store_name)
        if case.get("lang") == "th"
        else (True, [])
    )

    # Q30 skips normal lang/delivery gates for analytical report
    is_invalid_control = case.get("questionId") == "Q30"

    if is_invalid_control:
        # Off-topic (Q30): succeeded + non-empty reply is enough; no short-refusal gate.
        overall = case.get("status") == "succeeded" and invalid_ok
        fail_reasons = []
        if case.get("status") != "succeeded":
            fail_reasons.append(f"status:{case.get('status')}")
        if not invalid_ok:
            fail_reasons.append(f"invalid:{invalid_note}")
    else:
        fail_reasons = []
        if case.get("status") != "succeeded":
            fail_reasons.append(f"status:{case.get('status')}")
        if not lang_msg_ok:
            fail_reasons.append(f"msg_lang:{msg_lang}!={expected}")
        if not lang_prog_ok and progress.strip():
            fail_reasons.append(f"prog_lang:{prog_lang}!={expected}")
        if not store_name_kept and case.get("status") == "succeeded":
            fail_reasons.append("store_name_missing")
        if not monologue_clean:
            fail_reasons.extend(monologue_issues)
        if not mcp_q_ok and mcp_q_note != "n/a":
            fail_reasons.append(f"mcp_question_lang:{mcp_q_note}")
        if case.get("lang") == "th" and not thai_cjk_clean:
            fail_reasons.extend(thai_cjk_issues)
        overall = len(fail_reasons) == 0

    counts_msg = count_scripts(strip_store_name(message, store_name))
    counts_prog = count_scripts(strip_store_name(progress, store_name))

    return {
        "caseId": case.get("caseId"),
        "questionId": case.get("questionId"),
        "category": case.get("category"),
        "lang": case.get("lang"),
        "expectedLang": expected,
        "status": case.get("status"),
        "wallSec": case.get("wallSec"),
        "sessionId": case.get("sessionId"),
        "lang_output_msg": msg_lang,
        "lang_output_progress": prog_lang,
        "lang_msg_ok": lang_msg_ok,
        "lang_prog_ok": lang_prog_ok,
        "store_name_kept": store_name_kept,
        "monologue_clean": monologue_clean,
        "monologue_issues": monologue_issues,
        "invalid_ok": invalid_ok,
        "invalid_note": invalid_note,
        "mcp_question_lang_ok": mcp_q_ok,
        "mcp_question_lang": mcp_q_lang,
        "mcp_question_lang_note": mcp_q_note,
        "thai_cjk_clean": thai_cjk_clean,
        "thai_cjk_issues": thai_cjk_issues,
        "counts_msg": counts_msg,
        "counts_prog": counts_prog,
        "pass": overall,
        "fail_reasons": fail_reasons,
        "message_head": (message or "")[:200],
    }


def aggregate(scored: list[dict]) -> dict[str, Any]:
    total = len(scored)
    passed = sum(1 for s in scored if s["pass"])
    by_lang: dict[str, dict[str, int]] = defaultdict(lambda: {"total": 0, "pass": 0})
    by_category: dict[str, dict[str, int]] = defaultdict(lambda: {"total": 0, "pass": 0})
    clusters: dict[str, list[str]] = defaultdict(list)

    for s in scored:
        by_lang[s["lang"]]["total"] += 1
        by_category[s["category"]]["total"] += 1
        if s["pass"]:
            by_lang[s["lang"]]["pass"] += 1
            by_category[s["category"]]["pass"] += 1
        else:
            for reason in s["fail_reasons"]:
                key = reason.split(":")[0]
                clusters[key].append(s["caseId"])

    return {
        "total": total,
        "passed": passed,
        "passRate": round(passed / total, 4) if total else 0,
        "byLang": dict(by_lang),
        "byCategory": dict(by_category),
        "failureClusters": {k: v for k, v in sorted(clusters.items())},
    }


def render_txt(scored: list[dict], summary: dict[str, Any]) -> str:
    lines = [
        "GPOS Language Eval Summary",
        f"total={summary['total']} passed={summary['passed']} passRate={summary['passRate']:.1%}",
        "",
        "By language:",
    ]
    for lang, stats in sorted(summary["byLang"].items()):
        rate = stats["pass"] / stats["total"] if stats["total"] else 0
        lines.append(f"  {lang}: {stats['pass']}/{stats['total']} ({rate:.1%})")

    lines.append("")
    lines.append("Failure clusters:")
    for k, ids in summary.get("failureClusters", {}).items():
        lines.append(f"  {k}: {len(ids)} -> {', '.join(ids[:8])}{'...' if len(ids)>8 else ''}")

    lines.append("")
    lines.append("Failed cases:")
    for s in scored:
        if s["pass"]:
            continue
        lines.append(
            f"  {s['caseId']} session={s.get('sessionId','-')} reasons={';'.join(s['fail_reasons'])}"
        )
        lines.append(f"    head: {s['message_head'][:180]}")

    lines.append("")
    lines.append("All cases:")
    for s in sorted(scored, key=lambda x: x["caseId"]):
        mark = "PASS" if s["pass"] else "FAIL"
        thai_note = ""
        if s.get("lang") == "th" and s.get("thai_cjk_issues"):
            thai_note = f" cjk={';'.join(s['thai_cjk_issues'][:2])}"
        lines.append(
            f"  {mark} {s['caseId']:8} msg={s['lang_output_msg']:8} prog={s['lang_output_progress']:8} "
            f"wall={s.get('wallSec','-')}s{thai_note}"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Score GPOS lang eval cases")
    parser.add_argument("--cases-dir", type=Path, default=ROOT / "cases")
    parser.add_argument("--scored-dir", type=Path, default=ROOT / "scored")
    parser.add_argument("--results-dir", type=Path, default=ROOT / "results")
    parser.add_argument("--lang", action="append", dest="only_langs", choices=["zh", "en", "th"])
    args = parser.parse_args()

    args.scored_dir.mkdir(parents=True, exist_ok=True)
    args.results_dir.mkdir(parents=True, exist_ok=True)

    case_files = sorted(args.cases_dir.glob("Q*.json"))
    if args.only_langs:
        allowed = {f"_{lang}.json" for lang in args.only_langs}
        case_files = [p for p in case_files if any(p.name.endswith(s) for s in allowed)]
    if not case_files:
        print("no case files found", file=sys.stderr)
        return 1

    scored: list[dict] = []
    for path in case_files:
        with path.open(encoding="utf-8") as f:
            case = json.load(f)
        row = score_case(case)
        scored.append(row)
        with (args.scored_dir / path.name).open("w", encoding="utf-8") as f:
            json.dump(row, f, ensure_ascii=False, indent=2)

    summary_body = aggregate(scored)
    out = {"summary": summary_body, "cases": scored}
    summary_json = args.results_dir / "summary.json"
    summary_txt = args.results_dir / "summary.txt"
    with summary_json.open("w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=2)
    with summary_txt.open("w", encoding="utf-8") as f:
        f.write(render_txt(scored, summary_body))

    print(f"scored {len(scored)} cases -> {summary_json}", flush=True)
    print(f"pass {summary_body['passed']}/{summary_body['total']} ({summary_body['passRate']:.1%})", flush=True)
    return 0 if summary_body["passed"] == summary_body["total"] else 1


if __name__ == "__main__":
    sys.exit(main())
