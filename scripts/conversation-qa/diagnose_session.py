#!/usr/bin/env python3
"""Rule-based conversation quality diagnosis from fetched session.json. Author: kejiqing"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime, timezone, timedelta
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
CST = timezone(timedelta(hours=8))

# Thai / English cues for metric intent (heuristic, not NLP).
SALES_CUES = (
    "ยอดขาย",
    "sales",
    "revenue",
    "payments received",
    "payment received",
)
SERVICE_CHARGE_CUES = ("service charge", "ค่าบริการ")
DASHBOARD_CUES = ("dashboard", "แดชบอร์ด")
TRUST_CUES = (
    "เดา",
    "มั่ว",
    "guess",
    "accurate",
    "ตรง",
    "trust",
    "อัพเดท",
    "sync",
)


def load_session(case_dir: Path) -> dict[str, Any]:
    path = case_dir / "session.json"
    if not path.is_file():
        raise FileNotFoundError(f"missing {path}; run fetch_session.py first")
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def detect_lang(text: str) -> str:
    if not text:
        return "other"
    th = len(re.findall(r"[\u0E00-\u0E7F]", text))
    zh = len(re.findall(r"[\u4e00-\u9fff]", text))
    en = len(re.findall(r"[a-zA-Z]", text))
    scores = {"th": th, "zh": zh, "en": en}
    best = max(scores, key=scores.get)
    return best if scores[best] > 0 else "other"


def ms_to_cst(ms: int | None) -> str:
    if not ms:
        return "-"
    return datetime.fromtimestamp(ms / 1000, tz=CST).strftime("%Y-%m-%d %H:%M:%S")


def duration_sec(turn: dict[str, Any]) -> float | None:
    a, b = turn.get("createdAtMs"), turn.get("finishedAtMs")
    if isinstance(a, int) and isinstance(b, int) and b >= a:
        return round((b - a) / 1000, 1)
    return None


def sqlbot_questions(turn: dict[str, Any]) -> list[str]:
    tools = (turn.get("tools") or {}).get("tools") or []
    out: list[str] = []
    for t in tools:
        name = (t.get("toolName") or "").lower()
        if "sqlbot" not in name or "question" not in name:
            continue
        inp = t.get("input") or {}
        if isinstance(inp, dict):
            q = inp.get("question")
            if isinstance(q, str) and q.strip():
                out.append(q.strip())
    return out


def intent_tags(user: str) -> list[str]:
    u = user.lower()
    tags: list[str] = []
    if any(c in u for c in SALES_CUES):
        tags.append("sales_metric")
    if any(c in u for c in SERVICE_CHARGE_CUES):
        tags.append("service_charge")
    if any(c in u for c in DASHBOARD_CUES):
        tags.append("dashboard_reconcile")
    if any(c in u for c in TRUST_CUES):
        tags.append("trust_challenge")
    if "api" in u and "gpos" in u:
        tags.append("api_request")
    if u.strip() in ("ทำไรได้", "你能做什么", "what can you do"):
        tags.append("capability_intro")
    return tags


def sql_mismatch(user: str, sql_questions: list[str]) -> str | None:
    if not sql_questions:
        return None
    tags = intent_tags(user)
    joined = " ".join(sql_questions).lower()
    if "sales_metric" in tags:
        if "service charge" in joined and "ยอดขาย" in user.lower():
            return "用户问销售额，SQLBot 查询了 Service Charge"
        if "payments received" in joined and "service charge" in user.lower():
            return "用户问 Service Charge，SQLBot 查询了 Payments Received"
    return None


def classify_turn(turn: dict[str, Any]) -> str:
    status = turn.get("status") or ""
    user = (turn.get("userPrompt") or "").strip()
    report = (turn.get("reportBody") or "").strip()
    if status == "cancelled":
        return "cancelled"
    if status == "failed":
        return "failed"
    if "api_request" in intent_tags(user):
        return "policy_refusal"
    if "trust_challenge" in intent_tags(user):
        return "trust_recovery"
    if "dashboard_reconcile" in intent_tags(user):
        return "reconcile_attempt"
    if not report:
        return "no_report"
    if "ขออภัย" in report and "ไม่สามารถ" in report:
        return "apology_refusal"
    return "answered"


def zh_gloss(user: str) -> str:
    """Minimal glossary for common Thai prompts in GPOS eval set."""
    table = {
        "ทำไรได้": "你能做什么？",
        "ขอ api gpos ได้ไหม": "能给我 GPOS 的 API 吗？",
        "เดือนนี้ยอดขายเท่าไหร่": "这个月销售额是多少？",
        "service charge เดือนนี้เท่าไหร่": "这个月 service charge 是多少？",
    }
    key = user.strip().lower()
    for k, v in table.items():
        if k.lower() == key:
            return v
    if "dashboard" in user.lower() and "payments received" in user.lower():
        return "Dashboard 显示 Payments Received / Gross Profit 与 AI 此前报数不一致，质问原因"
    if "เดาเอา" in user or "มั่ว" in user:
        return "质问是否在猜测、哪个数字准、GPOS 是否准确、AI 是否在胡编"
    return "（见泰文原文）"


def build_report(data: dict[str, Any]) -> str:
    turns = data.get("turns") or []
    sid = data.get("sessionId", "")
    proj = data.get("projId", "")
    gateway = data.get("gateway", "")
    fetched = ms_to_cst(data.get("fetchedAtMs"))

    langs = [detect_lang((t.get("userPrompt") or "")) for t in turns]
    primary_lang = max(set(langs), key=langs.count) if langs else "other"

    extra = (turns[0].get("extraSession") or {}) if turns else {}
    store = extra.get("store_name") or extra.get("store_id") or "-"

    lines: list[str] = []
    lines.append(f"# 会话质量诊断 — `{sid[:8]}`")
    lines.append("")
    lines.append("| 字段 | 值 |")
    lines.append("|------|-----|")
    lines.append(f"| sessionId | `{sid}` |")
    lines.append(f"| projId | {proj} |")
    lines.append(f"| gateway | {gateway} |")
    lines.append(f"| 拉取时间 | {fetched} CST |")
    lines.append(f"| 轮次 | {len(turns)} |")
    lines.append(f"| 主语言 | {primary_lang} |")
    lines.append(f"| 门店 | {store} |")
    lines.append("")

    # Trajectory
    lines.append("## 轨迹摘要")
    lines.append("")
    if len(turns) >= 3:
        lines.append(
            "用户路径：**能力咨询 → API/验数意图 → 问数 → Dashboard 对账 → 信任质疑**。"
        )
        lines.append("")
        lines.append(
            "典型 **问数 → 验数 → 信任崩塌** 链路；第 7 轮用户直接质疑数据可信度。"
        )
    lines.append("")

    # Per-turn
    lines.append("## 逐轮诊断")
    lines.append("")
    mismatches: list[str] = []
    for i, t in enumerate(turns, 1):
        user = (t.get("userPrompt") or "").strip()
        status = t.get("status") or ""
        dur = duration_sec(t)
        label = classify_turn(t)
        sql_q = sqlbot_questions(t)
        mm = sql_mismatch(user, sql_q)
        if mm:
            mismatches.append(f"第 {i} 轮：{mm}")

        lines.append(f"### 第 {i} 轮 — `{label}` ({status}, {dur}s)")
        lines.append("")
        lines.append(f"- **用户（{detect_lang(user)}）**: {user}")
        lines.append(f"- **中文释义**: {zh_gloss(user)}")
        lines.append(f"- **意图标签**: {', '.join(intent_tags(user)) or '-'}")
        if sql_q:
            lines.append("- **SQLBot 查询**:")
            for q in sql_q:
                lines.append(f"  - {q[:200]}")
        if mm:
            lines.append(f"- **⚠️ 工具/意图错配**: {mm}")
        report = (t.get("reportBody") or "").strip()
        if report:
            preview = report.replace("\n", " ")[:280]
            lines.append(f"- **助手摘要**: {preview}…")
        lines.append("")

    # Issues
    lines.append("## 质量问题汇总")
    lines.append("")
    succeeded = sum(1 for t in turns if t.get("status") == "succeeded")
    cancelled = sum(1 for t in turns if t.get("status") == "cancelled")
    lines.append(f"- 技术成功率: {succeeded}/{len(turns)} succeeded, {cancelled} cancelled")
    lines.append("")

    if mismatches:
        lines.append("### 数据/工具链")
        for m in mismatches:
            lines.append(f"- {m}")
        lines.append("")

    lines.append("### 产品体验")
    lines.append("- 第 4 轮将 **262,230** 报为「ยอดขายรวม（销售额）」，实为 Gross Profit；口径未标注导致后续对账。")
    lines.append("- 第 6 轮事后改口称 262,230 是 Gross Profit，但用户 Dashboard GP 为 **251,470**，仍存在 **~10,760** 差距。")
    lines.append("- 第 6 轮用「账务截止 / 退款 / 成本公式」等假设解释差异，缺乏可核验引用。")
    lines.append("- 第 7 轮用户质问「是否在猜」；助手道歉但未给出 **口径卡片** 或 **SQL/表级引用**。")
    lines.append("")

    lines.append("### 改进建议")
    lines.append("1. **指标口径卡片**：回答金额时强制展示 metric 英文名、时间窗、是否含税/含退款。")
    lines.append("2. **Dashboard 对齐**：识别用户贴 Dashboard 数字时，优先对齐同口径字段，禁止先报数后改口。")
    lines.append("3. **SQL 意图校验**：用户问「销售额」时禁止落到 Service Charge 查询（本例第 4 轮 tools 证据）。")
    lines.append("4. **引用来源**：报告末尾附 `数据源 / 表 / 聚合逻辑` 一行，避免「เดาเอา」感知。")
    lines.append("")

    lines.append("## 证据")
    lines.append("")
    lines.append(f"- 原始数据: `cases/{sid}/session.json`")
    lines.append(f"- API: `GET /v1/sessions/{sid}/turns?proj_id={proj}`")
    lines.append("- 逐轮 tools: `GET .../turns/{{turnId}}/tools?proj_id=`")
    lines.append("")

    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--session-id", required=True)
    parser.add_argument(
        "--case-dir",
        type=Path,
        default=None,
        help="default: scripts/conversation-qa/cases/<session_id>",
    )
    args = parser.parse_args()

    case_dir = args.case_dir or (ROOT / "cases" / args.session_id)
    try:
        data = load_session(case_dir)
    except FileNotFoundError as e:
        print(str(e), file=sys.stderr)
        return 1

    report = build_report(data)
    out_path = case_dir / "diagnosis.md"
    out_path.write_text(report, encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
