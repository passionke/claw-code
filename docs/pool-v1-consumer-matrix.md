# Pool v1: worker artifacts ↔ HTTP consumer read matrix

> **Note (2026-07):** Worker runs in **e2b sandbox**; durable consumer state is still **PostgreSQL only**. Historical host pool / tmpfs wording is background.

Author: kejiqing

Pool-style solve writes worker `.claw/*` under the session on NAS; durable HTTP state lands in PostgreSQL (terminal readback + running sync).

**HTTP consumer rule:** read **PostgreSQL only**. No host `ds_X/sessions/.../.claw/*` for tools / progress / timeline APIs.

**Resolver:** `rust/crates/http-gateway-rs/src/pool_consumer_resolve.rs`

## Running `report_progress`（e2b：nas-api sync）

### 现象（旧）

`GET /v1/tasks/{taskId}` 在 **`status=running`** 时长期返回：

- `currentTaskDesc`: 「处理中」
- 无 `progressHistory` / `planTitle` / `todos`（空数组被 JSON 省略）

同一 turn **`succeeded` 后** 一次 poll 才出现完整 progress（worker 内已写 `.claw` 但未进 PG）。

### 根因（迁移后）

| 层 | 事实 |
| --- | --- |
| Worker | `report_progress` / MCP start / multi-agent `progress_sync` 写入 session `.claw/task-progress.json`、`progress-events.ndjson`（NAS 可见） |
| 终态 | Gateway `readback_turn_from_session_home` 经 **nas-api** 读 NAS → `replace_turn_progress_snapshot` |
| 缺口（已修） | 删宿主机 pool-daemon 后，`PoolOps::sync_turn_progress_to_db` 曾为 trait 空实现 → **running 期间不进 PG** |
| 结果 | HTTP 只读 PG → running poll 看不到中间进度 |

### 修法（当前）

1. **`E2bOrchestratedPool::sync_turn_progress_to_db`**
   - `turn_id` → session scope → nas-api 读 `progress-events.ndjson` + `task-progress.json` → `replace_turn_progress_snapshot`。
   - 实现：`session_db_sync::sync_turn_progress_from_session_home`（与终态 readback 共用）。

2. **`GET /v1/tasks` / 内存 progress poller**（`main.rs`）
   - `load_turn_progress_snapshot` → `maybe_sync_running_turn_progress_from_worker`（仅 `status=running`）→ `resolve_turn_progress` 读 PG。
   - 后台 `refresh_task_progress`（默认 400ms）与客户端 GET 共用同一 sync，暂不 debounce。

### 代码

| 角色 | 路径 |
| --- | --- |
| Progress-only NAS→PG | `rust/crates/http-gateway-rs/src/pool/session_db_sync.rs` — `sync_turn_progress_from_session_home` |
| e2b override | `rust/crates/http-gateway-rs/src/pool/e2b_orchestrated_pool.rs` — `sync_turn_progress_to_db` |
| Gateway 触发 | `rust/crates/http-gateway-rs/src/pool_consumer_resolve.rs` — `maybe_sync_running_turn_progress_from_worker` |
| Task poll | `rust/crates/http-gateway-rs/src/main.rs` — `load_turn_progress_snapshot`, `get_task`, `refresh_task_progress` |

### 部署约束

Gateway 需带本修复的镜像；nas-api 须在线（与终态 readback 同依赖）。沙箱**不**持有 PG 凭据。

```bash
# 验收：长任务 running 期间 poll 应出现 progressHistory / todos，而非仅终态
```

## Write path (worker → PG)

| Worker artifact | Readback | PG storage |
| --- | --- | --- |
| `gateway-solve-session.jsonl` | nas-api readback transcript | `cc_messages` (`render_session_jsonl`) |
| `progress-events.ndjson` | running sync + terminal readback | `solve_timing_jsonb.progressEvents` |
| `task-progress.json` | running sync + terminal readback | `solve_timing_jsonb.taskProgress` |
| solve timing / orchestration | terminal readback | `solveTimingEvents`, `orchestrationEvents` |

## Read path (HTTP consumer → PG)

| API | PG source | Module |
| --- | --- | --- |
| `GET .../turns/{turnId}/tools` | `cc_messages` + `progressEvents` (timestamps) | `turn_tools_api.rs` |
| `GET /v1/tasks/{taskId}` — `progressHistory`, `todos`, `currentTaskDesc` | `solve_timing_jsonb` | `pool_consumer_resolve` + `main.rs` |
| `GET .../execution` | same | `main.rs` |
| `GET .../turns/{turnId}/timeline` | `solve_timing_jsonb` | `pool_consumer_resolve` + `turn_timeline_api.rs` |

Worker in-container loop still writes local `.claw/*` for the active solve; that is **not** exposed to HTTP consumers.

## Regression guards

```bash
./tests/http-gateway-pool-consumer-chain.sh
./deploy/stack/lib/check-connectivity.sh   # [3c] timeline ↔ tools / progressHistory
```

## Related docs

- [`docs/http-gateway-container-pool.md`](http-gateway-container-pool.md)
- [`docs/persistence-model.md`](persistence-model.md)
- [`docs/architecture-governance.md`](architecture-governance.md)
