# Claw 运维部署 Runbook（本机开发 + 部署运维）

Author: kejiqing

**唯一入口命令：** `./deploy/stack/gateway.sh`（实现脚本在 `deploy/stack/lib/`）。

**Env 模板：** 复制 [`deploy/stack/env.selfhosted-e2b.example`](../deploy/stack/env.selfhosted-e2b.example) → 仓库根 `.env`。

**IP 真值（勿混用）：**

| 地址 | 角色 |
|------|------|
| `10.8.0.1` | PostgreSQL `:5433` + e2bserver API `:3000` / envd `:3002` |
| `10.8.0.11` | NAS NFS export |
| `supone.top` | e2b sandbox traffic 域名（`CLAW_E2B_DOMAIN`） |

**Backend 真值：** `CLAW_INTERACTIVE_BACKEND=e2b`、`CLAW_SOLVE_ISOLATION=e2b`（历史文档中的 `fc` 指同一 e2b 路径）。

---

## 0. 首次部署顺序（推荐）

```bash
# 1. 配置
cp deploy/stack/env.selfhosted-e2b.example .env   # 编辑 CLAW_CLUSTER_ID、PG URL、e2b keys

# 2. e2b 四类模板 → 写 PG templateId（dev 机，需 .venv-fc + e2b API 可达）
./deploy/e2b/build-selfhosted-templates.sh
# 或：./deploy/stack/gateway.sh e2b-pre-bootstrap

# 3. 起 gateway + playground
./deploy/stack/gateway.sh quick

# 4. 确保 e2b 单例（nas-api / ovs / observe）— gateway 启动也会自动 ensure
./deploy/stack/gateway.sh e2b-singletons-up

# 5. 验收
./deploy/stack/gateway.sh verify
./deploy/stack/gateway.sh check
```

预发全链路：`./deploy/stack/gateway.sh pre-252-e2b-up`（preflight → templates → singletons → up --release → verify）。

---

## 1. e2b 组件注册与 tplId 写入 PG

### 1.1 组件与 PG 契约

所有配置在 PostgreSQL 表 `gateway_global_settings`（按 `CLAW_CLUSTER_ID` 分行）的 `settings_json` JSONB：

| 组件 | 构建脚本 | PG 键 | 默认 alias | 生效优先级 |
|------|----------|-------|------------|------------|
| Worker (strict) | `deploy/e2b/build-claw-worker-selfhosted.py` | `e2bWorker.templateId` | `claw-worker` | PG → `CLAW_E2B_TEMPLATE` → alias |
| Worker (relaxed) | `deploy/e2b/build-claw-worker-relaxed-selfhosted.py` | **不写 PG** | `claw-worker-relaxed` | e2b alias；exec mode 由 gateway `worker_profile_json` 选 |
| NAS API | `deploy/e2b/build-claw-nas-api-selfhosted.py` | `e2bNasApi.templateId` | `claw-nas-api` | PG → env → alias |
| OVS | `deploy/e2b/build-claw-ovs-selfhosted.py` | `e2bOvs.templateId` | `claw-ovs` | 同上 |
| Observe | `deploy/e2b/build-claw-observe-selfhosted.py` | `e2bObserve.templateId` | `claw-observe` | 同上 |

单例运行时（非 templateId）：`e2bOvs.baseUrl` / `e2bObserve.baseUrl` / `e2bNasApi.baseUrl` + `clawTap`（observe 代理 URL）。

PG 写入 helper：[`deploy/e2b/e2b_pg_settings.py`](../deploy/e2b/e2b_pg_settings.py) 的 `merge_settings_json_key()`。需 `CLAW_GATEWAY_DATABASE_URL`。

### 1.2 一键构建命令

```bash
# 全部：worker(strict+relaxed alias) + nas-api + ovs + observe
./deploy/e2b/build-selfhosted-templates.sh [--skip-cache]

# 单目标
./deploy/e2b/build-selfhosted-templates.sh --only worker
./deploy/e2b/build-selfhosted-templates.sh --only ovs

# 经 gateway 包装（templates → singletons）
./deploy/stack/gateway.sh e2b-pre-bootstrap [--skip-templates] [--reset]
```

