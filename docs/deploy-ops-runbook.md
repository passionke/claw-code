# Claw 运维部署 Runbook（本机开发 + 部署运维）

Author: kejiqing

**唯一入口命令：** `./deploy/stack/gateway.sh`（实现脚本在 `deploy/stack/lib/`）。

**三层边界（勿混）：** 见 [`deploy/SERVICES.md`](../deploy/SERVICES.md)。

| 层 | Jenkins Job | 产物 | 写 PG？ |
|----|-------------|------|---------|
| Gateway | `claw-code-gateway-nora` | Nora `claw-code` | 否 |
| Playground | `claw-code-playground-nora` | Nora `claw-gateway-playground` | 否 |
| e2b 协议 | `claw-e2b-protocol-nora` | Nora `claw-worker-base*`（neuro-* only）+ e2b `Template.build` | **否** |
| Agent 引擎 / neuro-harness-acp | `claw-agent-engine-nora` | Nora raw tar（含 `claw`） | 否 |
| 运行时绑定 | Admin / Gateway API | `e2bWorker.templateId` / `agentEngines` | **是** |

---

## H. home 系列升级（Nora + e2b.home）— 默认运维路径

**Home 仓库：** `https://code.passionke.top/passionke/claw-code.git`  
**Home git tag：** **`vX.Y.Z`**（禁止在 gitea 打 `release-v*`；`release-v*` 只给 GitHub Actions）  
**制品：** `nora.home.passionke.top/passionke`  
**e2b API：** `http://e2b.home.passionke.top:3000`（sandbox `:3002`）  
**Jenkins：** `https://jenkins.home.passionke.top/`（agent label **`nora-pack`**；tag hook 自动跑 Gateway Job）  
**neurogate：** `https://neurogate.home.passionke.top`（`10.8.0.21`）

### H.1 Gateway/Admin 发版（职责拆分）

| 角色 | 机器 / Job | 做什么 | 不做什么 |
|------|------------|--------|----------|
| **重活** | Jenkins `claw-code-gateway-nora` / `claw-code-playground-nora`（label `nora-pack`） | path 门禁；各自 clone、编译/构建、skopeo 推 Nora | 不改 neurogate 运行态 |
| **轻活** | `10.8.0.21` Gitea Actions（label `neurogate`） | 等/拉 Nora 镜像 → `gateway.sh up --release` | **不**编译、**不**推镜像 |

**`nora-pack` 节点（离线自动换机，禁止写死单机）：**

| 节点 | 角色 |
|------|------|
| `home29`（`10.8.0.29`） | 主打包机；label 含 `nora-pack` + `primary` |
| `home-ubt`（`10.8.0.21` **宿主机** SSH agent） | 备份打包机（Jenkins 容器也在 21，但 Built-In **不**打包，`numExecutors=0`） |

```text
正式线（v* tag）:
  git tag vX.Y.Z && git push gitea vX.Y.Z
  → Actions home-neurogate-deploy
  → Jenkins gateway + playground GIT_TAG=vX.Y.Z → Nora :vX.Y.Z
  → up --release vX.Y.Z

合入 main:
  → Actions home-neurogate-up
  → Jenkins GIT_REF=<sha>（path 门禁；变更侧 Nora :sha-<12> + :latest）
  → 至少一侧打出 sha 镜像后 up --release latest
  → 两侧都跳过则不 up
```

Jenkins 入参：`GIT_TAG`（正式）与 `GIT_REF`（main 这次提交）二选一。  
由 Actions `home-neurogate-pack-via-jenkins.sh` 调 Jenkins CLI 并等待；**不要**再配 Gitea webhook。

**home29 / CI 编译（gateway Job）— 单一默认路径：**

| 旋钮 | 默认 | 作用 |
|------|------|------|
| release `opt-level` | **3**（Cargo.toml） | 不在 CI 里永久降到 2 |
| linker | **mold**（`config.toml.example`） | 不在 CI 里永久改成 lld |
| `CARGO_INCREMENTAL` | `0`（CI） | 每次 job 的 target 目录会被清，关掉 incremental |
| `CLAW_DISABLE_SCCACHE` | **`1`（home 打包）** | Nora/gateway Job 不走 sccache，直调 rustc |
| `CLAW_CARGO_CACHE_RESET=1` / Job 勾选 `CARGO_CACHE_RESET` | 关 | 编译前清空 `cargo-registry` + `cargo-git` + `sccache` |
| `CLAW_COMPILE_RETRY_ON_FAIL` | `1`（CI） | **失败后自动清上述缓存再编一次**（标准恢复） |

不要用拧 `opt-level` / `jobs` 当根因修复。若清缓存重试后仍是 `rustc`/`gcc`/`ld` SIGSEGV，查 **home29 主机**（`dmesg`、重启、memtest），不是再改 Cargo 旋钮。

