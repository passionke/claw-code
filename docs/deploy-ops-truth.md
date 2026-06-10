# 运维指令与「真值」验收

Author: kejiqing

避免 **pack-deploy / up 显示成功但仍在跑旧代码**（谎报军情）。部署后必须用 **`verify`** 或让 **`pack-deploy`** 自带的验收通过。

## 指令对照（macOS 宿主机 pool）

| 指令 | 做什么 | 是否保证新代码 |
|------|--------|----------------|
| **`gateway.sh pack-deploy`** | `build`（镜像 + 宿主机 pool 二进制）→ `down`+`up` → **`claw-stack-verify`** → `check` | **是**（推荐唯一标准发布） |
| **`gateway.sh up`** | 起栈；**Darwin 每次重编** `claw-sandbox` | 仅 pool 二进制；**Gateway 镜像**若未 build 仍可能旧 |
| **`gateway.sh build`** | 镜像 + stamp；Darwin **强制**编 pool（不再 `\|\| true` 吞失败） | 只构建，不重启 |
| **`gateway.sh check`** | healthz + solve_async 冒烟 | **不**检查 `claw_pool` / `pool_id` |
| **`gateway.sh verify`** | PG 表结构、pool 二进制 strings、daemon.log 注册、心跳 | **必须**用于确认 |

## 以前为什么会「代码是老的」

1. **`up.sh`**：若 `sandbox/target/release/claw-sandbox` 已存在就**不重编** → 长期跑旧 pool。  
   **现：** macOS / `CLAW_POOL_REBUILD_DAEMON=1` 时**强制** `cargo build -p claw-sandbox-server`。
2. **`build.sh`（Darwin）**：`cargo build … \|\| true` 失败也继续 → 镜像新、pool 旧。  
   **现：** 失败即 build 失败。
3. **宿主机 pool 的 DB URL**：`.env` 里 `@postgres:5432` 在宿主机解析失败 → 监听 9943 成功但 **不注册** `claw_pool`。  
   **现：** `claw_pool_daemon_database_url` 改写为 `127.0.0.1:${CLAW_GATEWAY_PG_HOST_PORT:-5433}`；`pool-daemon-up` 无注册则 **exit 1**。
4. **`check-connectivity`**：只证明 Gateway 能 solve，**不**证明 pool 注册与 schema。

## 标准发布（本机）

```bash
./deploy/stack/gateway.sh pack-deploy
# 任一步 VERIFY FAIL → 整命令非 0，不得当成功

./deploy/stack/gateway.sh verify   # 日常复检
```

## verify 检查项（摘要）

1. PG：`claw_pool` 表、`gateway_turns.pool_id` / `worker_name` 列  
2. **Pool 模式**  
   - **macOS / `CLAW_POOL_HOST_DAEMON=1`**：宿主机二进制、`daemon.pid`、`daemon.log`、DB URL 不含 `@postgres:`  
   - **v1 host pool（默认）**：宿主机 **`claw-sandbox`** 监听 **9944**（单进程；strict/relaxed 为 worker profile，非双 daemon）；`gateway.env` 中 `CLAW_SANDBOX_URL` / `CLAW_POOL_HTTP_BASE`；存在 `claw-worker-*-strict-*` / `*-relaxed-*`；`curl http://127.0.0.1:9944/healthz/live-report` 返回 ok；`verify` 对 relaxed 开启时检查 Capacity RPC 含 `relaxed` profile  
3. `pool-registry.env` 存在  
4. `claw_pool` 有本机 `CLAW_POOL_ID` 行且心跳 &lt; 120s  

**勿混用**：`CLAW_POOL_ADVERTISE_HOST=192.168.x.x`（registry 广播 IP）≠ 容器内访问 pool 用的 `host.containers.internal` / `host.docker.internal`（见 `gateway.env`）。

## 构建戳

`deploy/stack/.claw-build-stamp.env`：最后一次 `build.sh` 的 git rev / 时间 / 特性列表（人工对照用）。

## 旧 turn

迁移**不会**给历史 `gateway_turns` 回填 `pool_id`；只有 **verify 通过之后的新 solve** 才可信。

## Live SSE 无静默降级

`running`/`queued` 的 `GET /v1/biz_advice_report?stream=true` **必须** `gateway_turns.pool_id` + `claw_pool` JOIN 成功；否则 **503**（日志 `route=pool_proxy_sse_denied`）。**不会**再悄悄用 `CLAW_POOL_HTTP_BASE`。