构建成功日志应含 `persisted e2bWorker.templateId`（strict）或 `skip PG e2bWorker`（relaxed）。

### 1.3 日常改 worker 二进制（dev）

```bash
./deploy/stack/gateway.sh e2b-worker-deploy [--skip-compile]
```

唯一手册：[`deploy/e2b/WORKER-BUILD.md`](../deploy/e2b/WORKER-BUILD.md)。

### 1.4 构建 env（摘要）

| 变量 | 用途 |
|------|------|
| `CLAW_GATEWAY_DATABASE_URL` | 构建脚本写 PG |
| `CLAW_E2B_API_URL` / `CLAW_E2B_API_KEY` | e2bserver |
| `CLAW_E2B_CN=1` | 国内 debian 镜像 |
| `CLAW_E2B_TEMPLATE_SKIP_CACHE=1` | 强制重建 |
| `CLAW_E2B_WORKER_ARCH=amd64` | 自托管 worker 节点（必须 amd64） |

---

## 2. 组件检测与生命周期维护

### 2.1 Gateway 栈健康

| 命令 | 脚本 | 检查什么 |
|------|------|----------|
| `gateway.sh verify` | `lib/claw-stack-verify.sh` | PG schema、migrate、gateway healthz；**不**验 e2b 模板 |
| `gateway.sh check` | `lib/check-connectivity.sh` | healthz + 连通性冒烟 |
| `gateway.sh solve-e2e` | `lib/admin-solve-e2e.sh` | solve_async 端到端 |

### 2.2 e2b 单例生命周期（标准路径：Gateway API）

Gateway 启动自动：`ensure_e2b_singletons_on_startup` + `reconcile_project_workers_on_startup`（Rust：`gateway_e2b_singleton_lifecycle.rs`）。

| 命令 | 作用 |
|------|------|
| `gateway.sh e2b-singletons-up` | ensure nas-api + ovs + observe |
| `gateway.sh e2b-singletons-up --reset` | 重建三个单例沙箱 |
| `gateway.sh nas-api-up` | 仅 nas-api |
| `gateway.sh ovs-up` | 仅 OVS |
| `gateway.sh observe-tap-up` | 仅 observe（写 `clawTap`） |

**前提：** gateway 已 `up`（curl `http://127.0.0.1:${GATEWAY_HOST_PORT:-18088}/healthz`）。

**Admin UI：** Playground `http://127.0.0.1:18765/admin` → **E2b 核心组件** — 查看 templateId、在线状态、ensure/reset。

**Admin API：**

```bash
curl -s "http://127.0.0.1:18088/v1/gateway/global-settings/e2b-singletons" | jq .
curl -X POST "http://127.0.0.1:18088/v1/gateway/global-settings/e2b-singletons/ovs/ensure"
curl -X POST "http://127.0.0.1:18088/v1/gateway/global-settings/e2b-singletons/ovs/reset"
```

### 2.3 E2E 与清理

| 脚本 | 用途 |
|------|------|
| `deploy/stack/lib/verify-e2b-ovs-e2e.sh` | OVS e2e |
| `deploy/stack/lib/verify-e2b-nas-inject.sh` | NAS 注入验收 |
| `deploy/stack/lib/e2b-sandbox-cleanup.sh` | 清理 orphan sandboxes |

### 2.4 Legacy（勿作默认）

`deploy/e2b/e2b-*-up.py` 可直连 e2b API 写 PG，已被 gateway API 取代。排障文档若引用 Python 脚本，优先改用 `gateway.sh` 对应命令。

---

## 3. Gateway / Admin 本地开发与发布

### 3.1 日常命令

| 目的 | 命令 | 说明 |
|------|------|------|
| 日常起栈 | `gateway.sh quick` | admin dist + playground 镜像 + up + check |
| Rust 网关改动 | `gateway.sh pack-deploy` | build → down/up → verify → check |
| 只改 env | `gateway.sh up` | 不重新编译 |
| 停栈 | `gateway.sh down` | gateway + playground；PG 可选保留 |
| 看日志 | `gateway.sh logs` / `ps` | |

