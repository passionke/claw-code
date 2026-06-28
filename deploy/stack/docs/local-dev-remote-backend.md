# 本地开发 · 远程后端（模式 B）

> **DEPRECATED（2026-06）** 远程 pool RPC 路径已移除。请用 **自托管 e2b**：[`docs/local-dev.md`](../../../docs/local-dev.md)、`env.selfhosted-e2b.example`。

Author: kejiqing

## 结论（先看）

**远程沙箱可以用**，适合「本机扛不动 worker / 要连稳定机真实 PG·tap·SQLBot / 多人共用一台 sandbox」。

**但日常写 Rust、solve、调 preflight 不推荐模式 B**：实测 **性能明显差于模式 A**，且 **gateway（Mac）与 pool worker（远端）是两套制品**，只更新一边仍会挂。

| | 模式 A 全本地 | 模式 B 远程后端 |
|--|--------------|----------------|
| **默认推荐** | ✅ 日常开发 | 仅特定场景 |
| **首 turn 额外耗时（经验值）** | 本机 pool acquire，通常 &lt;2s | Mac gateway materialize + **跨网 RPC** 到远端 pool，常见 **~4s+**（不含 LLM） |
| **改 `rust/` 后** | 一次 `gateway.sh build && up` | **Mac + 稳定主机各 build/up 一次**（同 commit / 同 release tag） |
| **排障** | 一套日志 | Mac gateway 日志 + 远端 pool/worker 日志 + 网络 |

**默认请用** [`env.local.example`](../env.local.example)（`gateway.sh quick`）。模式 B 见 [`env.local-remote-backend.example`](../env.local-remote-backend.example)。

## 是什么

| 模式 | 模板 | 本机 | 稳定主机（如 10.22.28.94） |
|------|------|------|---------------------------|
| A 全本地栈 | `env.local.example` | gateway + PG + pool + tap | — |
| **B 远程后端** | `env.local-remote-backend.example` | gateway + playground | PG + pool + tap |

`CLAW_DEPLOY_PROFILE` 仍为 `local`；用 `CLAW_POOL_REMOTE_BASE` 等 env 区分。

## 适用 / 不适用

**适用**

- 笔记本不想跑重 worker / podman 槽位吃满内存
- 必须连 **dev-stable** 上的 PG、tap、SQLBot 等（与 CI `sunmi-ci-01` 隔离）
- 团队共用一台稳定沙箱主机，避免互相抢本机 pool

**不适用（请回模式 A）**

- 日常改 gateway、worker、solve、preflight 逻辑
- 追求低延迟首 turn / 快速 `check` / `solve-e2e` 迭代
- 无法接受「Mac gateway 与远端 worker **版本必须对齐**」的运维成本

## 性能与架构（为何慢）

Solve 链路在模式 B 下被 **刻意拆开**：

1. **Mac gateway**（`http-gateway-rs`）：PG 读 turn、`render_session_jsonl`、**materialize** 到远端 worker 前
2. **跨网** `CLAW_POOL_REMOTE_BASE`：pool acquire、guest write、exec solve、readback
3. **远端 worker**（`gateway-solve-turn`）：容器内实际跑 claw

因此：

- 每次 solve 多一跳 **Mac ↔ 稳定主机** RTT（materialize、进度同步、readback）
- **冷路径**上 pool acquire + 远端 guest 准备，在 LAN 上也曾观测到 **~4s** 量级（随网络与负载波动）
- **只 rebuild 远端 worker、Mac gateway 仍是旧镜像** → materialize / preflight 行为不一致，表现为「worker 更新了照样失败」

这不是「远端 pool 坏了」，而是 **双机双制品** 的固定成本。模式 A 一次 `build` 同时对齐 gateway 与 worker。

## 快速开始

```bash
cp deploy/stack/env.local-remote-backend.example .env
./deploy/stack/gateway.sh pack-deploy local
./deploy/stack/gateway.sh up
curl -fsS http://127.0.0.1:18088/healthz
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
```

## 注册三要素

Solve RPC（`CLAW_POOL_REMOTE_BASE`）与 Live SSE（`claw_pool` 表）是两条链路，须同时对齐：

1. **同一 PostgreSQL** — `CLAW_GATEWAY_DATABASE_URL` 指向稳定主机 `:5433`
2. **同一 `CLAW_POOL_ID`** — 与稳定主机 pool 注册行一致（勿用 hostname 自动生成的 `pool-MacBook`）
3. **远程 pool 在线** — `curl http://<host>:9944/healthz/live-report` 且 PG 里 heartbeat &lt; 120s

## 与模式 A 切换

```bash
cp deploy/stack/env.local.example .env              # 全本地（推荐默认）
cp deploy/stack/env.local-remote-backend.example .env  # 远程后端
./deploy/stack/gateway.sh up
```

切回模式 A 后：本机起 PG + host pool + tap；勿再依赖 `CLAW_POOL_REMOTE_BASE`。若本机 `claw-workspace` 曾 root 属主导致 up 失败，可设 `CLAW_POOL_WORK_ROOT_BIND_SRC` 指向新的可写目录（见模式 A `.env` 注释）。

## 版本对齐（模式 B 必做）

改 `rust/` 后 **两边同 commit**：

```bash
# Mac
./deploy/stack/gateway.sh build && ./deploy/stack/gateway.sh up

# 稳定主机（git pull 同分支，勿 rsync）
./deploy/stack/gateway.sh build && ./deploy/stack/gateway.sh up
# 或 stable-dev：./deploy/stack/gateway.sh stable-dev-up
```

## 排障

| 现象 | 检查 |
|------|------|
| running SSE 503 | `SELECT pool_id FROM claw_pool WHERE pool_id='<CLAW_POOL_ID>';` |
| solve LLM 失败 | `CLAW_TAP_PROXY_URL` 须稳定主机可达（非 127.0.0.1） |
| up 失败 remote registry | Mac 能否连 `10.22.28.94:5433` 与 `:9944` |

模式 A（本机全栈）见下节 clawTap 排障。

稳定主机部署见 [`stable-sandbox-host.md`](stable-sandbox-host.md)。