用户日常**不**手搓 `up --release`（紧急除外）。  
- 正式：[`home-neurogate-deploy.yml`](../.gitea/workflows/home-neurogate-deploy.yml)  
- 21 启动：[`home-neurogate-up.yml`](../.gitea/workflows/home-neurogate-up.yml)（`push: branches: [main]`，先 Jenkins 再 up）  
- 脚本：[`home-neurogate-pack-via-jenkins.sh`](../deploy/stack/lib/home-neurogate-pack-via-jenkins.sh) 然后 [`home-neurogate-deploy-from-tag.sh`](../deploy/stack/lib/home-neurogate-deploy-from-tag.sh)（`v*` 或 `latest`）  
- runner：[`gitea-act-runner-neurogate.md`](../deploy/docs/gitea-act-runner-neurogate.md)  
- 队列：`up` / `deploy` **各自** `concurrency` + `cancel-in-progress`（新 run 取消同组旧排队/在跑；两组互不掐）

### H.1b 协议 / Agent 引擎（仍各自 Job）

```text
Jenkins claw-e2b-protocol-nora  GIT_TAG=vX.Y.Z
Jenkins claw-agent-engine-nora  GIT_TAG=vX.Y.Z ENGINE_ID=… VERSION=…
```

协议 Job 成功日志应有：

- `OK: template 'claw-worker' (tpl_…) ready on http://e2b.home.passionke.top:3000`
- `OK: relaxed worker template 'claw-worker-relaxed' (tpl_…) from_image`
- **不会**写 Gateway PG（正确）。

### H.2 紧急手工起 / 升级 Gateway

根 `.env` 至少：

```bash
CLAW_IMAGE_PREFIX=nora.home.passionke.top/passionke
CLAW_E2B_API_URL=http://e2b.home.passionke.top:3000
CLAW_E2B_SANDBOX_URL=http://e2b.home.passionke.top:3002
# + CLAW_GATEWAY_DATABASE_URL / CLAW_CLUSTER_ID / e2b key 等
```

```bash
# 同 tag 镜像已在 Nora 时：
bash deploy/stack/lib/home-neurogate-deploy-from-tag.sh vX.Y.Z
# 或等价：
CLAW_IMAGE_PREFIX=nora.home.passionke.top/passionke \
  ./deploy/stack/gateway.sh up --release vX.Y.Z
./deploy/stack/gateway.sh verify
```

`up --release` 拉的镜像名是 **`claw-code`** / **`claw-gateway-playground`**（与 Nora Job 一致）。

### H.3 绑定运行时（Admin，写 PG）

1. Playground Admin → **E2b 核心组件**
2. 从 e2b 模板列表选 `claw-worker` / `claw-worker-relaxed`（或对应 `tpl_…`）→ 保存  
   → `PUT …/e2b-worker` 写 `e2bWorker.templateId`
3. **Agent 引擎 map**：填 Nora raw URL + digest → 保存  
   例：`https://nora.home.passionke.top/raw/claw-agent-engines/opencode-1.18.34-amd64.tar.gz`
4. 已有 worker：Admin reset / 重建后才吃新 template / 新 engine。

### H.4 验收

```bash
./deploy/stack/gateway.sh check
./deploy/stack/gateway.sh solve-e2e   # 或 Admin 上跑一题
```

边界文档：[`deploy/SERVICES.md`](../deploy/SERVICES.md)。协议细节：[`deploy/e2b/WORKER-BUILD.md`](../deploy/e2b/WORKER-BUILD.md)。

---

**Env 模板（本机 / 非 home）：** 复制 [`deploy/stack/env.selfhosted-e2b.example`](../deploy/stack/env.selfhosted-e2b.example) → 仓库根 `.env`。

**IP 真值（勿混用；旧 lab 地址，home 以 H 节为准）：**

| 地址 | 角色 |
|------|------|
| `10.8.0.1` | 旧 lab PostgreSQL `:5433`（home 用自己的 PG） |
| `e2b.home.passionke.top` | home e2b API `:3000` / sandbox `:3002` |
| `10.8.0.11` | NAS NFS export（若启用） |

**Backend 真值：** `CLAW_INTERACTIVE_BACKEND=e2b`、`CLAW_SOLVE_ISOLATION=e2b`。

---

## 0. 首次部署顺序（本机 / 非 home Jenkins）

