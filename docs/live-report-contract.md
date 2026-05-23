# Live BOSS 报告契约（stdout-v1）

Author: kejiqing

本文是 **运行中 BOSS 报告增量流** 的唯一权威说明：架构、顺序保证、已踩过的生产级缺陷、观测与验收。实现与排障以本文为准，勿再文档化已拆除路径。

---

## 1. 为什么这个方案难

Live 报告看起来只是「模型吐字 → 浏览器打字机」，实际要同时满足：

| 约束 | 含义 |
| --- | --- |
| **跨进程** | Worker 在容器内；pool 在宿主机；Gateway 在另一容器；不能共享内存，只能 stdout 行 + HTTP。 |
| **同步回调里不能 await** | `podman exec` 按行回调是同步的；转发若每行 `tokio::spawn`，HTTP 会乱序。 |
| **迟到订阅** | Admin 在 `running` 中途才连 SSE，必须先 **catch-up 已有全文** 再 tail 增量。 |
| **catch-up 与 tail 不能重叠** | subscribe 与 snapshot 若分两次加锁，中间 ingest 的 chunk 会 **双发** 或 **漏发**。 |
| **结束边界** | `solve.done` 与最后几条 `report.delta` 几乎同时到达；错误退出会 **截断尾段**（用户看到「退货率为 0%」后面整段消失）。 |
| **静默失败** | daemon 缺 `CLAW_GATEWAY_INTERNAL_*` 时曾 **无日志丢弃** 全部 live（已加 `live_report_audit`）。 |

因此：**没有中间件（Redis/PG 分片）也照样难**——难点在顺序、边界与多层 hook，不在存储选型。

---

## 2. 禁止路径（已拆除）

**禁止**再实现或文档化：

- Worker `:18765` 侧车 SSE
- `POST …/assistant-stream`、`POST …/report-stream`
- PostgreSQL `gateway_turn_live_chunks`、`worker_report_host/port`

当前 **唯一** live 路径：**stdout 行 → hub → SSE**（stdout-v1）。

---

## 3. 角色共识（谁产、谁传、谁消费）

| 角色 | 职责 | 代码入口 |
| --- | --- | --- |
| **Producer** | `claw gateway-solve-once`：模型 `TextDelta` → `emit_report_delta` → stdout | `gateway-solve-turn/src/gateway_stdout.rs`，`lib.rs` `DirectApiClient::stream` |
| **Relay（exec）** | `podman exec` 按行读 stdout；解析 `__CLAW_GATEWAY_STDOUT__` 前缀行 | `pool/docker_cli.rs`，`pool/docker_pool.rs` `merge_stdout_hooks` |
| **Relay（daemon HTTP）** | 宿主机 `claw-pool-daemon`：`hub` 为 `None` 时 POST 到网关 | `turn_stdout_hub.rs` `forward_claw_stdout_line`，`live_report_audit.rs` |
| **Ingest** | `POST /v1/internal/turns/{turnId}/stdout-event` → `TurnStdoutHub::ingest_json` | `main.rs` `post_turn_stdout_event` |
| **Consumer** | Admin：`GET /v1/biz_advice_report?stream=true` → SSE `biz.report.*` | `turn_stdout_live_sse.rs`，`web/gateway-admin/.../useBizReportStream.ts` |

---

## 4. 端到端数据流（compose 默认：远程 daemon）

```mermaid
sequenceDiagram
  participant W as Worker (gateway-solve-once)
  participant D as claw-pool-daemon (host)
  participant G as Gateway (TurnStdoutHub)
  participant A as Admin (EventSource)

  W->>W: TextDelta → emit_report_delta
  W->>D: stdout line __CLAW_GATEWAY_STDOUT__ report.delta
  Note over D: merge_stdout_hooks: mpsc FIFO, 单消费者 await forward
  D->>G: POST /v1/internal/turns/{turnId}/stdout-event
  G->>G: state.text += chunk; broadcast HubMsg::Delta
  A->>G: GET /v1/biz_advice_report?stream=true
  G->>A: catch-up chunks (snapshot, 48 chars/段)
  loop until HubMsg::SolveDone
    G->>A: biz.report.delta
  end
  W->>D: stdout solve.done
  D->>G: POST stdout-event ev=solve.done
  G->>G: broadcast HubMsg::SolveDone
  G->>A: biz.report.done (report_text = hub 全文 snapshot)
```

**结构化 stdout 行格式：**

```text
__CLAW_GATEWAY_STDOUT__{"ev":"report.delta","text":"…"}
__CLAW_GATEWAY_STDOUT__{"ev":"solve.done","clawExitCode":0,"outputText":"…","outputJson":{…}}
```

定义见 `rust/crates/gateway-solve-turn/src/gateway_stdout.rs`。