**端口：** Gateway `18088`；Playground + Admin `18765`（`/admin`）。

### 3.2 Admin React 热更新（仅本地）

```bash
CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1 ./deploy/stack/gateway.sh admin-build
./deploy/stack/gateway.sh admin-reload
# 或 bind 挂载：
CLAW_GATEWAY_ADMIN_BIND=1 ./deploy/stack/gateway.sh up
```

**生产禁止**在服务器跑 `admin-build` / `admin-reload`；Admin 随 CI 镜像发布。

### 3.3 生产发布

```bash
# CI：push tag release-vX.Y.Z → .github/workflows/claw-code-image.yaml
./deploy/stack/gateway.sh up --release release-vX.Y.Z
./deploy/stack/gateway.sh verify
./deploy/stack/gateway.sh solve-e2e
```

离线镜像：`deploy/stack/lib/ship-release-tar-to-remote.sh release-vX.Y.Z`。

---

## 4. gateway.sh 命令 → 脚本索引

| 命令 | 实现脚本 |
|------|----------|
| `quick` | `lib/quick.sh` |
| `clean` | `lib/clean.sh` |
| `build` | `lib/build.sh` → `lib/linux-compile.sh` |
| `pack-deploy` | `lib/pack-deploy.sh` |
| `e2b-worker-deploy` | `lib/e2b-worker-deploy.sh` → `deploy/e2b/build-claw-worker-selfhosted.py` |
| `up` / `down` / `restart` | `lib/up.sh` / `lib/down.sh` |
| `pg-up` / `pg-down` | `lib/pg-up.sh` / `lib/pg-down.sh` |
| `admin-build` / `admin-reload` | `lib/build-gateway-admin.sh` / `lib/admin-reload.sh` |
| `check` | `lib/check-connectivity.sh` |
| `verify` | `lib/claw-stack-verify.sh` |
| `solve-e2e` | `lib/admin-solve-e2e.sh` |
| `cluster-verify` | `lib/claw-cluster-verify.sh` |
| `e2b-singletons-up` | `lib/e2b-singletons-up.sh` |
| `ovs-up` / `observe-tap-up` / `nas-api-up` | `lib/e2b-ovs-up.sh` 等 |
| `e2b-pre-bootstrap` | `lib/e2b-pre-bootstrap.sh` → `deploy/e2b/build-selfhosted-templates.sh` |
| `pre-252-e2b-up` | `lib/pre-252-e2b-pipeline.sh` |
| `install-docker` | `lib/install-docker.sh` |
| `e2e` | `tests/http-gateway-session-continuity-e2e.sh` |

完整列表：`./deploy/stack/gateway.sh help`。

---

## 5. 文档地图（deep-dive，不重复正文）

| 主题 | 文档 |
|------|------|
| 架构总纲 | [`architecture-governance.md`](architecture-governance.md) |
| 本地开发懒人版 | [`local-dev.md`](local-dev.md) |
| Worker 模板唯一手册 | [`deploy/e2b/WORKER-BUILD.md`](../deploy/e2b/WORKER-BUILD.md) |
| NAS 挂载契约 | [`e2b-nas-workspace.md`](e2b-nas-workspace.md) |
| 运维手册（排障） | [`deploy/stack/README.md`](../deploy/stack/README.md) |
| 命令真值对照 | [`deploy-ops-truth.md`](deploy-ops-truth.md) |
| Env 变量清单 | [`env-config.md`](env-config.md) |
| e2b 模板构建细节 | [`deploy/e2b/README.md`](../deploy/e2b/README.md) |
| 预发 252 全链路 | [`deploy/docs/pre-252-e2b-pipeline.md`](../deploy/docs/pre-252-e2b-pipeline.md) |
| Observe 502 排障 | [`deploy/docs/e2b-observe-tap-troubleshoot.md`](../deploy/docs/e2b-observe-tap-troubleshoot.md) |

**Cursor Skill：** [`.cursor/skills/claw-deploy-ops/SKILL.md`](../.cursor/skills/claw-deploy-ops/SKILL.md) — Agent 执行本 runbook 的可操作版本。
