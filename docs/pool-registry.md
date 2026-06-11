# Pool registry (`claw_pool`)

Author: kejiqing

Each **`claw-sandbox`** (host pool on `:9944`) registers in PostgreSQL; each running turn records which pool and worker ran it.

**Deployment locality (KISS):** solve/cancel **RPC** is **same-host only**. Each machine runs **gateway + pool daemon** together. Gateway dials co-located pool via **`CLAW_POOL_HTTP_BASE`** / `CLAW_POOL_DAEMON_*`. It does **not** look up `pool_id` in DB to pick an RPC target for new solves.

**Multi-machine:** share one PostgreSQL; each host runs its own `(gateway + pool)` pair. **`claw_pool`** lists all registered pools; **`GET /v1/pools`** and Admin **Pool 集群** expose the registry. Playground chat turn cards show **`poolId`** / **`workerName`** per turn.

**Cross-host** use of `claw_pool` is **observability**, **live SSE HTTP proxy** (read path), and **turn metadata in API/UI** — not remote exec.

## `claw_pool` table

| Column | Meaning |
|--------|---------|
| `pool_id` | Stable id (`CLAW_POOL_ID` or `pool-{hostname}`; deploy sets via `claw-pool-registry-env.sh`) |
| `registration_time_ms` | First register time |
| `slots_max` / `slots_min` | From `CLAW_*_POOL_SIZE` / `CLAW_*_POOL_MIN_IDLE` |
| `advertise_ip` | Externally reachable host (`CLAW_POOL_ADVERTISE_HOST`) |
| `sse_port` | From `CLAW_POOL_HTTP_BIND` (default 9944) |
| `gateway_base` | Browser-reachable gateway URL (`CLAW_POOL_GATEWAY_BASE` or `http://{advertise_ip}:{GATEWAY_HOST_PORT}`) |
| `last_heartbeat_ms` | Updated every **60s** while daemon runs |
| `advertise_ip` / `gateway_base` | Refreshed on each heartbeat (auto-detect LAN IP when `CLAW_POOL_ADVERTISE_HOST` not pinned in `.env`) |

## `gateway_turns` extensions

| Column | Set when |
|--------|----------|
| `pool_id` | Gateway **enqueue prebind** (`CLAW_POOL_ID` / co-located); confirmed on pool exec |
| `worker_name` | Pool `exec_solve` starts (container name) |

## Worker / task file

On exec, pool passes env: `CLAW_POOL_ID`, `CLAW_SESSION_ID`, `CLAW_TURN_ID`, `CLAW_WORKER_NAME`.

`gateway-solve-task.json` may include `sessionId` (gateway writes); `poolId` / `workerName` are filled when the worker runs.

## Gateway → pool channels

| Channel | Routing | Same-host assumption |
|---------|---------|----------------------|
| **RPC** (acquire / exec / release / cancel) | Fixed env `CLAW_POOL_DAEMON_*` only | **Required** — no DB `pool_id` lookup |
| **Live SSE** (`GET …?stream=true`, running) | **必须** DB join `gateway_turns.pool_id` → `claw_pool` → `http://{advertise_ip}:{sse_port}` | 无 join → Gateway **503**（已禁用 `CLAW_POOL_HTTP_BASE` fallback） |

For `running` / `queued` stream: join `gateway_turns.pool_id` → `claw_pool`, proxy to `http://{advertise_ip}:{sse_port}/v1/biz_advice_report/live`. **禁止**静默 fallback；`pool_id` 未预绑或 `claw_pool` 无行时客户端收到明确错误。

## Admin / API observability

| Surface | Purpose |
|---------|---------|
| **`GET /v1/pools`** | All `claw_pool` rows + `coLocatedPoolId` for this gateway |
| **`DELETE /v1/pools/{poolId}`** | Remove stale registry row (daemon re-registers on next `pool-up`) |
| **Admin → 全局配置 → Pool 集群** | Table view; 30s refresh; delete offline/zombie rows |
| **Playground chat turn card** | Cyan **`pool {poolId}`** tag per turn; tooltip **`workerName`** when set |
| **`GET /v1/sessions/…/turns`**, **`GET /v1/tasks/…`**, **`POST /v1/solve_async`** | JSON fields `poolId`, `workerName` |

Playground **solve and poll should use the same `gatewayBase`** (same host). Cross-gateway status poll has known gaps for running progress/cancel; see [`deploy-ops-truth.md`](deploy-ops-truth.md).

