# 宿主机 claw-pool-daemon（Admin solve 必读）

Author: kejiqing

Admin `POST /v1/solve_async` 依赖 **gateway（compose）+ 宿主机 pool（9944）** 两条线同时活着。gateway 有 `restart: unless-stopped`；pool **没有** compose 托管，必须单独起、且 **macOS 上必须用 launchd**。

---

## 1. 架构（一条线）

| 组件 | 形态 | 检查 |
|------|------|------|
| PostgreSQL | `claw-gateway-postgres` | `podman ps` |
| Gateway | `claw-gateway-rs` | `${GATEWAY_HOST_PORT}/healthz` |
| **Pool** | **宿主机 `claw-pool-daemon`** | `127.0.0.1:9944/healthz/live-report` |
| Playground | `claw-gateway-playground` | Admin UI |
| Worker | `claw-worker-*` | pool 借出，非常驻 |

Gateway → pool：`CLAW_POOL_HTTP_BASE=http://host.containers.internal:9944`（见 `deploy/stack/.claw-pool-rpc/gateway.env`）。

起栈：

```bash
./deploy/stack/gateway.sh up    # compose + pool-up（macOS 走 launchd）
```

---

## 2. macOS：为什么必须 launchd

**已确认根因（2026-06-05）**：用 `nohup` / `disown` 从 **Cursor agent shell** 或短生命周期终端拉起的 pool，进程仍挂在该 shell 的进程树里。**shell 结束会被 SIGKILL**，`daemon.log` **没有** `shutting down`，`pool-daemon-down` **也没有** audit 记录。

表现：

- 同一 shell 里连续 solve 可能全过；
- **新开一条 shell** 或 agent 下一条命令 → 503，`connection refused :9944`；
- gateway 仍 healthy（compose 与 pool 生命周期无关）。

**修法（已落地）**：Darwin 上 `pool-daemon-up.sh` 用 **launchd**（`lib/pool-daemon-launchd.sh`），plist 写入当前 `PATH`（否则 launchd 环境找不到 `/opt/homebrew/bin/podman`）。

```bash
# 确认 launchd 持有 pool
launchctl print "gui/$(id -u)/com.claw.pool-daemon" | head -12
lsof -nP -iTCP:9944 -sTCP:LISTEN
```

Linux 仍用 `nohup`（无 Cursor agent 进程树问题时可接受；生产建议 systemd，与本文 launchd 对称）。

---

## 3. 命令

| 命令 | 作用 |
|------|------|
| `gateway.sh up` | compose + `pool-up`（推荐） |
| `gateway.sh pool-up` | 仅 pool；HTTP 已 up 则 skip |
| `gateway.sh pool-up --restart` | down + 再起 |
| `gateway.sh pool-reset` | down + 删全部 worker |
| `gateway.sh down` | **停 gateway + 停 pool**（必须杀 pool，勿改语义） |

Plist：`deploy/stack/.claw-pool-rpc/com.claw.pool-daemon.plist`（生成物，勿手改；由 `pool-up` 重写）。

---

## 4. 验收（唯一标准）

**禁止**把「起 pool 进程、数 N 秒看会不会死」当排查或验收——那是碰运气，不是假设验证。

**必须**：

```bash
# 1. gateway 能打到 pool
podman exec claw-gateway-rs curl -fsS http://host.containers.internal:9944/healthz/live-report

# 2. Admin 同等路径：连续多轮 solve
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
./deploy/stack/lib/admin-solve-e2e.sh 1 ping   # 第二轮仍须 succeeded

# 3. 换一个新终端 / 新 agent 命令再跑一轮（验证 launchd 跨 session）
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
```

仅 `curl healthz` **不能**代替上述验收。

---

## 5. Admin 503 排查（失败瞬间 30 秒内）

```bash
date -u
podman ps --format '{{.Names}} {{.Status}}' | rg claw-
lsof -nP -iTCP:9944 -sTCP:LISTEN
curl -sS -m 3 http://127.0.0.1:9944/healthz/live-report || echo host-pool-FAIL
curl -sS -m 3 http://127.0.0.1:${GATEWAY_HOST_PORT:-18088}/healthz || echo gateway-FAIL
podman exec claw-gateway-rs curl -sS -m 3 http://host.containers.internal:9944/healthz/live-report 2>&1 || echo gw-to-pool-FAIL
tail -30 deploy/stack/.claw-pool-rpc/daemon.log
tail -10 deploy/stack/.claw-pool-rpc/daemon-down.audit.log 2>/dev/null || echo no-down-audit
launchctl print "gui/$(id -u)/com.claw.pool-daemon" 2>&1 | head -12 || echo no-launchd
```

| 现象 | 含义 | 处理 |
|------|------|------|
| host-pool-FAIL，gateway OK | pool 不在 | `gateway.sh pool-up` 或 `up` |
| host OK，gw-to-pool-FAIL | 容器到宿主机网络 | 查 `host.containers.internal` / gvproxy |
| `shutting down reason=SIGTERM` | 显式 down 或 `--restart` | 查 `daemon-down.audit.log` 调用栈 |
| 无 shutdown、9944 突然 down | 曾用 nohup 被 shell 杀 / SIGKILL | macOS 改 launchd 后 `pool-up --restart` |
| `spawn podman: No such file` | launchd PATH 缺 homebrew | 重新 `pool-up`（会重写 plist PATH） |

现场流水模板：`deploy/stack/.claw-pool-rpc/INVESTIGATE.md`。

---

## 6. Agent / 维护者：禁止的低级错误

1. **不要**用「存活 N 秒」代替 solve 多轮验收。
2. **不要**在 macOS 改回纯 `nohup` 起 pool（agent 环境必复现 503）。
3. **不要**把 pool 崩归咎于用户「没跑命令」——先查 pool 是否 launchd、是否被 agent shell 杀掉。
4. **不要**只验 `healthz` 就宣称 Admin resolve 通过。
5. **不要**改 `down.sh` 为不杀 pool（用户要求 down 必须停 pool）。
6. **不要**在 e2e 里静默 `pool-up` 掩盖 503；应 fail loud 并指向本文。

Rust pool 逻辑变更后仍须：`cargo test` + `pack-deploy local` + **本节 §4 验收**。

---

## 7. 相关文件

| 路径 | 说明 |
|------|------|
| `lib/pool-daemon-up.sh` | 起 pool（Darwin → launchd） |
| `lib/pool-daemon-down.sh` | 停 pool + audit 日志 |
| `lib/pool-daemon-launchd.sh` | plist 生成 / bootstrap / bootout |
| `lib/admin-solve-e2e.sh` | Admin 同等 solve 冒烟 |
| `.claw-pool-rpc/daemon.log` | pool 日志 |
| `.claw-pool-rpc/daemon-down.audit.log` | 谁调了 down |
| `.claw-pool-rpc/INVESTIGATE.md` | 假设登记 + 流水 |

设计细节：`docs/http-gateway-container-pool.md`、`docs/pool-registry.md`。
