#!/usr/bin/env python3
"""Patch proj 27 CLAUDE.md Thai anti-CJK rules and push to pre-prod. Author: kejiqing"""

from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request

PRE = "http://192.168.9.252:18088"
PROJ = 27

# --- v1 blocks (initial diagnostic/lang-check upgrade) ---
LANGUAGE_LOCK_BLOCK = """#### Language lock（本节范例优先于下文中文措辞）
- Gateway 已注入 `[LANG_TAG]`。下文凡中文仅为**结构说明**，生成时必须换成 `[LANG_TAG]` 对应措辞。
- **禁止抄写**进用户可见正文（当 `[LANG_TAG]` ≠ Chinese）：`数据显示`、`建议考虑`、`做什么`、`怎么做`、`依据何在`、`话术`、`门店`（`store_name` 专名除外）。
- 店名 `ทองหล่อมินิมาร์ท` 等保持原文，不翻译；句子语法与小节标题仍须 100% `[LANG_TAG]`。
- **`[LANG_TAG]=Thai` 时整份报告（含 `**การวิเคราะห์เชิงวินิจฉัย**` 小节正文）禁止出现任何 CJK 汉字**；不得泰文标题 + 中文段落。事实句用 `ข้อมูลแสดงว่า…`，建议句用 `พิจารณา…`。

"""

OLD_DIAG = """**诊断性解读**

- 陈述事实用"数据显示…"，提出推论用"建议考虑…"。
- 因果链必须完整（如"套餐渗透率为0% → 缺乏组合溢价 → 利润空间受限"）。
- 解读范围限于当前实体（门店或机构下属门店），禁用行业基准或平台均值。
- 仅当数据波动显著（如客单价突降≥20%）时，才标记为风险或机会。
- 所有文字描述必须与表格数字严格一致，禁止矛盾或模糊指代（如"热销品类"需明确为"饮品"而非笼统归为"啤酒"）。
- **预测类任务中，必须区分"历史事实"与"未来推演"，后者需标注假设条件**。
- **所有提及金额处必须附带"泰铢"或"THB"单位**。

**可落地的建议（≤4项）**

- 按影响力与可行性排序，聚焦店长可执行动作（话术、定价、物料、菜单）。
- 每条必须包含：
    - **做什么**（如"上线饮品+零食套餐"）
    - **怎么做**（如"定价75泰铢，服务员话术：'很多客人选这个组合，省5泰铢还吃得更丰富'"）
    - **依据何在**（如"饮品贡献销售额TOP1，占529件"——**必须基于可信销售额或订单占比，而非异常销量**）
"""

NEW_DIAG = """**诊断性解读**

- 陈述事实：使用 `[LANG_TAG]` 的「数据呈现」句式 — Chinese: 数据显示… / English: Data shows… / Thai: ข้อมูลแสดงว่า…（**不得**在 Thai/English 输出里写「数据显示」字面）。
- 提出推论：使用 `[LANG_TAG]` 的「审慎建议」句式 — Chinese: 建议考虑… / English: Consider… / Thai: พิจารณา…（**不得**混用其他语种套话）。
- **`[LANG_TAG]=Thai`：本小节标题可用 `**การวิเคราะห์เชิงวินิจฉัย**`，但正文必须 100% 泰文，禁止夹杂中文句或 CJK 字符。**
- 因果链必须完整（如"套餐渗透率为0% → 缺乏组合溢价 → 利润空间受限"）。
- 解读范围限于当前实体（门店或机构下属门店），禁用行业基准或平台均值。
- 仅当数据波动显著（如客单价突降≥20%）时，才标记为风险或机会。
- 所有文字描述必须与表格数字严格一致，禁止矛盾或模糊指代（如"热销品类"需明确为"饮品"而非笼统归为"啤酒"）。
- **预测类任务中，必须区分"历史事实"与"未来推演"，后者需标注假设条件**。
- **所有提及金额处必须附带"泰铢"或"THB"单位**。

**可落地的建议（≤4项）**

- 按影响力与可行性排序，聚焦店长可执行动作（定价、物料、菜单、员工口径）。
- 每条必须包含三小节，**小节标题与正文均须 `[LANG_TAG]`**（禁止抄写中文标签）：
    - **Action** / **做什么** / **ทำอะไร**
    - **How** / **怎么做** / **ทำอย่างไร**（含员工台词示例，台词语言 = `[LANG_TAG]`）
    - **Rationale** / **依据** / **เหตุผล**（基于可信销售额或订单占比，**禁止**输出「依据何在」字面）
"""

OLD_LANG_CHECK = """**语言一致性验证**：
- 确认用户提问语言（中/英/泰）。
- 检查所有输出（包括 `report_progress` 中的 `current_task_desc` 等字段）是否已严格切换为对应语言。
- **严禁出现“中文提问，英文/泰文输出”或混合语言的情况**。"""

