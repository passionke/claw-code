# claw-tap（claude-tap）对接要求

Author: kejiqing

本文档汇总 **claw-code / http-gateway-rs** 本次改造后对 **claude-tap** 的硬性要求，供 claude-tap 仓库实现与联调。算法细节与 gateway 实现见 [`claw-tap-cluster-identity.md`](claw-tap-cluster-identity.md)、[`cluster_identity.rs`](../rust/crates/http-gateway-rs/src/cluster_identity.rs)。

---

## 1. 角色

| 组件 | 职责 |
|------|------|
| **gateway** | 编排 solve；Admin 配置 clawTap 地址；轮询 health；**仅**在一致性 `strict` 时放行 solve |
| **claw-tap** | OpenAI 兼容代理；worker 的 `OPENAI_BASE_URL` **必须**指向 tap；从 **同一 PG** 读生效 LLM 配置 |
| **worker** | 每通 solve 由 gateway 经 pool `Exec -e` 注入 `OPENAI_BASE_URL` / `OPENAI_API_KEY` / `CLAW_DEFAULT_MODEL`（不写落盘 env 文件） |

**禁止**：gateway 在 cluster 不一致或 tap 不可达时，让 worker **直连** PG 里的 upstream（无 `DegradedDirect` / 无 bypass tap）。

---

## 2. 部署配置（与 gateway 对齐）

### 2.1 集群 ID

- 环境变量 **`CLAW_CLUSTER_ID`**：运维在部署 `.env` 中设置（如 `prod-claw-01`、`local-dev`）。
- **不是**从 PostgreSQL URL 推导的字符串；**不是** Admin 可改字段。
- 规则：非空、最长 64、仅 `[A-Za-z0-9_-]`。
- **claw-tap 进程必须配置与 gateway 相同的 `CLAW_CLUSTER_ID`**（同一集群内所有 gateway/tap 副本一致）。

### 2.2 PostgreSQL

- 环境变量 **`CLAW_GATEWAY_DATABASE_URL`**（或与 gateway **完全相同**的 PG 连接串）。
- 用于：
  1. 计算 **clusterHash**（与 gateway 同一算法）；
  2. 读取 gateway 在 PG 中的**当前生效 LLM**（`gateway_global_settings` / active model，具体表结构以 gateway 为准）。

### 2.3 网络

- Admin 在 gateway 登记 tap 的 **`host` + `proxyPort`**（默认代理端口 `8080`）。
- Gateway 探测与轮询：`GET http://{host}:{proxyPort}/healthz`（8s 超时）。
- Worker 侧 `OPENAI_BASE_URL` = `http://{host}:{proxyPort}`（由 gateway 注入，非 tap 自报）。

---

## 3. Health 接口（必须实现）

### 3.1 路径

`GET {tap_base}/healthz`  
`tap_base` = `http://{host}:{proxyPort}`，无尾部 `/`。

### 3.2 HTTP

- 成功：**200**，JSON body。
- 失败或非 200：gateway 视为 tap 不可达 → `cluster_mismatch`，**禁止 solve**。

### 3.3 JSON 字段

支持 **顶层** 或 **`clawTap` / `cluster` 嵌套**（与 gateway 解析一致）：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `ok` | bool | 是 | 须为 `true`，否则 gateway 拒绝 |
| `clusterId` | string | 是 | 等于本机 **`CLAW_CLUSTER_ID`** |
| `dbHost` | string | 是 | 本机 PG URL 解析出的 **host**（展示/诊断） |
| `clusterHash` | string | 是 | 见 §4，须与 gateway 本地计算 **完全一致** |

示例（顶层）：

```json
{
  "ok": true,
  "clusterId": "prod-claw-01",
  "dbHost": "postgres",
  "clusterHash": "sha256:abcdef..."
}
```

缺 `clusterId` 时 gateway 报错：`clawTap health missing clusterId (upgrade claude-tap)`。

---

## 4. clusterHash 算法（必须与 gateway 字节级一致）

**clusterId + 库身份 + 库名**，不含 host/port：

| 项 | 来源 |
|----|------|
| clusterId | `CLAW_CLUSTER_ID` |
| 库身份 | PG URL 的 `scheme`、`user`（`postgres` / `postgresql` 解析规则与 gateway 一致） |
| 库名 db | PG URL 的 `dbname` |

```text
payload = {clusterId}|{scheme}|{user}|{dbname}
clusterHash = "sha256:" + hex(SHA256(payload))
```

`dbHost` 只在 `/healthz` 里展示，**不参与** hash。

