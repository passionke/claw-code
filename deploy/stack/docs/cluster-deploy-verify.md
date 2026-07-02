# 集群部署验收（预发 / 生产）

> **2026-06：** 宿主机 pool / systemd `claw-sandbox` 已移除。新部署按 [`docs/architecture-governance.md`](../../../docs/architecture-governance.md) 验收 e2b + 外连 PG。下文部分检查项为历史集群 pool 口径。

Author: kejiqing

**问题根因：** GitLab CI（10.22.28.94）是 **单机 + nohup**，不覆盖：

- 多机共享 PostgreSQL、`claw_pool` 注册表
- Linux **systemd** `claw-sandbox.service`
- 升级后 **legacy poolId 僵尸行**、Admin Pool 下拉、跨机 `GET /v1/pools`

预发若只验 `healthz` 或单机 solve，**集群问题会漏到生产**。

---

## 每台机器升级后（171、172、…）

```bash
git pull
./deploy/stack/gateway.sh up --release release-vX.Y.Z
./deploy/stack/gateway.sh pool-up --restart
./deploy/stack/gateway.sh verify
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
./deploy/stack/lib/admin-solve-e2e.sh 1 ping   # 新 shell 再一轮（launchd/systemd 持久化）
```

---

## 全集群升完后（任意一台能连共享 PG 即可）

```bash
./deploy/stack/gateway.sh cluster-verify
```

检查项：

1. **无 legacy 僵尸行**：同 `gateway_base` 已有 online `*-strict` 时，不得再留无后缀旧 `pool_id`
2. **每个 `gateway_base` 有 online strict**
3. **逐机 HTTP**：`GET /healthz`、`GET /v1/pools`，`coLocatedPoolId` 必须以 `-strict` 结尾

失败时按提示删 Admin → **Pool 集群** offline 行，或 `DELETE /v1/pools/{poolId}`。

---

## CI 与预发分工

| 环境 | 覆盖 |
|------|------|
| GitLab CI | … node B 经 **`claw-gateway-postgres:5432`**（docker 网络）+ host pool **`127.0.0.1:5433`** → **`cluster-verify`** |
| 预发多机 | **每台** `verify` + **一次** `cluster-verify` |
| 生产多机 | 同上；禁止只 curl healthz |

`CLAW_POOL_DAEMON_USE_SYSTEMD=0` 在旧 CI 有意为之；**e2b-only 栈不再使用 pool systemd**。见 `docs/deploy-ops-truth.md`。
