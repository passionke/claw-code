---
name: product-manual-qa
description: 当用户询问 GPOS / POS / Back Office 产品操作 how-to 时使用。按用户语种快速路由静态手册：泰文输入→/claw_ds/home/kb/th + gpos.co.th/th 链接；其他语种→/claw_ds/home/kb/en + gpos.co.th/en 链接。禁止 SQLBot；禁止用模型改写手册原文。
---

# product-manual-qa（GPOS 产品操作手册 · 双语快路由）

Author: kejiqing

## 何时使用

GPOS/POS/Back Office **操作或配置步骤**（加商品、打印机、Grab、会员、折扣、分店、库存单据、扫码点餐等）。

**不属于**：销售额/收款占比/菜品销量等经营问数 → SQLBot。

## report_progress（强制，对商家可见）

知识库 how-to **不得**静默直出终答。阶段变化时必须调用 `report_progress`，遵守 CLAUDE.md 的 STATE MUTEX（同一助手轮次：要么只 progress，要么只终答）。

1. **查阅知识库文档**（调用 `grep_search` / `read_file` / `glob_search` **之前**的一轮）：仅 `report_progress`，无用户可见正文。
   - `current_task_desc` / `plan_title` / `todos[].title`：纯业务语、`[LANG_TAG]`、≤80 字
   - 中文示例：`查阅知识库文档`
   - 英文示例：`Looking up the knowledge base`
   - 泰文示例：`กำลังค้นหาเอกสารคู่มือ`
2. **整理知识库文档**（已读到手册要点、输出终答**之前**的一轮）：再报一次，仅 progress。
   - 中文示例：`整理知识库文档`
   - 英文示例：`Organizing knowledge base notes`
   - 泰文示例：`กำลังจัดเรียงเนื้อหาคู่มือ`
3. **终答轮**：只输出用户可见步骤 + `source_url`；**禁止**同轮再调 `report_progress`。

禁止在 progress 中暴露：`/claw_ds`、文件路径、工具名、`grep_search`、KB 目录等内部细节。

## 语言路由（强制）

| 用户输入 | KB 目录 | 官方链接前缀 |
|----------|---------|--------------|
| **泰文**（含泰文字符） | `/claw_ds/home/kb/th/` | `https://gpos.co.th/th/user-manual/...` |
| **其他**（中文 / English / 其它） | `/claw_ds/home/kb/en/` | `https://gpos.co.th/en/user-manual/...` |

- 先读 `/claw_ds/home/kb/index.md` 可确认双语入口。
- **禁止**把泰文问句检索到 `en/` 后把英文链接当泰文答案来源。
- **禁止**用大模型「翻译/改写」手册正文；只从命中 md 摘 3–8 条要点 + 原文 `source_url`。

## 禁止

1. 禁止 `mcp__sqlbot*` / `mcp_isolated_question_analysis`。
2. 禁止编造菜单路径；0 命中则给对应语种手册首页：  
   - 泰：`https://gpos.co.th/th/user-manual`  
   - 其他：`https://gpos.co.th/en/user-manual`
3. 禁止向用户暴露 `/claw_ds` 等内部路径。

## 检索协议

1. 先按上一节完成「查阅知识库文档」progress。
2. 按上表选定 `KB_LANG_ROOT`（`.../kb/th` 或 `.../kb/en`）。
3. 可选 `read_file` → `$KB_LANG_ROOT/index.md`。
4. `grep_search(path=$KB_LANG_ROOT, pattern=<关键词>)`。
5. `read_file` 命中文章；  
   - 官网页：使用 frontmatter `source_url`（`gpos.co.th`，语种与路由一致）。  
   - 内部 FAQ：摘步骤即可；**勿**把 `internal:` / Mind URL 写进终答。
6. 0 命中：`glob_search(path=$KB_LANG_ROOT, pattern=**/*.md)` 后再读 1–2 篇；仍无则回手册首页链接。
7. 终答前完成「整理知识库文档」progress，再输出终答。

## 输出（对用户可见）

用用户语种，结构固定：

1. 一句结论  
2. 3–8 条步骤（摘自 KB，勿整页粘贴、勿二次创作）  
3. **必须**给出该文章 `source_url`，规则如下：  
   - **官网手册**（`source_url` 以 `https://gpos.co.th/` 开头）：终答**必须**原样附上该链接。  
   - **内部 FAQ**（`source_url` 以 `internal:` 开头，或路径含 `internal/mind/faq`）：**禁止**向用户输出任何 Mind / 内部文档 URL；只输出步骤，**不要**加「来源」「📎」链接行。  
   - **禁止**输出 `mind.maxiot-inc.com` 任意链接。

## 执行

载入本 skill 后：先 `report_progress`（查阅）→ 按语种路由检索 → `report_progress`（整理）→ 输出终答。不要调用经营分析 MCP。

## Hard language lock (critical)

- If the user message contains **Thai script** (`\u0E00-\u0E7F`):  
  - `grep_search` / `read_file` / `glob_search` path MUST be under `/claw_ds/home/kb/th` only.  
  - Final answer MUST contain a `https://gpos.co.th/th/user-manual/...` link and MUST NOT contain `/en/user-manual/`.
- Otherwise (Chinese / English / mixed without Thai letters):  
  - path MUST be under `/claw_ds/home/kb/en` only.  
  - If answer cites **official** manual: MUST contain `https://gpos.co.th/en/user-manual/...` and MUST NOT contain `/th/user-manual/`.  
  - If answer uses **internal FAQ only**: steps only; MUST NOT contain `mind.maxiot-inc.com` or `internal:` URLs.
- If you already opened the wrong language tree, stop and switch before answering.
- Discount / tax / printer / sales-channel **setup how-to** is still product-manual — never SQLBot.
