# 预发 252 部署（外连 250 PG + e2b）

Author: kejiqing

**目标：** `192.168.9.252` 只跑 gateway + Admin（CI 镜像）；`192.168.9.250` 承载 PostgreSQL + e2bserver。

**与旧版本地栈的唯一区别：** 原先在 252 宿主机上跑的组件（`claw-pool-daemon`、compose `claude-tap`、OVS/NAS 侧车等）已**内化到 e2b 沙箱**（worker + nas-api / ovs / observe 单例）。起栈命令仍是原来的：

```bash
./deploy/stack/gateway.sh up --release release-vX.Y.Z
```

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
    SBX[e2b sandboxes<br/>worker / nas-api / ovs / observe]
  end
  subgraph host252["192.168.9.252"]
    GW[gateway-rs + Admin<br/>CI 镜像]
  end
  T -->|SDK Template.build| E2B
  E2B --> SBX
  GW --> PG
  GW --> E2B
```

| 曾在本机（252） | 现在 |
|-----------------|------|
| `claw-pool-daemon` + podman worker | e2b `claw-worker` 沙箱 |
| compose `claude-tap` | e2b `claw-observe` 单例 |
| 本机 OVS / NAS API 进程 | e2b `claw-ovs` / `claw-nas-api` 单例 |

Gateway 启动时会自动 `ensure_e2b_singletons_on_startup`（nas-api / ovs / observe）并 reconcile project workers，无需单独 `pool-up` 或 `tap-up`。

---

## 基础设施（250，一次性）

| 组件 | 地址 | 说明 |
|------|------|------|
| PostgreSQL | `192.168.9.250:5432` | `claw_gateway` 库 |
| e2b API | `http://192.168.9.250:3000` | `config.toml` 的 **api_key**（不是 worker_token） |
| e2b envd | `http://192.168.9.250:3002` | sandbox 通道 |

252 `.env` 模板：`deploy/stack/env.pre-252.e2b.example`

---

## 推荐流程

### 1. 开发机 — e2b 模板（首次或 worker 模板变更时）

```bash
cd ~/work/claw-code
cp deploy/stack/env.pre-252.e2b.example .env   # 或已有 .env
# 必填：CLAW_E2B_API_URL、CLAW_E2B_API_KEY

./deploy/e2b/build-selfhosted-templates.sh
# 或仅 worker：./deploy/e2b/build-selfhosted-templates.sh worker --skip-cache
```

等价一条龙（模板 + singleton 注册，**不**起 gateway）：

```bash
./deploy/stack/gateway.sh e2b-pre-bootstrap
```

### 2. 252 — 起 gateway（与生产相同命令）

```bash
cd ~/work/claw-code
cp deploy/stack/env.pre-252.e2b.example .env   # 填 CLAW_E2B_API_KEY、PG URL 等

./deploy/stack/gateway.sh up --release release-v1.6.18
./deploy/stack/gateway.sh verify
./deploy/stack/lib/admin-solve-e2e.sh 1 ping
```

`up --release` 会：拉 CI 镜像 → 起 gateway + Admin → 等待 HTTP ready → bootstrap 默认项目 → gateway 内建 ensure e2b 单例。

可选（幂等）：`./deploy/stack/gateway.sh e2b-singletons-up --reuse`

---

## `.env` 要点（252）

```bash
CLAW_GATEWAY_DATABASE_URL=postgres://claw_gateway:...@192.168.9.250:5432/claw_gateway
CLAW_E2B_API_URL=http://192.168.9.250:3000
CLAW_E2B_API_KEY=e2b_...          # api_key，非 worker_token
# 勿设 GATEWAY_IMAGE=:local；用 up --release 写 stack/.claw-image-release.env
# e2b-only：勿再设 CLAW_INTERACTIVE_BACKEND / CLAW_SOLVE_ISOLATION / CLAUDE_TAP_MODE
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
| `nas-api-up` 503 | 先完成模板 build，再 `e2b-singletons-up` 或重启 gateway |
| `nasConfig.mountPoints[].hostMountRoot is empty` | 250 建 `/data/claw-nas`，`.env` 设 `CLAW_E2B_NAS_HOST_MOUNT`；e2b `config.toml` `[nas].host_mount_root` 同步 |
| observe / clawTap **502**、PG 里是 `192.168.9.252` | **勿手填 252**；`observe-tap-up --reset`；详见 [`e2b-observe-tap-troubleshoot.md`](./e2b-observe-tap-troubleshoot.md) |
| `singleton_id does not exist`（observe-tap-up 写 PG 失败） | PG 已迁 `cluster_id`；确认 `e2b_pg_settings.py` 用 `CLAW_CLUSTER_ID`，再重跑 `observe-tap-up` |
| `/readyz` 503、`clawTapCluster` 非 `strict` | 先修好 observe 单例，再 `curl …/healthz` 验 `8080-sbx_*/healthz` |

**日志：**

- 开发机：SDK `Template.build` 流式输出
- 250：`journalctl -u e2bserver -f` 或 e2b builder 容器日志

---

## 升级 release

```bash
cd ~/work/claw-code
git pull
./deploy/stack/gateway.sh up --release release-vX.Y.Z
```

Worker 模板变更时需先在开发机重跑 `build-selfhosted-templates.sh worker`。
