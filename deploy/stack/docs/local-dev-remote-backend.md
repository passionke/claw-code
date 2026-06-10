# 本地开发 · 远程后端（模式 B）

Author: kejiqing

## 是什么

| 模式 | 模板 | 本机 | 稳定主机（如 10.22.28.94） |
|------|------|------|---------------------------|
| A 全本地栈 | `env.local.example` | gateway + PG + pool + tap | — |
| **B 远程后端** | `env.local-remote-backend.example` | gateway + playground | PG + pool + tap |

`CLAW_DEPLOY_PROFILE` 仍为 `local`；用 `CLAW_POOL_REMOTE_BASE` 等 env 区分。

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
cp deploy/stack/env.local.example .env              # 全本地
cp deploy/stack/env.local-remote-backend.example .env  # 远程后端
./deploy/stack/gateway.sh up
```

## 排障

| 现象 | 检查 |
|------|------|
| running SSE 503 | `SELECT pool_id FROM claw_pool WHERE pool_id='<CLAW_POOL_ID>';` |
| solve LLM 失败 | `CLAW_TAP_PROXY_URL` 须稳定主机可达（非 127.0.0.1） |
| up 失败 remote registry | Mac 能否连 `10.22.28.94:5433` 与 `:9944` |

稳定主机部署见 [`stable-sandbox-host.md`](stable-sandbox-host.md)。
