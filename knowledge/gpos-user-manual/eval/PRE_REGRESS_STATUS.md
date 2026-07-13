# Pre-regress status (feat/product-manual-kb-intent)

Author: kejiqing

## Live route smoke (2026-07-13) — PASS 4/4

| Case | Expect | Result | Evidence |
|------|--------|--------|----------|
| How do I add a product in Back Office? | product_manual | pass | URL + steps; session `f146d527-…` |
| 后台怎么连接厨房打印机？ | product_manual | pass | 官方手册链接; session `51938234-…` |
| Tell me a joke | chitchat | pass | self-introduction; session `73eff6f7-…` |
| What were yesterday's sales? | analysis | pass | 22299 THB report; session `6705dda7-…` |

Artifacts: `eval/route-smoke-results.jsonl`, `eval/route-smoke-summary.json`, runner `eval/route_smoke_271.py`.

## Applied on pre 271

| Step | Status |
|------|--------|
| commit draft | `savedContentRev=2026-07-13_05-33-50` |
| activate | `activeContentRev=2026-07-13_05-33-50` materialized |
| KB sync | rsync → NAS `/data/claw-nas/pre-claw-01/proj_271/home/.claw/project-home-versions/2026-07-13_05-33-50/home/kb` |
| skills on NAS | `.claw/skills/product-manual-qa` present |

## Offline 100Q

`verify_offline.py` still 100% path/url/must.

## Remaining (full acceptance C)

Full live batch of all 100 `core-questions.jsonl` via solve — not required for route smoke; run when ready with same `extraSession`.
