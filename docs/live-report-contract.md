# Live BOSS 报告契约（stdout-v1-pool-sse）

> **2026-06 e2b-only：** 宿主机 `claw-sandbox` 已移除。路径 B 现经 **gateway LiveReportHub + e2b exec stdout**；`CLAW_SANDBOX_URL` / `:9944` **不再使用**。`claw_pool` JOIN 仍用于历史 turn 元数据。

Author: kejiqing

运行中 BOSS 报告增量流的权威说明。实现与排障以本文为准。

---

## 1. 架构（三条路径）

| 路径 | 条件 | 行为 |
|------|------|------|
| **A. DB 快照 SSE** | `GET /v1/biz_advice_report?stream=true` 且 `gateway_turns.status == succeeded` | Gateway 从 DB 读 `report_message` / `output_json.message`，只发 `biz.report.start` + `biz.report.done`（全文在 done，**无 delta**） |
| **B. Live 代理** | 同上且 status 为 `running` / `queued` | Gateway **LiveReportHub** 消费 sandbox `ExecSolve` NDJSON（pool_outside）；legacy 为反向代理 pool HTTP live |
| **C. 润色 / JSON** | `stream=false` 或 `_bak` | 现有 LLM 润色路径（与 live 正交） |

**禁止**再使用：Gateway `TurnStdoutHub`、`POST /v1/internal/turns/{turnId}/stdout-event`、daemon → gateway HTTP 转发。

> **2026-07 multi-gateway：** 同 `CLAW_CLUSTER_ID` 多 gateway 时，running live SSE 在**非 owner** 机器上 **HTTP 反代**到 `gateway_turns.gateway_base`（见 [`multi-gateway-cluster.md`](multi-gateway-cluster.md)）。**禁止**在错机建空 Hub。

**Live 路由（路径 B，e2b）：** Gateway **LiveReportHub** 仅在 **turn owner** 进程 ingest stdout。入队写入 `gateway_turns.gateway_id` / `gateway_base`（`CLAW_GATEWAY_ID` / `CLAW_GATEWAY_BASE`）。`pool_id = e2b-cloud` 为后端类型标记，**不**表达入口。

**Legacy pool-daemon：** 入队预写 `CLAW_POOL_ID` + `claw_pool_join` 代理仅适用于已拆除的宿主机 pool；e2b 新部署勿依赖。

---

## 2. 端到端数据流（路径 B）

```mermaid
sequenceDiagram
  participant W as Worker
  participant P as claw_sandbox
  participant G as Gateway
  participant A as Admin

  W->>P: stdout __CLAW_GATEWAY_STDOUT__ report.delta
  P->>P: FIFO ingest LiveReportHub
  A->>G: GET /v1/biz_advice_report?stream=true
  G->>P: proxy GET /v1/biz_advice_report/live
  P->>A: SSE biz.report.*
  W->>P: solve.done
  P->>P: HubMsg SolveDone; cleanup if no subscribers
```

---

## 3. 角色与代码

| 角色 | 路径 |
|------|------|
| Producer | `gateway-solve-turn/src/gateway_stdout.rs` |
| Pool ingest | `http-gateway-rs/src/pool/live_report_hub.rs`，`pool/docker_pool.rs` `merge_stdout_hooks` |
| Pool SSE | `http-gateway-rs/src/pool/live_report_sse.rs`，`pool/http_server.rs` |
| Gateway DB 快照 | `biz_advice_report.rs` `db_snapshot_report_sse_response` |
| Gateway 代理 | `biz_report_pool_proxy.rs` |
| Admin 客户端 | `web/gateway-admin/src/hooks/useBizReportStream.ts` |

---

## 4. 部署与环境（FC）

| 变量 | 说明 |
|------|------|
| `CLAW_E2B_API_URL` | e2b API |
| `CLAW_CLUSTER_ID` | PG 行级隔离 |
| `gateway_turns.pool_id` | 历史 JOIN；FC 新 turn 可能为空或占位 |

改 Rust 后：`./deploy/stack/gateway.sh pack-deploy`。

---

## 5. Hub 清理

`LiveReportHub::try_remove_turn`：当 `solve.done` 已 ingest 且 `broadcast::receiver_count() == 0` 时删除 turn 状态，避免内存泄漏。

---

## 6. 观测

| 阶段 | 位置 |
|------|------|
| Pool ingest | `daemon.log` → `report.delta.ingest` |
| Gateway 快照 | 无 pool 调用；DB `gateway_turns` |
| Live SSE | pool HTTP 或经 gateway 代理 |

`GET /healthz` → `liveReport.contract == stdout-v1-pool-sse`

---

## 7. 验收

- running + stream：pool 有 ingest；SSE 有 delta；gateway 无 stdout ingest
- succeeded + stream：仅 `start` + `done` 两事件；done 全文 ≈ `GET /v1/tasks` → `result.outputJson.message`

```bash
rg -n 'stdout-event|forward_claw_stdout|turn_stdout_live_sse' rust deploy scripts --glob '!*.md' || true
# 代码中应为 0；文档可保留「已拆除」说明
```

---

## 8. 变更记录

| 日期 | 说明 |
|------|------|
| 2026-05-23 | stdout-v1-pool-sse：hub/SSE 下沉 pool；gateway DB 快照 + 代理；拆除 gateway ingest |
| 2026-07-20 | Multi-gateway：turn owner 反代 live SSE；`gateway_id`/`gateway_base` 入队；错机禁止空 Hub |
| 2026-05-23 | Gateway 入队预写 `pool_id`（`CLAW_POOL_ID`），排队期 live SSE 可走 `claw_pool_join` |
| 2026-05-23 | 禁用 `CLAW_POOL_HTTP_BASE` fallback；无 JOIN → 503 + `pool_proxy_sse_denied` |
