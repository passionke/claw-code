# Pool v1: worker artifacts ↔ HTTP consumer read matrix

Author: kejiqing

Pool v1 runs solve in a worker with **tmpfs** `CLAW_PROJECT_CONFIG_ROOT=/claw_host_root`. Worker `.claw/*` is ephemeral; durable state lands in PostgreSQL on `readback_out`.

**HTTP consumer rule:** read **PostgreSQL only**. No host `ds_X/sessions/.../.claw/*` for tools / progress / timeline APIs.

**Resolver:** `rust/crates/http-gateway-rs/src/pool_consumer_resolve.rs`

## Running `report_progress`（已确认根因 2026-06-06）

### 现象

`GET /v1/tasks/{taskId}` 在 **`status=running`** 时长期返回：

- `currentTaskDesc`: 「处理中」
- 无 `progressHistory` / `planTitle` / `todos`（空数组被 JSON 省略）

同一 turn **`succeeded` 后** 一次 poll 才出现完整 progress（与 worker 内 `report_progress` 实际已写入 tmpfs 矛盾）。

### 根因

| 层 | 事实 |
| --- | --- |
| Worker | `report_progress` 写入槽位 tmpfs：`.claw/task-progress.json`、`progress-events.ndjson` |
| 终态 | **pool_outside**：Gateway `readback` 经 sandbox RPC；legacy 路径为宿主机 pool `readback_out` |
| 错误路径（已删除） | **gateway 容器内**直接 `podman exec` 读 worker → **失败**（gateway 与 worker 不在同一 runtime 命名空间；worker 由宿主机 pool 管理） |
| 结果 | **`running` 期间 PG `solve_timing_jsonb` 为空**；HTTP 消费端只读 PG → poll 看不到中间进度 |

### 修法

1. **Pool RPC `sync_turn_progress`**（`POST /v1/pool/rpc`，op `sync_turn_progress`）  
   - **Gateway**（`PoolRpcClient`）在 `status=running` 的每次 progress 解析前调用。  
   - **Pool daemon（宿主机）**：`gateway_turns.worker_name` → `podman exec` → 读 worker `.claw/progress*` → `replace_turn_progress_snapshot` 写入 PG。

2. **`GET /v1/tasks` running 轮询**（`main.rs`）  
   - `load_turn_progress_snapshot` → 先 RPC sync，再 `resolve_turn_progress` 读 PG。  
   - `queued`/`running`：**每次 poll** 用 PG 刷新 `currentTaskDesc`、`progressHistory`、`planTitle`、`todos`（不再仅在 `currentTaskDesc.is_none()` 时更新）。  
   - 后台 `refresh_task_progress` poller 同步更新内存 `TaskRecord` 的同字段。

### 代码

| 角色 | 路径 |
| --- | --- |
| RPC 定义 / 客户端 | `rust/crates/http-gateway-rs/src/pool/rpc.rs` — `PoolRpcReq::SyncTurnProgress` |
| 宿主机执行 | `rust/crates/http-gateway-rs/src/pool/docker_pool.rs` — `sync_turn_progress_to_db` |
| Gateway 触发 | `rust/crates/http-gateway-rs/src/pool_consumer_resolve.rs` — `maybe_sync_running_turn_progress_from_worker` |
| Task poll | `rust/crates/http-gateway-rs/src/main.rs` — `load_turn_progress_snapshot`, `get_task`, `refresh_task_progress` |

### 部署约束

**Gateway 与 pool daemon 必须同版本升级。** 仅升 gateway、pool daemon 仍为旧版时，RPC `sync_turn_progress` 无法识别，`running` 中间进度仍进不了 PG。

```bash
./deploy/stack/gateway.sh pack-deploy local   # 或目标环境 up --release
# 验收：长任务 running 期间 poll 应出现 progressHistory，而非仅终态
```

## Write path (worker → PG)

| Worker artifact | Readback | PG storage |
| --- | --- | --- |
| `gateway-solve-session.jsonl` | `readback_out` transcript | `cc_messages` (`render_session_jsonl`) |
| `progress-events.ndjson` | `readback_timing_to_db` | `solve_timing_jsonb.progressEvents` |
| `task-progress.json` | `readback_timing_to_db` | `solve_timing_jsonb.taskProgress` |
| solve timing / orchestration | `readback_timing_to_db` | `solveTimingEvents`, `orchestrationEvents` |

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
- [`deploy/stack/docs/host-pool-daemon.md`](../deploy/stack/docs/host-pool-daemon.md)
