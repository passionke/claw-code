# 预发 252 完整部署流程（外连 250 PG + e2b）

Author: kejiqing

**目标：** `192.168.9.252` 只跑 gateway（CI 镜像）；`192.168.9.250` 承载 PostgreSQL + e2bserver。

**分仓：**

| 仓库 | 机器 | 职责 |
|------|------|------|
| `claw-code` | 开发机 | e2b 模板构建（`deploy/e2b/` 全量） |
| `claw-deploy` | 252 | gateway 启动、singleton 注册、verify |

---

## 架构一览

```mermaid
flowchart LR
  subgraph dev["开发机 claw-code"]
    T[build-selfhosted-templates.sh]
  end
  subgraph host250["192.168.9.250"]
    PG[(PostgreSQL :5432)]
    E2B[e2bserver :3000 / :3002]
  end
  subgraph host252["192.168.9.252 claw-deploy"]
    GW[gateway-rs CI 镜像]
    S[e2b-singletons-up]
  end
  T -->|SDK Template.build| E2B
  S -->|nas-api/ovs/observe| E2B
  S -->|gateway_global_settings| PG
  GW --> PG
  GW --> E2B
```

---

## 基础设施（250，一次性）

| 组件 | 地址 | 说明 |
|------|------|------|
| PostgreSQL | `192.168.9.250:5432` | `claw_gateway` 库，密码以 `ALTER USER` 为准 |
| e2b API | `http://192.168.9.250:3000` | `config.toml` 的 **api_key**（不是 worker_token） |
| e2b envd | `http://192.168.9.250:3002` | sandbox 通道 |

252 `.env` 模板：`claw-deploy/env/pre-252.env.example`

---

## 串联命令（推荐）

### 一条命令（按仓库自动分 phase）

```bash
# === 开发机 claw-code：模板 + singleton（可不加 --release）===
cd ~/work/claw-code
cp .env.example .env   # 或已有 .env
# 必填：CLAW_E2B_API_URL、CLAW_E2B_API_KEY、CLAW_GATEWAY_DATABASE_URL
# 国内 build：CLAW_E2B_CN=1（注释请单独一行，勿写在 = 后面）

./deploy/stack/gateway.sh pre-252-e2b-up --skip-gateway --skip-cache

# === 252 claw-deploy：singleton + gateway + verify ===
cd ~/work/claw-deploy
cp env/pre-252.env.example .env   # 填 CLAW_E2B_API_KEY 等

./deploy/stack/gateway.sh pre-252-e2b-up --release release-v1.6.18
```

`pre-252-e2b-up` 阶段：

1. **preflight** — PG 连通、e2b `/health`、Claw 四类模板是否已注册
2. **templates** — 仅 `claw-code` 执行 `build-selfhosted-templates.sh`
3. **singletons** — `nas-api-up` + `ovs-up` + `observe-tap-up` → 写入 PG
4. **gateway** — `up --release <tag>`（需 `--release`）
5. **verify** — `claw-stack-verify.sh`

### 分步（等价）

```bash
# A. 开发机 — 模板
cd ~/work/claw-code
./deploy/e2b/build-selfhosted-templates.sh --skip-cache

# B. 252 — singleton + 网关
cd ~/work/claw-deploy
./deploy/stack/gateway.sh e2b-singletons-up --reuse
./deploy/stack/gateway.sh up --release release-v1.6.18
./deploy/stack/gateway.sh verify
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
```

---

## `.env` 要点（252）

```bash
CLAW_SOLVE_ISOLATION=e2b
CLAW_INTERACTIVE_BACKEND=e2b
CLAUDE_TAP_MODE=off
CLAW_GATEWAY_DATABASE_URL=postgres://claw_gateway:...@192.168.9.250:5432/claw_gateway
CLAW_E2B_API_URL=http://192.168.9.250:3000
CLAW_E2B_API_KEY=e2b_...          # api_key，非 worker_token
# 勿设 GATEWAY_IMAGE=:local；用 up --release 写 stack/.claw-image-release.env
```

开发机构建模板时额外建议：

```bash
CLAW_E2B_CN=1                       # debian 走 docker.1ms.run
CLAW_E2B_TEMPLATE_SKIP_CACHE=1    # 或 build 脚本 --skip-cache
```

---

## 常见问题

| 现象 | 处理 |
|------|------|
| `no schedulable worker for template claw-nas-api` | 250 上缺模板 → 开发机跑 `build-selfhosted-templates.sh` |
| e2b 401 | `.env` 用了 worker_token，改 api_key |
| `CLAW_E2B_CN` 不生效 | `.env` 行内 `#` 注释会被误读；注释单独一行 |
| debian 拉取超时 | `CLAW_E2B_CN=1` 或 250 Docker daemon 配 registry mirror |
| gateway PG 连不上 | 库内密码与 URL 不一致；迁移后用 `ALTER USER` |
| `nas-api-up` 503 | 先完成模板 build，再 `e2b-singletons-up` |
| `nasConfig.mountPoints[].hostMountRoot is empty` | 250 缺 NAS 目录：建 `/data/claw-nas`，`.env` 设 `CLAW_E2B_NAS_HOST_MOUNT=/data/claw-nas`；e2b `config.toml` `[nas].host_mount_root` 同步 |
| observe / clawTap **502**、PG 里是 `192.168.9.252` | **勿手填 252**；`observe-tap-up --reset`；详见 [`e2b-observe-tap-troubleshoot.md`](./e2b-observe-tap-troubleshoot.md) |
| `singleton_id does not exist`（observe-tap-up 写 PG 失败） | PG 已迁 `cluster_id`；确认 `e2b_pg_settings.py` 用 `CLAW_CLUSTER_ID`，再重跑 `observe-tap-up` |
| `/readyz` 503、`clawTapCluster` 非 `strict` | 先修好 observe 单例（上两行），再 `curl …/healthz` 验 `8080-sbx_*/healthz` |

**日志：**

- 开发机：SDK `Template.build` 流式输出
- 250：`journalctl -u e2bserver -f` 或 e2b builder 容器日志

---

## 升级 release

```bash
cd ~/work/claw-deploy
git pull
./deploy/stack/gateway.sh up --release release-vX.Y.Z
./deploy/stack/gateway.sh verify
```

Worker 模板变更时需先在开发机重跑 `build-selfhosted-templates.sh worker`。