```bash
# 1. 配置
cp deploy/stack/env.selfhosted-e2b.example .env   # 编辑 CLAW_CLUSTER_ID、PG URL、e2b keys

# 2. 协议镜像已在 registry 后，注册 e2b 模板（不写 Gateway PG）
./deploy/e2b/bootstrap-templates-from-ci-tag.sh release-vX.Y.Z

# 3. 起 gateway + playground
./deploy/stack/gateway.sh quick

# 4. Admin 绑定 worker templateId + agentEngines（写 PG）

# 5. 确保 e2b 单例（nas-api / observe）— gateway 启动也会自动 ensure
./deploy/stack/gateway.sh e2b-singletons-up

# 6. 验收
./deploy/stack/gateway.sh verify
./deploy/stack/gateway.sh check
```

预发 252：`cp deploy/stack/env.pre-252.e2b.example .env` 后 `./deploy/stack/gateway.sh up --release <tag>`（见 [`deploy/docs/pre-252-e2b-pipeline.md`](../deploy/docs/pre-252-e2b-pipeline.md)）。

---

## 1. e2b 模板与 PG 绑定（运行时）

### 1.1 组件与 PG 契约

所有**运行时**配置在 PostgreSQL 表 `gateway_global_settings`（按 `CLAW_CLUSTER_ID`）的 `settings_json`：

| 组件 | 谁产出模板 | PG 谁写 | PG 键 | 默认 alias |
|------|------------|---------|-------|------------|
| Worker (strict) | 协议 Job / `publish-worker-protocol.sh` | **Admin / Gateway API** | `e2bWorker.templateId` | `claw-worker` |
| Worker (relaxed) | 同上 | **Admin / Gateway API** | `e2bWorkerRelaxed.templateId` | `claw-worker-relaxed` |
| Worker SKU (cpu/mem) | e2b `Template.build` only（`ensure-worker-resource-template.py`） | Admin「确保派生模板」/ Gateway API | 项目 `resourceTemplate` | `claw-worker-r-cpuN-memM` |
| NAS API | `build-claw-nas-api-selfhosted.py` 等 | Gateway ensure / Admin | `e2bNasApi.templateId` | `claw-nas-api` |
| Observe | `build-claw-observe-selfhosted.py` | Gateway ensure / Admin | `e2bObserve.templateId` | `claw-observe` |

**协议发布 Job 不写 PG。** 绑定入口：Admin → E2b 核心组件。

**SKU 不编镜像：** Gateway 只 spawn 脚本 → e2b 用已有 `{CLAW_IMAGE_PREFIX}/debian-bookworm-claw-worker:{CLAW_E2B_WORKER_BASE_TAG}` 做 `Template.build`；缺 PREFIX/BASE_TAG 或 Nora 无该镜像则硬失败（先跑协议 Job）。

### 1.2 协议模板注册（制品侧）

```bash
# home：Jenkins claw-e2b-protocol-nora
# 或已有 Nora 镜像时本地注册：
CLAW_IMAGE_PREFIX=nora.home.passionke.top/passionke \
CLAW_E2B_API_URL=http://e2b.home.passionke.top:3000 \
  bash deploy/e2b/bootstrap-templates-from-ci-tag.sh release-vX.Y.Z
```

手册：[`deploy/e2b/WORKER-BUILD.md`](../deploy/e2b/WORKER-BUILD.md)。

### 1.3 构建 env（摘要）

| 变量 | 用途 |
|------|------|
| `CLAW_E2B_API_URL` / `CLAW_E2B_API_KEY` | e2bserver（注册） |
| `CLAW_IMAGE_PREFIX` | Nora / registry 前缀 |
| `CLAW_E2B_TEMPLATE_SKIP_CACHE=1` | 强制重建模板 |
| `CLAW_GATEWAY_DATABASE_URL` | **仅** Gateway/Admin 运行时与单例脚本；协议 Job 不用 |

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
| `e2b-worker-deploy` | 已移除。模板发布：`deploy/e2b/bootstrap-templates-from-ci-tag.sh` |
| `up` / `down` / `restart` | `lib/up.sh` / `lib/down.sh` |
| `pg-up` / `pg-down` | `lib/pg-up.sh` / `lib/pg-down.sh` |
| `admin-build` / `admin-reload` | `lib/build-gateway-admin.sh` / `lib/admin-reload.sh` |
| `check` | `lib/check-connectivity.sh` |
| `verify` | `lib/claw-stack-verify.sh` |
| `solve-e2e` | `lib/admin-solve-e2e.sh` |
| `cluster-verify` | `lib/claw-cluster-verify.sh` |
| `e2b-singletons-up` | `lib/e2b-singletons-up.sh` |
| `ovs-up` / `observe-tap-up` / `nas-api-up` | `lib/e2b-ovs-up.sh` 等 |
| `e2b-pre-bootstrap` | `lib/e2b-pre-bootstrap.sh`（仅 `--skip-templates` 拉单例；不打模板） |
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