**联调验收**：gateway 与 tap 的 `CLAW_CLUSTER_ID` 相同，且连的是同一 logical 库（user + dbname 相同）；宿主机用 `127.0.0.1:5433`、容器用 `postgres:5432` 可 hash 一致。旧版把 host/port 算进 hash 的 tap 需升级。

---

## 5. Gateway 校验逻辑（tap 需满足的语义）

Gateway 本地计算 `local = local_cluster_identity(CLAW_CLUSTER_ID, CLAW_GATEWAY_DATABASE_URL)`，再拉 tap health 得 `tap`，调用 `verify_tap_cluster(local, tap)`：

| 检查 | 失败时 |
|------|--------|
| `tap.clusterId == local.clusterId` | `clusterId mismatch` |
| `tap.clusterHash == local.clusterHash` | `clusterHash mismatch` |

通过后才允许：

- Admin **保存** clawTap（`PUT …/claw-tap`，保存前必先 probe 成功）；
- 运行时轮询进入 **`strict`**；
- **`GET /readyz` → 200**；
- **solve** 注入 worker env。

轮询间隔：gateway 环境变量 `CLAW_TAP_CLUSTER_POLL_INTERVAL_SECS`（默认 30s，0 关闭轮询）。

不一致或 tap 不可达：`clawTapCluster.consistency = cluster_mismatch`，**solve 返回错误**，无降级路径。

---

## 6. LLM / 上游模型（tap 侧）

- **生效模型真源**：与 gateway 相同的 PostgreSQL（Admin「全局推理」写入的 active LLM）。
- Gateway solve 时仍把 Admin 中的 `apiKey`、`model` 经 `Exec -e` 传给 worker；`OPENAI_BASE_URL` **固定为 tap**。
- **期望**：claw-tap 用 PG 中的配置做代理/路由（实现细节在 claude-tap 仓库；本仓库只要求 health 契约 + 同库同 hash）。

Gateway `output_json.llmRoute` 快照字段（供审计，非 tap 接口）：

```json
{
  "mode": "clawTap",
  "clusterId": "<CLAW_CLUSTER_ID>",
  "clusterHash": "sha256:…",
  "clawTapBaseUrl": "http://host:8080",
  "upstreamBaseUrl": "<Admin 配置的 LLM base>",
  "model": "<modelName>"
}
```

---

## 7. claude-tap 实现清单（Checklist）

- [ ] 读取 **`CLAW_CLUSTER_ID`**、**`CLAW_GATEWAY_DATABASE_URL`**（与 gateway 同值）
- [ ] `GET /healthz` 返回 `ok`、`clusterId`、`dbHost`、`clusterHash`（算法 §4）
- [ ] `clusterId` **不得**用 PG URL 拼接代替 `CLAW_CLUSTER_ID`
- [ ] 连接**同一 PG** 读取 gateway 全局 LLM 配置（与 gateway Admin 一致）
- [ ] 提供 OpenAI 兼容代理（worker 只连 tap，不连 upstream 根地址）
- [ ] 与 gateway 联调：probe 成功 → `readyz` 200 → solve 通

---

## 8. 非目标（勿在 tap 侧假设）

- 无「cluster 不一致仍放行 solve、worker 直连 upstream」模式。
- 无通过 gateway HTTP API **修改** `CLAW_CLUSTER_ID`。
- clusterId **不是** `postgres://user@host:port/dbname` 形式（除非运维故意把 `CLAW_CLUSTER_ID` 设成该字符串，**不推荐**）。

---

## 9. 相关 gateway 行为（便于 tap 联调）

| 接口 / 行为 | 说明 |
|-------------|------|
| `POST /v1/gateway/global-settings/claw-tap/probe` | 保存前探测；比对 clusterId + clusterHash |
| `PUT /v1/gateway/global-settings/claw-tap` | 仅 `{ host, proxyPort }`；probe 失败则 400 |
| `GET /healthz` | 含 `clawTapCluster` 状态 |
| `GET /readyz` | `strict` 前 503 |
| `GET /v1/gateway/global-settings` | `clusterId` 只读回显 gateway 的 `CLAW_CLUSTER_ID` |

---

## 10. 参考实现位置（claw-code）

| 内容 | 路径 |
|------|------|
| Hash / health 解析 | `rust/crates/http-gateway-rs/src/cluster_identity.rs` |
| 轮询与 solve 门禁 | `rust/crates/http-gateway-rs/src/claw_tap_cluster_state.rs` |
| Admin probe | `rust/crates/http-gateway-rs/src/gateway_claw_tap_settings.rs` |
| 部署变量示例 | `deploy/stack/env.local.example`、`env.production.example` |