---

## 5. 部署模式（二选一）

| 模式 | 条件 | stdout → hub |
| --- | --- | --- |
| **A. 同进程** | 网关进程内 pool，`stdout_hub: Some` | `handle_claw_stdout_line` **直接** `ingest_json`，无 HTTP |
| **B. 远程 daemon（本地 compose 默认）** | `poolRpcRemote: true`；`deploy/stack/.claw-pool-rpc/` | daemon `hub: None` → **必须** `CLAW_GATEWAY_INTERNAL_BASE_URL` + `CLAW_GATEWAY_INTERNAL_TOKEN` |

模式 B 注意：

- daemon 在 **宿主机**，不能用容器 DNS `http://claw-gateway-rs:8080`。
- 应使用 `http://127.0.0.1:${GATEWAY_HOST_PORT}`（见 `deploy/stack/lib/pool-daemon-up.sh`）。
- 启动日志应有：`live_report.forward_env_ok`（`deploy/stack/.claw-pool-rpc/daemon.log`）。

`GET /healthz` → `liveReport` 字段描述当前契约（`contract: stdout-v1`）。

---

## 6. TurnStdoutHub 与 Live SSE 语义

### 6.1 Hub 状态（每 `turnId` 一行）

- `text`：已 ingest 的 delta **全文拼接**（late connect 的 snapshot 来源）。
- `tx`：`broadcast::Sender<HubMsg>`，容量 `256`（慢消费者可能 `Lagged`，SSE 侧 `continue`）。
- `solve_done`：ingest 过 `solve.done` 后为 true（**不再**单独给 SSE 轮询退出用，见 §7.4）。

### 6.2 Ingest 事件

| `ev` | Hub 行为 |
| --- | --- |
| `report.delta` | `text.push_str`；`tx.send(HubMsg::Delta)`；打 `report.delta.ingest` |
| `solve.done` | `solve_done = true`；`tx.send(HubMsg::SolveDone)` **（有序结束哨兵）** |

### 6.3 Live SSE 连接时（`turn_stdout_live_sse.rs`）

1. **`subscribe_with_snapshot`**：同一把锁内 `subscribe` + `clone(text)`，避免 catch-up 与 broadcast **重叠/漏字**（§7.1）。
2. **Catch-up**：`split_catchup_chunks(snapshot, 48)` → 多条 `biz.report.delta`。
3. **Tail**：`recv` 直到 `HubMsg::SolveDone`（§7.4）。
4. **Done**：`biz.report.done`，`report_text` / `report_json.message` 来自 **hub 最终 `snapshot_text`**（与 delta 拼接结果一致）。

### 6.4 `hasReport` / Admin 开门

- `GET /v1/tasks` → `hasReport`：`status` 为 `running` 或 `succeeded` 即为 `true`（**不表示**已有 delta；无 delta 时 SSE 空转）。
- `reportTime`：hub 首条 delta 时间，否则 `startedAtMs` / `finishedAtMs`。

---

## 7. 2026-05-23 已修复缺陷（必读）

以下四条均在真实任务「哪个菜卖得好」+ `store_id=S20241007172800004204` 上复现并验证；修前会出现 **逐字重复、乱序、尾段截断**。

### 7.1 Catch-up 与 broadcast 竞态（双发 / 字符交错）

**现象：** SSE 文本像 `"好的好的，，数据我已经了解到数据我已经了解到"`。

**根因：** 先 `subscribe()` 再 `snapshot_text()` 两次加锁；中间 ingest 的 chunk 既进 snapshot 又进 broadcast，catch-up 与 tail 各发一遍。

**修复：** `TurnStdoutHub::subscribe_with_snapshot` — 单次锁内 subscribe + clone `text`。

- `rust/crates/http-gateway-rs/src/turn_stdout_hub.rs`
- `rust/crates/http-gateway-rs/src/turn_stdout_live_sse.rs`（连接时只调此方法）

### 7.2 Daemon 路径 stdout hook **双层叠加**（每 chunk 发两次）

**现象：** `live_report.forward_ok` 条数 ≈ ingest 的 **2 倍**；SSE 相邻 delta 完全相同（`"好的"` / `"好的"`）。

**根因：**

1. `pool/rpc.rs` `dispatch_pool_rpc` 对 `Exec` 预包一层 `merge_stdout_hooks`；
2. `docker_pool.rs` `exec_solve` 再包一层，内层对 **同一行** 调 `outer(line)` + `handle_claw_stdout_line(line)` → **两次 HTTP POST**。

**修复：** daemon RPC `Exec` 向 `exec_solve` 传 `on_stdout_line: None`，只保留 `exec_solve` 内部 **一层** `merge_stdout_hooks`。