Admin **Pool dropdown** (shared PG): only when **≥2 online** `claw_pool` rows have non-empty **`gateway_base`** (offline / zombie rows stay in **Pool 集群** table only). Each option is **`{poolId} · {gateway host}`** or **`本机 · {poolId}`** for co-located; default = playground **`defaultGatewayBase`**. Pool-daemon registers `gateway_base` at startup; production `gateway.sh up` sets **`CLAW_POOL_GATEWAY_BASE`** / **`PLAYGROUND_PUBLIC_GATEWAY_BASE`** from **`CLAW_POOL_ADVERTISE_HOST`** (per machine — do not copy another host's IP into `.env`).

## Multi-host deploy (shared PG)

Each pool host `.env` (or `pool-daemon.env`):

| Variable | Value |
|----------|--------|
| `CLAW_POOL_DAEMON_DATABASE_URL` or host-rewritten `CLAW_GATEWAY_DATABASE_URL` | Central PostgreSQL (host must **not** use `@postgres:` — see `claw-pool-registry-env.sh`) |
| `CLAW_POOL_ID` | Globally unique (e.g. `pool-prod-02`) |
| `CLAW_POOL_ADVERTISE_HOST` | LAN IP reachable by gateways for live SSE proxy |

**Acceptance on each host** (from repo root):

```bash
./deploy/stack/gateway.sh pack-deploy local
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
./deploy/stack/gateway.sh verify
```

## Env (pool daemon)

- `CLAW_GATEWAY_DATABASE_URL` — required for registry (warn + skip if missing)
- `CLAW_POOL_ADVERTISE_HOST` — routable IP/hostname (auto on `gateway.sh up` if unset; **pinned** when set in `.env` — then restart `pool-up` after IP change)
- `CLAW_POOL_ID` — optional override; default `pool-$(hostname -s)` from deploy script

### Deploy auto-detect (`gateway.sh up`)

1. `up.sh` sources `lib/claw-pool-registry-env.sh` → writes `deploy/stack/.claw-pool-rpc/pool-registry.env`
2. `pool-daemon-up.sh` re-exports same vars into the daemon process (+ `CLAW_GATEWAY_DATABASE_URL`)
3. Detection order: existing env → LAN IPv4 (`en0` / `hostname -I` / `ip route`) → short hostname

RPC stays co-located (`CLAW_POOL_DAEMON_TCP`); `advertise_ip` is for DB/SSE metadata and cross-gateway HTTP proxy.

**运维验收：** 见 [`deploy-ops-truth.md`](deploy-ops-truth.md)；发布必须 `gateway.sh pack-deploy` 或 `gateway.sh verify` 通过。

## 跑的时候怎么论证链路

### 1. PostgreSQL（元数据是否写上）

**升级后仍见 offline 旧行：** 旧 dual-pool 时代的 `pool-{host}` / `…-strict` / `…-relaxed` 行可能长期 offline。统一 pool（`CLAW_POOL_ID`，无 profile 后缀）重新注册后，gateway 会 prune 同 `advertise_ip` 上已 offline 的 legacy 行。若仍残留：`pool-up --restart` 一次，或手动 `DELETE FROM claw_pool WHERE …`。

```sql
SELECT pool_id, advertise_ip, sse_port, last_heartbeat_ms FROM claw_pool ORDER BY last_heartbeat_ms DESC;
SELECT turn_id, status, pool_id, worker_name FROM gateway_turns WHERE turn_id = '<T_...>';
```

- `claw_pool` 有行且 `last_heartbeat_ms` 在涨 → daemon 注册/心跳 OK  
- `gateway_turns.pool_id` / `worker_name` 在 exec 后有值 → turn 已绑到本机 pool  

### 2. 部署留痕

- `deploy/stack/.claw-pool-rpc/pool-registry.env` — 本次 up 探测的 `CLAW_POOL_ID`、`CLAW_POOL_ADVERTISE_HOST`  
- `deploy/stack/.claw-pool-rpc/daemon.log` — pool 启动与 `claw_pool registered`  
- `pool-daemon-up.sh` stderr：`pool_id=… advertise=…`

### 3. 日志（`RUST_LOG=info` 或默认 info）

| `target` / 关键字 | 含义 |
|-------------------|------|
| `claw_gateway_pool` + `claw_pool registered` | pool 写入 DB |
| Gateway `assign_turn_pool_worker` + `pool_id` + `worker_name` | turn 绑 pool + worker（**pool_outside**：gateway 在 acquire 后写 PG） |
| `claw_live_report` + `route=db_snapshot_sse` | Gateway 路径 **A**：终态只读 DB |
| `claw_live_report` + `route=pool_proxy_sse` + `pool_http_source=claw_pool_join` | Gateway 路径 **B**：用 DB 的 ip:port |
| `claw_live_report` + `route=pool_proxy_sse_denied` | 路径 **B** 拒绝：无 `pool_id` 或 `claw_pool` 无行（不再 fallback env） |
| `biz_report_pool_proxy` + `upstream_url` | Gateway 实际请求的 pool HTTP URL |
| `pool_http` + `live_sse_subscribe` | 请求已到 pool 9944（本机直连或经 Gateway 代理） |
| `live_report.ingest` | worker stdout 进 pool Hub |
| `biz_report_sse` + `biz.report.delta` | 客户端已收到 delta（经 Gateway 转发的 SSE） |

Gateway 启动：`live_report.gateway` + `co_located_pool_id`（入队预绑用）。  
`GET /healthz` JSON 里 `liveReport.poolHttpBase` 仅为 env 留档，**不参与** live 路由。

### 4. 三条路径对照

| 路径 | `gateway_turns.status` | 关键日志 |
|------|------------------------|----------|
| **A DB 快照** | `succeeded` | `route=db_snapshot_sse`，无 `upstream_url` |
| **B Live 代理** | `running` / `queued` | `route=pool_proxy_sse`，`pool_http_source`，`upstream_url`，pool 侧 `live_sse_subscribe` |
| **RPC solve** | — | `claw_gateway_solve_pool` acquire/exec；**不**按 DB 选 RPC 地址 |