NEW_LANG_CHECK = """**语言一致性验证**：
- 以 gateway **已注入的 `[LANG_TAG]`** 为准（非店名、非 SQLBot 返回原文语种）。
- 全文扫描：是否存在与 `[LANG_TAG]` 不符的整句（店名/商品名/支付渠道名除外）。
- **模板泄漏**：`[LANG_TAG]` 为 Thai 或 English 时，正文不得出现「数据显示」「依据何在」「依据:」「话术」等中文套话。
- **`[LANG_TAG]=Thai` 专项**：最终报告任意连续 ≥8 个 CJK 汉字、或出现 `数据显示` 等中文套话 → **整份报告作废并重写为纯泰文**。
- 检查 `report_progress`（`current_task_desc` 等）与最终报告语言一致，且 **progress 描述的业务任务 = 用户本轮问题**。
- **MCP `question` 语种**：每个 `mcp_isolated_question_analysis` 的 `question` 参数 dominant script = `[LANG_TAG]`（店名专名除外）。
- **严禁** Thai 提问却夹杂未翻译的中文句（含诊断段）。**严禁**中文提问却泰文/英文全文作答。"""

# --- v2 supplement: footnotes / 客流 ---
TRAFFIC_METRIC_OLD = (
    '客流数据缺失时，禁用"转化率""人均消费"等需客流支撑的术语，'
    "仅基于订单量、金额、客单价（=总金额÷订单数）进行分析。"
)

TRAFFIC_METRIC_NEW = """客流数据缺失时，禁用"转化率""人均消费"等需客流支撑的术语，仅基于订单量、金额、客单价（=总金额÷订单数）进行分析。
- **`[LANG_TAG]=Thai` 缺客流量时**：脚注/备注写 `*หมายเหตุ: ข้อมูลปริมาณลูกค้าไม่พร้อมใช้งาน การวิเคราะห์ใช้จำนวนออเดอร์และยอดขายเท่านั้น*` — **禁止**在泰文句中插入 `客流`、`转化率` 等 CJK 术语字面。"""

FOOTNOTE_LANG_CHECK = """- **脚注/备注（Thai）**：`blockquote`、`*หมายเหตุ*`、表格脚注同样禁止 CJK；不得 `ข้อมูล客流…` 这类泰中夹杂。
"""

THAI_SPECIAL_ANCHOR = "- **`[LANG_TAG]=Thai` 专项**："

# --- v3: 客流借词 → 泰文替换表（Q01/Q13 建议段泄漏）---
LANG_LOCK_V2_TAIL = (
    "事实句用 `ข้อมูลแสดงว่า…`，建议句用 `พิจารณา…`。\n\n"
)
LANG_LOCK_V3_TAIL = """事实句用 `ข้อมูลแสดงว่า…`，建议句用 `พิจารณา…`。
- **泰文禁止借词 `客流`**（含 `แม้客流น้อย`、`ดึง客流`、`ตัวดึง客流` 等泰中夹杂）。**`คำแนะนำที่ปฏิบัติได้จริง` 的 `ทำอะไร` / `ทำอย่างไร` / `เหตุผล` 同样适用。**
- **泰文替代表**（仅 `[LANG_TAG]=Thai` 输出）：
  - 客流量 / 客流少 → `ปริมาณลูกค้า` / `ลูกค้าน้อย` / `มีลูกค้าไม่มาก`
  - 拉客流 / 引流 → `ดึงดูดลูกค้า` / `เพิ่มการเข้าร้าน` / `ตัวดึงดูดลูกค้า`
  - ❌ `แม้客流น้อย` → ✅ `แม้ลูกค้าน้อย`；❌ `ตัวดึง客流` → ✅ `ตัวดึงดูดลูกค้า`

"""

RECOMMENDATIONS_V2_TAIL = (
    "    - **Rationale** / **依据** / **เหตุผล**（基于可信销售额或订单占比，**禁止**输出「依据何在」字面）\n"
)
RECOMMENDATIONS_V3_EXTRA = """    - **Rationale** / **依据** / **เหตุผล**（基于可信销售额或订单占比，**禁止**输出「依据何在」字面）
- **`[LANG_TAG]=Thai` 建议段**：`ทำอะไร` / `ทำอย่างไร` / `เหตุผล` 正文 **零 CJK**（含单字 `客流`）；输出前扫描全文，命中则重写该条建议。
"""

LANG_CHECK_V3_EXTRA = """- **借词扫描（Thai）**：全文不得出现 `客流` 字面（含嵌入泰语句中的 `แม้客流น้อย`、`ดึง客流`）；命中则作废并重写为 `ปริมาณลูกค้า` / `ดึงดูดลูกค้า` 等纯泰文表述。
"""


def req(method: str, url: str, body: dict | None = None) -> dict:
    data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
    r = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(r, timeout=60) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"{method} {url} -> HTTP {e.code}: {e.read().decode()[:600]}")


