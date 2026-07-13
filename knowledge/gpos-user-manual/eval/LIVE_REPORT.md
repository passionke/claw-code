# GPOS 产品手册前置意图 · 双语 Live 验收报告

Author: kejiqing  
Date: 2026-07-13  
Project: pre **projId=271**  
Active contentRev: **`2026-07-13_06-48-55`**（Admin 同构接口 commit/activate，有版本记录）

---

## 1. 结论

| 项 | 结果 |
|----|------|
| 全量 live | **105 / 105** 跑完 |
| 通过 | **101 / 105（96.19%）** |
| 语种链接正确率 `url_lang_ok` | **97.14%** |
| 错语种链接题数 | **2**（仍残留） |
| 会员双语对照 | **en/zh→en 链接、th→th 链接 均通过** |
| Admin 配置 | 经 `project_*_put_draft` → `commit_draft` → `activate`，非旁路改盘 |

达标对照计划门槛：内容通过率 ≥90% **已满足**；错语种链接未到 0（见 §6）。

---

## 2. 知识库产物（不经 LLM 改写）

源站：

- EN: https://gpos.co.th/en/user-manual  
- TH: https://gpos.co.th/th/user-manual（示例会员篇：https://gpos.co.th/th/user-manual/membership/add-member-back-office ）

本地种子：

```text
knowledge/gpos-user-manual/
  index.md
  en/   # 141 pages
  th/   # 140 pages
  eval/
```

- 爬取脚本：`scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all`
- 策略：**原文抽取落盘**，不做模型翻译/改写
- 同 slug 双语对照（如 `membership/add-member-back-office.md`）`source_url` 分别为 `/en/` 与 `/th/`

NAS 运行时（worker `/claw_ds/home/kb`）：

`/data/claw-nas/pre-claw-01/proj_271/home/.claw/project-home-versions/2026-07-13_06-48-55/home/kb/{en,th}/`

---

## 3. 配置变更（同构 Admin API · 有记录）

| 步骤 | contentRev / 说明 |
|------|-------------------|
| 双语路由首发 | `2026-07-13_05-49-00` note: `feat: bilingual GPOS KB routing th/en` |
| 语种硬锁加固 | `2026-07-13_06-48-55` note: `fix: hard lock en/th manual URL routing` |

变更面：

- Skill `product-manual-qa`：泰文→`kb/th`+`/th/user-manual`；其他→`kb/en`+`/en/user-manual`；禁 SQLBot；禁改写手册
- CLAUDE：手册语种 hard lock + how-to 不得进分析
- Rules：`product-manual-guard` 同步语种规则

仓库 fixtures 对齐：`scripts/fixtures/skills/product-manual-qa.SKILL.md`

---

## 4. 语言路由规则（落地）

| 用户输入 | KB | 官方链接 |
|----------|-----|----------|
| 含泰文字符 | `/claw_ds/home/kb/th` | `https://gpos.co.th/th/user-manual/...` |
| 中文 / English / 其他 | `/claw_ds/home/kb/en` | `https://gpos.co.th/en/user-manual/...` |

会员对照实测（`pair-add-member-*`）：

| 问法 | 期望链接语种 | 结果 |
|------|--------------|------|
| How do I add a member in Back Office? | en | pass → `/en/user-manual/membership/add-member-back-office` |
| 后台怎么新增会员？ | en | pass → `/en/...` |
| เพิ่มสมาชิกในระบบหลังบ้านอย่างไร? | th | pass → `/th/user-manual/membership/add-member-back-office` |

---

## 5. Live 105 题统计

题集：`eval/core-questions.jsonl`（en 35 + zh 35 + th 35）  
跑批：`eval/run_live_core_271.py` → `eval/results.jsonl`  
门店：`store_id=S002501221841976200006188`

| 语种 | 题数 | 通过 | 通过率 |
|------|------|------|--------|
| en | 35 | 34 | 97.1% |
| zh | 35 | 33 | 94.3% |
| th | 35 | 34 | 97.1% |
| **合计** | **105** | **101** | **96.19%** |

对失败子集在 `06-48-55` 加固后 **rerun** 并入最终 `results.jsonl`。

---

## 6. 仍失败的 4 题（证据）

详见 `eval/failures.md`（以最新 summary 为准）：

1. **en-…-qr-code-…-en-16** — 英文问句仍给出 `/th/` 链接（错语种）
2. **…-scan-to-order-settings-zh-15** — 中文问句给出 `/th/` 链接（错语种）
3. **…-label-production-…-zh-32** — 重跑 240s timeout
4. **th-…-kitchen-printers-…-th-27** — 已给 th 链接，但 `must_include` 要点命中不足（评分偏严/摘要过短）

建议后续：对错语种 2 题再收紧工具 path 允许列表（若平台支持）；timeout 题单独加长或降并发。

---

## 7. 产物清单

| 路径 | 用途 |
|------|------|
| `knowledge/gpos-user-manual/en|th/**` | 双语 KB |
| `eval/core-questions.jsonl` | 105 题 |
| `eval/results.jsonl` | 逐题 live 结果 |
| `eval/summary.json` / `summary.md` | 汇总 |
| `eval/failures.md` | 失败明细 |
| `docs/gpos-user-manual-kb-ops.md` | 运维（含 NAS rsync） |
| `scripts/gpos-manual-crawl/crawl_gpos_user_manual.py` | 双语爬取 |

---

## 8. 边界说明

- **配置变更**已走 Admin 同构接口并留下 `contentRev` 记录。  
- **KB** 仅 rsync 原文，无模型加工。  
- 经营分析链路未做 105 题干扰集全量；前期 smoke 已验证销售额仍走分析。
