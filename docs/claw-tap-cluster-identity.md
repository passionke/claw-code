# clawTap 集群身份契约

Author: kejiqing

## 集群 ID（部署）

- 仓库根 `.env` 设置 **`CLAW_CLUSTER_ID`**（如 `prod-claw-01`），网关进程启动时读取。
- **无** Admin / HTTP 修改接口；Admin 只读展示当前值。
- 与 PostgreSQL 连接串无关；换库 URL 不改变 clusterId，除非运维改 `.env` 并重启网关。

## 一致性校验（内部）

Gateway、claude-tap 须连**同一 PG**。tap `GET /healthz` 回报 `clusterId`、`dbHost`、`clusterHash`；gateway 用本机 `CLAW_CLUSTER_ID` + `CLAW_GATEWAY_DATABASE_URL` 计算本地指纹并比对。

```json
{
  "ok": true,
  "clusterId": "prod-claw-01",
  "dbHost": "10.0.0.5",
  "clusterHash": "sha256:…"
}
```

### clusterHash 算法

**只认三件事**：**clusterId**（`.env` 的 `CLAW_CLUSTER_ID`）+ **库身份**（PG URL 里的 `scheme`、`user`）+ **库名**（`dbname`）。**不算** host、port。

见 [`cluster_identity.rs`](../rust/crates/http-gateway-rs/src/cluster_identity.rs)：

1. 解析 `CLAW_GATEWAY_DATABASE_URL`（忽略 password）→ 取出 `scheme`、`user`、`dbname`（`dbHost` 仅展示，不进 hash）
2. payload：`{clusterId}|{scheme}|{user}|{dbname}`
3. `clusterHash = "sha256:" + hex(SHA256(payload))`

例：`CLAW_CLUSTER_ID=local-dev` + `postgres://claw_gateway:***@postgres:5432/claw_gateway` 与宿主机 `...@127.0.0.1:5433/claw_gateway` **hash 相同**（同一 user、同一 db）。

| 阶段 | clusterId | clusterHash |
|------|-----------|-------------|
| Admin 保存 clawTap | 与 gateway `CLAW_CLUSTER_ID` 相等 | 与 gateway 本地计算相等 |
| `/readyz` / solve | `strict` only | 不一致则 `cluster_mismatch`，禁止 solve |
| Worker LLM | — | `OPENAI_BASE_URL` = clawTap（pool `Exec -e` 注入，不落盘） |

claude-tap 须配相同 `CLAW_CLUSTER_ID`，并按同一规则从自己的 `CLAW_GATEWAY_DATABASE_URL` 算 hash（在 claude-tap 仓库实现）。
