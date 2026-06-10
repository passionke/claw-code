# Live BOSS 报告契约（stdout-v1-pool-sse）

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

**同机部署：** solve/cancel 的 **Pool RPC** 只打本机 `CLAW_POOL_DAEMON_TCP`（不按 DB `pool_id` 选池）。多机 = 多组 gateway+pool；详见 `docs/pool-registry.md`。

**Live 路由（路径 B）：** Gateway 在 `/v1/solve` / `/v1/solve_async` **入队建 turn** 时即用本机 `CLAW_POOL_ID`（`pool_registry::resolve_pool_id`）预写 `gateway_turns.pool_id`，使排队阶段 `GET /v1/biz_advice_report?stream=true` 可走 `claw_pool` JOIN（`pool_http_source=claw_pool_join`），不必等 worker `exec_solve_start`。`worker_name` 仍在 pool 开 exec 时写入。

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

## 4. 部署与环境

| 变量 | 说明 |
|------|------|
| `CLAW_SANDBOX_URL` | Gateway → pool HTTP RPC（`POST /v1/sandbox/rpc`） |
| `CLAW_POOL_HTTP_BASE` | 与 `CLAW_SANDBOX_URL` 同义后备；live JOIN 失败返回 **503** |
| `CLAW_POOL_ID` | Gateway 与 `claw-sandbox` 须一致；入队时预绑 `gateway_turns.pool_id` |
| `CLAW_POOL_HTTP_BIND` | `claw-sandbox` 监听（默认 `0.0.0.0:9944`） |

**必须**运行宿主机 **`claw-sandbox`**。Gateway 持本地 LiveReportHub（exec 流中继）。

改 Rust 后：`./deploy/stack/gateway.sh pack-deploy`，并重启 pool daemon。

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
| 2026-05-23 | Gateway 入队预写 `pool_id`（`CLAW_POOL_ID`），排队期 live SSE 可走 `claw_pool_join` |
| 2026-05-23 | 禁用 `CLAW_POOL_HTTP_BASE` fallback；无 JOIN → 503 + `pool_proxy_sse_denied` |