- `rust/crates/http-gateway-rs/src/pool/rpc.rs`（注释说明禁止预 wrap）

### 7.3 每行 stdout `tokio::spawn` 并发转发（乱序）

**现象：** 中文语序错乱（如「数据。现在」顺序颠倒）；表格 markdown 被打散。

**根因：** `merge_stdout_hooks` 每行 `tokio::spawn(async { forward... })`，多条 HTTP 竞态到达网关，hub ingest 顺序 ≠ worker stdout 顺序。

**修复：** 每 turn 一个 `mpsc::unbounded_channel` + **单消费者** 顺序 `await` `handle_claw_stdout_line`；同步回调只 `send(line)`。

- `rust/crates/http-gateway-rs/src/pool/docker_pool.rs` `merge_stdout_hooks`

### 7.4 SSE 在 `solve_done` 状态位上提前 break（**尾段丢失**）

**现象：** Admin 在「📌 整体无退货 — 所有 Top10 菜品的净销量」处截断；claude-tap 有完整结尾「与销量一致，退货率为 0%…欢迎告诉我」。curl 直连网关有时「看起来完整」——慢路径（playground `__proxy_sse__`）更易触发。

**根因：** SSE worker 每消费一条 delta 后查 `hub.solve_done()`；`solve.done` ingest 只翻标志位、**不**走 broadcast。时序上常出现：已 `recv` 一条 delta → 查标志为 true → `break` → broadcast 队列里 **未消费的尾 delta** 永久丢失。

**修复：**

- ingest `solve.done` 时 `tx.send(HubMsg::SolveDone)`；
- SSE 循环只等 `HubMsg::SolveDone`，**不再**在循环内轮询 `solve_done()`。

- `rust/crates/http-gateway-rs/src/turn_stdout_hub.rs`（`HubMsg` enum）
- `rust/crates/http-gateway-rs/src/turn_stdout_live_sse.rs`

### 7.5 修复后验收指标（同一 turn）

| 指标 | 期望 |
| --- | --- |
| daemon `forward_ok` | ≈ gateway `report.delta.ingest`（差 1 以内可为 solve.done 行） |
| SSE `biz.report.delta` 条数 | ≈ ingest 条数 |
| 相邻完全重复 delta | **0**（标点/空格相邻相同不计为 bug） |
| SSE 拼接尾段 vs `result.outputJson.message` 尾段 | **一致**（见 §8） |

---

## 8. Live SSE vs 任务落盘（勿读错字段）

| 数据源 | 路径 | 用途 |
| --- | --- | --- |
| **运行中打字机** | `GET /v1/biz_advice_report?stream=true` → 拼接所有 `biz.report.delta` | Admin 实时 UI |
| **任务结束后正文** | `GET /v1/tasks/{taskId}` → **`result.outputJson.message`** | 轮询、刷新页、与 claude-tap 对照 |
| **SSE 结束快照** | 同连接上的 `biz.report.done` → `report_text` / `report_json.message` | 应与 hub 全文一致 |

**常见误判：** 直接读顶层 `outputJson.message`（为空）——正文在 **`result`** 里。

Worker `solve.done` 的 `outputJson.message` 来自 **会话 assistant 文本块拼接**（`run_gateway_solve_turn`），与 stdout 流式 delta **同源不同路径**；正常时应与 hub 全文大致一致（长度可能差少量空白）。

---

## 9. Admin 前端（gateway-admin）

- Hook：`web/gateway-admin/src/hooks/useBizReportStream.ts`
- SSE URL：`/__proxy_sse__?target=.../v1/biz_advice_report?...&stream=true`（playground 代理，端口见 `.env` `GATEWAY_PLAYGROUND_HOST_PORT`，默认 **18675**）
- Delta 经 `requestAnimationFrame` 批量 `setText`；`biz.report.done` 时 `flushPending` 后关闭 EventSource
- 调试：`window.__bizReportObsByTurn`、`[biz-report-stream]` 控制台日志

前端 **不会** 主动截断正文；尾段丢失若仅在 Admin 出现、curl 直连完整，优先查 §7.4 是否已部署新网关镜像。

---

## 10. 可观测性（禁止静默失败）

| 阶段 | 位置 | 成功 | 失败 |
| --- | --- | --- | --- |
| Worker 产 delta | worker stderr/stdout | 带前缀的 JSON 行 | 无 `__CLAW_GATEWAY_STDOUT__` → 老二进制或未走 gateway-solve-once |
| Daemon 转发 | `deploy/stack/.claw-pool-rpc/daemon.log` | `live_report.forward_ok` + `turn_id` | `live_report.forward_env_missing` / `forward_http_error` / `forward_network_error` |
| Gateway ingest | `podman logs claw-gateway-rs` | `report.delta.ingest` + `turn_id` | 无 ingest → 上游未到 |
| SSE 写出 | gateway 日志 | `report.delta.sse_emit`（若启用） | — |
| 契约自检 | `GET /healthz` | `liveReport.contract == stdout-v1` | — |

