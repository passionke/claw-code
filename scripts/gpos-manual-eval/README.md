# gpos-manual-eval

Author: kejiqing

GPOS 产品手册意图分流的评测工具。**KB 正文不入库**：默认读本地 `knowledge/gpos-user-manual/`（`.gitignore`），也可通过环境变量指向任意目录。

| 环境变量 | 含义 |
|----------|------|
| `GPOS_MANUAL_KB` | 本地 KB 根（含 `en/` `th/`） |
| `GPOS_MANUAL_EVAL_OUT` | 跑批产物目录（默认 `$GPOS_MANUAL_KB/eval`） |
| `CLAW_ADMIN_TOKEN` | Admin MCP Bearer（live 必填） |
| `CLAW_ADMIN_MCP_URL` | 默认预发 Admin MCP |

```bash
# 先爬取（产出到 knowledge/，不 commit）
python3 scripts/gpos-manual-crawl/crawl_gpos_user_manual.py --lang all

export CLAW_ADMIN_TOKEN=...
python3 scripts/gpos-manual-eval/route_smoke_271.py
python3 scripts/gpos-manual-eval/run_live_core_271.py --min 100
```

运维真源：[`docs/gpos-user-manual-kb-ops.md`](../../docs/gpos-user-manual-kb-ops.md)