def patch_claude_md(md: str) -> tuple[str, list[str]]:
    """Idempotent patches; returns (new_md, applied_labels)."""
    applied: list[str] = []
    out = md

    if OLD_DIAG in out:
        out = out.replace(OLD_DIAG, NEW_DIAG, 1)
        applied.append("v1_diagnostic")
    if OLD_LANG_CHECK in out:
        out = out.replace(OLD_LANG_CHECK, NEW_LANG_CHECK, 1)
        applied.append("v1_lang_check")

    anchor = "### 6. 结构化输出格式（强制顺序）\n\n"
    if LANGUAGE_LOCK_BLOCK.strip() not in out and anchor in out:
        out = out.replace(anchor, anchor + LANGUAGE_LOCK_BLOCK, 1)
        applied.append("v1_language_lock")

    if TRAFFIC_METRIC_OLD in out and "ข้อมูลปริมาณลูกค้าไม่พร้อมใช้งาน" not in out:
        out = out.replace(TRAFFIC_METRIC_OLD, TRAFFIC_METRIC_NEW, 1)
        applied.append("v2_traffic_footnote")

    if (
        FOOTNOTE_LANG_CHECK.strip() not in out
        and THAI_SPECIAL_ANCHOR in out
        and "**脚注/备注（Thai）**" not in out
    ):
        out = out.replace(THAI_SPECIAL_ANCHOR, FOOTNOTE_LANG_CHECK + THAI_SPECIAL_ANCHOR, 1)
        applied.append("v2_footnote_lang_check")

    if LANG_LOCK_V2_TAIL in out and "泰文禁止借词 `客流`" not in out:
        out = out.replace(LANG_LOCK_V2_TAIL, LANG_LOCK_V3_TAIL, 1)
        applied.append("v3_lang_lock_traffic_glossary")

    if RECOMMENDATIONS_V2_TAIL in out and "建议段" not in out.split(RECOMMENDATIONS_V2_TAIL, 1)[1][:120]:
        out = out.replace(RECOMMENDATIONS_V2_TAIL, RECOMMENDATIONS_V3_EXTRA, 1)
        applied.append("v3_recommendations_zero_cjk")

    if (
        "**借词扫描（Thai）**" not in out
        and THAI_SPECIAL_ANCHOR in out
        and LANG_CHECK_V3_EXTRA.strip() not in out
    ):
        out = out.replace(THAI_SPECIAL_ANCHOR, LANG_CHECK_V3_EXTRA + THAI_SPECIAL_ANCHOR, 1)
        applied.append("v3_lang_check_traffic_borrow")

    return out, applied


def push_config(cur: dict, md: str, note: str) -> str:
    payload = {
        "rulesJson": cur["rulesJson"],
        "mcpServersJson": cur["mcpServersJson"],
        "skillsSourcesJson": cur.get("skillsSourcesJson") or [],
        "skillsJson": cur["skillsJson"],
        "allowedToolsJson": cur["allowedToolsJson"],
        "claudeMd": md,
        "languagePipelineJson": cur.get("languagePipelineJson"),
    }
    for k in (
        "gitSyncJson",
        "solvePreflightJson",
        "solveOrchestrationJson",
        "extraSessionFieldsJson",
        "promptLimitsJson",
        "workerIsolationJson",
    ):
        if k in cur and cur[k] is not None:
            payload[k] = cur[k]

    req("PUT", f"{PRE}/v1/project/config/{PROJ}", payload)
    commit = req(
        "POST",
        f"{PRE}/v1/project/config/{PROJ}/versions/commit",
        {"note": note},
    )
    rev = commit["savedContentRev"]
    act = req("POST", f"{PRE}/v1/project/config/{PROJ}/versions/{rev}/activate")
    print("activated", act.get("activated"), "rev", rev)
    return rev


def main() -> int:
    cur = req("GET", f"{PRE}/v1/project/config/{PROJ}")
    md = cur.get("claudeMd") or ""
    patched, applied = patch_claude_md(md)
    if not applied:
        print("no patches applied (already up to date)")
        return 0
    print(f"claudeMd chars: {len(md)} -> {len(patched)}")
    print("applied:", ", ".join(applied))
    rev = push_config(
        cur,
        patched,
        "Thai 客流借词替换表 + 建议段零CJK自检 (kejiqing)",
    )
    verify = req("GET", f"{PRE}/v1/project/config/{PROJ}")
    vmd = verify.get("claudeMd") or ""
    print("verify ข้อมูลปริมาณลูกค้า", "ข้อมูลปริมาณลูกค้าไม่พร้อมใช้งาน" in vmd)
    print("verify footnote rule", "**脚注/备注（Thai）**" in vmd)
    print("verify traffic glossary", "泰文禁止借词 `客流`" in vmd)
    print("verify borrow scan", "**借词扫描（Thai）**" in vmd)
    print("stableContentRev", verify.get("stableContentRev"), "saved", rev)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
