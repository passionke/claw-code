#!/usr/bin/env python3
# Build DingTalk markdown payloads from eval results. Author: kejiqing
"""Generate /tmp/payload-*.json for manual or MCP update_document calls."""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MAIN_NODE = "XPwkYGxZV3RPe0qpt3QgkZgMWAgozOKL"
DETAIL_NODE = "lyQod3RxJK3glrYMU46QRbdDJkb4Mw9r"


def build_markdown() -> dict[str, str]:
    questions = json.loads((ROOT / "questions.json").read_text(encoding="utf-8"))["questions"]
    results = json.loads((ROOT / "results/summary.json").read_text(encoding="utf-8"))
    summary = results["summary"]
    scored = results["cases"]
    by_id = {s["caseId"]: s for s in scored}
    passed = summary["passed"]
    total = summary["total"]
    pass_pct = summary["passRate"] * 100

    a = [
        "# GPOS 经营问题语言评测报告（30×3）",
        "",
        "Author: kejiqing | **每条结果均标注 sessionId**",
        "",
        "## 一、评测概述",
        "",
        "| 项 | 值 |",
        "| --- | --- |",
        "| 环境 | http://10.22.11.19:18088 |",
        "| 项目 | projId=10 |",
        f"| 通过率 | **{passed}/{total}（{pass_pct:.1f}%）** |",
        "",
        "> Admin：`GET /v1/tasks/{sessionId}?proj_id=10`",
        "",
        "## 二、30 道评测题",
        "",
        "| ID | 类别 | 中文 | English | ไทย |",
        "| --- | --- | --- | --- | --- |",
    ]
    for q in questions:
        a.append(f"| {q['id']} | {q['category']} | {q['zh']} | {q['en']} | {q['th']} |")

    b = [
        "",
        "## 三、90 轮结果（含 sessionId）",
        "",
        "| Case | 结果 | 输出语种 | sessionId | 失败原因 |",
        "| --- | --- | --- | --- | --- |",
    ]
    for q in questions:
        for lang in ["zh", "en", "th"]:
            cid = f"{q['id']}_{lang}"
            s = by_id[cid]
            reasons = "; ".join(s.get("fail_reasons", [])) or "—"
            b.append(
                f"| {cid} | {'PASS' if s['pass'] else 'FAIL'} | {s['lang_output_msg']} "
                f"| {s.get('sessionId', '—')} | {reasons} |"
            )

    c = ["", "## 四、FAIL 明细（sessionId）", ""]
    for s in sorted(scored, key=lambda x: x["caseId"]):
        if s["pass"]:
            continue
        c.append(f"### {s['caseId']} — `{s.get('sessionId')}`")
        c.append(f"- 原因: {'; '.join(s.get('fail_reasons', []))}")
        c.append(f"- 开头: {s.get('message_head', '')[:180]}")
        c.append("")

    d = [
        "## 五、结论",
        "",
        "**语言**：泰 30/30；英 29/30（Q30_en 无效题未短拒）；中 26/30（Q22_zh 语种漂移 + Q16/23/24_zh solve 失败）。",
        "**独白**：仅检测报告开头是否出现规划/自言自语（如 Now let me、让我先查），不检查 Markdown 标题格式。",
        "**店名**：泰文店名不翻译 = 预期。",
    ]

    parts = {
        "part-a": "\n".join(a),
        "part-b": "\n".join(b),
        "part-c": "\n".join(c),
        "part-d": "\n".join(d),
        "full-detail": "# GPOS评测结果明细90轮（含 sessionId）\n\nAuthor: kejiqing\n\n"
        + "\n".join(b)
        + "\n"
        + "\n".join(c),
    }
    return parts


def main() -> None:
    parts = build_markdown()
    out_dir = Path("/tmp/gpos-dingtalk-payloads")
    out_dir.mkdir(exist_ok=True)

    json.dump(
        {"nodeId": MAIN_NODE, "mode": "overwrite", "markdown": parts["part-a"]},
        open(out_dir / "main-overwrite.json", "w"),
        ensure_ascii=False,
    )
    for key, mode in [("part-b", "append"), ("part-c", "append"), ("part-d", "append")]:
        json.dump(
            {"nodeId": MAIN_NODE, "mode": mode, "markdown": parts[key]},
            open(out_dir / f"main-{key}.json", "w"),
            ensure_ascii=False,
        )
    json.dump(
        {"nodeId": DETAIL_NODE, "mode": "overwrite", "markdown": parts["full-detail"]},
        open(out_dir / "detail-overwrite.json", "w"),
        ensure_ascii=False,
    )
    print(f"wrote payloads to {out_dir}")


if __name__ == "__main__":
    main()
