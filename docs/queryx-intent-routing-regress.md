# QueryX 前置意图分流 — 回归清单

Author: kejiqing

**上线与日常运维真源：** [`docs/gpos-user-manual-kb-ops.md`](gpos-user-manual-kb-ops.md)  
本文只列验收检查项；发布 / rsync / 回滚步骤见运维手册。

目标 project：预发 **271**；生产 **27**（或灰度 id）。先完成 KB 同步与 Admin **commit + activate**。

---

## 0. 离线 / 题集

```bash
# 重建双语题集（≥100，含 en/zh/th）
python3 knowledge/gpos-user-manual/eval/run_live_core_271.py --build-only --min 100
wc -l knowledge/gpos-user-manual/eval/core-questions.jsonl
```

要求：题数 ≥100；`expected_url_lang` 与问句语种一致（泰→`th`，其他→`en`）。

---

## 1. 产品手册（抽样 + 全量）

冒烟：

```bash
python3 knowledge/gpos-user-manual/eval/route_smoke_271.py
```

| 问法 | 期望 |
|------|------|
| How do I add a product in Back Office? | 手册 + **en** 链接 |
| 后台怎么创建商品分类？ | 手册 + **en** 链接 |
| เพิ่มสมาชิกในระบบหลังบ้านอย่างไร? | 手册 + **th** 链接 |

全量：

```bash
python3 knowledge/gpos-user-manual/eval/run_live_core_271.py --min 100
```

| 检查 | 门槛 |
|------|------|
| 跑完 | 100% |
| 通过率 | ≥90% |
| 语种链接 `url_lang_ok` | ≥95% |
| 错语种链接 | 尽量 0 |
| 产品题误调 SQLBot | 0（抽检） |

产出：`eval/results.jsonl`、`summary.json`、`failures.md`、`LIVE_REPORT.md`。

---

## 2. 闲聊对照（`eval/chitchat.jsonl`）

- 仅 `self-introduction`（或等价固定介绍）
- 无 SQLBot；不要求手册 URL

---

## 3. 经营分析对照（`eval/analysis.jsonl`）

- 仍进分析 skills / SQLBot
- 不用手册文章当经营数据答案
- 续聊 `sessionId` + `biz_advice_report` 可用

---

## 4. 边界串路

| 问法 | 期望 |
|------|------|
| 怎么加商品 | product-manual-qa + en 链接 |
| เพิ่มสมาชิก… | product-manual-qa + **th** 链接 |
| 昨天销售额 | analysis |
| 你好 | self-introduction |

---

## 5. 协议

外部 `SolveRequest` 字段不变；灰度仅允许换 `projId`。
