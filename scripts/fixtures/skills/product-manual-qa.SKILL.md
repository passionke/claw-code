---
name: product-manual-qa
description: 当用户询问 GPOS / POS / Back Office 产品操作 how-to 时使用。按用户语种快速路由静态手册：泰文输入→/claw_ds/home/kb/th + gpos.co.th/th 链接；其他语种→/claw_ds/home/kb/en + gpos.co.th/en 链接。禁止 SQLBot；禁止用模型改写手册原文。
---

# product-manual-qa（GPOS 产品操作手册 · 双语快路由）

Author: kejiqing

## 何时使用

GPOS/POS/Back Office **操作或配置步骤**（加商品、打印机、Grab、会员、折扣、分店、库存单据、扫码点餐等）。

**不属于**：销售额/收款占比/菜品销量等经营问数 → SQLBot。

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

1. 按上表选定 `KB_LANG_ROOT`（`.../kb/th` 或 `.../kb/en`）。
2. 可选 `read_file` → `$KB_LANG_ROOT/index.md`。
3. `grep_search(path=$KB_LANG_ROOT, pattern=<关键词>)`。
4. `read_file` 命中文章；使用 frontmatter 的 `source_url`（必须与语种一致：`/th/` 或 `/en/`）。
5. 0 命中：`glob_search(path=$KB_LANG_ROOT, pattern=**/*.md)` 后再读 1–2 篇；仍无则回手册首页链接。

## 输出（对用户可见）

用用户语种，结构固定：

1. 一句结论  
2. 3–8 条步骤（摘自 KB，勿整页粘贴、勿二次创作）  
3. **必须**给出该文章 `source_url`（泰文问题 → th 链接；否则 → en 链接）

## 执行

载入后立刻按语种路由检索并输出；不要调用经营分析 MCP。


## Hard language lock (critical)

- If the user message contains **Thai script** (`\u0E00-\u0E7F`):  
  - `grep_search` / `read_file` / `glob_search` path MUST be under `/claw_ds/home/kb/th` only.  
  - Final answer MUST contain a `https://gpos.co.th/th/user-manual/...` link and MUST NOT contain `/en/user-manual/`.
- Otherwise (Chinese / English / mixed without Thai letters):  
  - path MUST be under `/claw_ds/home/kb/en` only.  
  - Final answer MUST contain `https://gpos.co.th/en/user-manual/...` and MUST NOT contain `/th/user-manual/`.
- If you already opened the wrong language tree, stop and switch before answering.
- Discount / tax / printer / sales-channel **setup how-to** is still product-manual — never SQLBot.