---

## 11. 标准部署与验收

### 11.1 部署（改 Rust 后必做）

```bash
cd /path/to/claw-code
./deploy/stack/gateway.sh pack-deploy
# 日志: deploy/stack/.build.log
```

禁止只重启容器不编镜像（「老 worker / 老 gateway」会导致仍走废弃路径或无 `emit_report_delta`）。

### 11.2 提交真实 BOSS 任务

```bash
curl -sS -X POST http://127.0.0.1:18088/v1/solve_async \
  -H 'Content-Type: application/json' \
  -d '{
    "dsId": 1,
    "userPrompt": "哪个菜卖得好",
    "extraSession": {
      "tenant_code": "GPOS",
      "solution_code": "restaurant",
      "biz_type": "BOSS_REPORT",
      "store_id": "S20241007172800004204",
      "org_id": ""
    }
  }'
```

记下 `turnId`、`taskId`（通常与 `sessionId` 相同）。

### 11.3 链路计数（须近似 1:1:1）

```bash
TURN='T_xxxxxxxx'   # 替换

# daemon（模式 B）
rg -c "forward_ok turn_id=${TURN}" deploy/stack/.claw-pool-rpc/daemon.log

# gateway ingest
podman logs claw-gateway-rs 2>&1 | grep 'report.delta.ingest' | grep -c "${TURN}"

# SSE（直连或经 playground 代理）
# 直连: curl -sN "http://127.0.0.1:18088/v1/biz_advice_report?sessionId=...&turnId=${TURN}&dsId=1&stream=true"
# 代理: http://127.0.0.1:18675/__proxy_sse__?target=...
```

### 11.4 尾段完整性

```bash
TASK='...'  # taskId
curl -sS "http://127.0.0.1:18088/v1/tasks/${TASK}" | python3 -c "
import sys, json
d = json.load(sys.stdin)
msg = ((d.get('result') or {}).get('outputJson') or {}).get('message') or '')
print('message chars:', len(msg))
print('TAIL:', repr(msg[-200:]))
"
```

与 Admin 屏幕末段、claude-tap 报告末段对照；应包含完整「简要分析」列表及结尾邀请语（若模型生成）。

---

## 12. 排障决策树

```text
Admin 无增量 / 不增长
├─ hasReport=false → 任务未 running/succeeded；查 /v1/tasks status
├─ 无 report.delta.ingest → 见下「ingest 为零」
└─ 有 ingest 无 SSE → Admin 是否连 stream=true；代理端口是否 18675

ingest 为零
├─ daemon.log 无 forward_ok
│   ├─ live_report.forward_env_missing → 修 pool-daemon-up.sh / .env，重启 daemon
│   └─ forward_http_error → BASE_URL/TOKEN/端口
├─ forward_ok 有、ingest 无 → 网关 internal 路由 / token
└─ 有 ingest 但 UI 重复/乱序/截断 → 查 §7 四缺陷是否已 pack-deploy 新镜像

仅尾段丢失（前半正常）
└─ 几乎必是 §7.4 旧 SSE break；升级网关后重测

claude-tap 完整、Admin 短一截
└─ 同上 + 确认 Admin 走 proxy 时网关已含 HubMsg::SolveDone 修复
```

---

## 13. 相关文件索引

| 主题 | 路径 |
| --- | --- |
| Worker stdout 协议 | `rust/crates/gateway-solve-turn/src/gateway_stdout.rs` |
| Hub + HTTP 转发 | `rust/crates/http-gateway-rs/src/turn_stdout_hub.rs` |
| Live SSE | `rust/crates/http-gateway-rs/src/turn_stdout_live_sse.rs` |
| FIFO stdout hook | `rust/crates/http-gateway-rs/src/pool/docker_pool.rs` |
| Daemon 禁止双 hook | `rust/crates/http-gateway-rs/src/pool/rpc.rs` |
| 转发审计日志 | `rust/crates/http-gateway-rs/src/live_report_audit.rs` |
| Admin SSE 客户端 | `web/gateway-admin/src/hooks/useBizReportStream.ts` |
| 部署入口 | `deploy/stack/gateway.sh`（`pack-deploy`） |
| 持久化（无 live PG） | `docs/persistence-model.md` |
| HTTP API 摘要 | `docs/http-gateway-rs-api.md` |

---

## 14. 变更记录

| 日期 | 说明 |
| --- | --- |
| 2026-05-23 | stdout-v1 契约成文；记录 §7 四条生产缺陷及修复；补充验收与 `result.outputJson.message` 说明 |
