# Podman：网关（http-gateway-rs）稳定部署说明

Author: kejiqing

**运维总入口：** [`docs/deploy-ops-runbook.md`](../../docs/deploy-ops-runbook.md)（e2b 注册、生命周期、gateway/admin 发布）。

**稳定做法只有一条**：在仓库根目录准备好 `.env`，用 **`./deploy/stack/gateway.sh`** 起栈；不要用「手写一长串 compose / 只挂单个 compose 文件 / 在容器里配 macOS 的 `/Users/...` 路径」这类容易翻车的玩法。

**路线方针（维护优先，不搞多套叙事）**：

| 场景 | 容器引擎 | Worker | 镜像 / 模板从哪来 | 入口命令 |
| --- | --- | --- | --- | --- |
| **本地开发** | Podman（`auto` 优先 podman） | **e2b** | gateway：`quick` / `pack-deploy`；**e2b worker 模板 dev：`e2b-worker-deploy`（不走 CI）** | `./deploy/stack/gateway.sh quick` |
| **线上 Linux** | Docker + compose | **e2b** | **只拉 CI tag**（GHCR/ACR）；e2b 模板 `from_image` | `./deploy/stack/gateway.sh up --release release-v…` |

两套环境用 **同一份脚本树** `deploy/stack/lib/`；差别在根 `.env`（模板见下表）。**solve / interactive 均经 e2b**，无宿主机 `claw-sandbox` `:9944`。

**`.env` 模板**：

| 环境 | 模板 | 说明 |
| --- | --- | --- |
| **自托管 e2b（推荐）** | `env.selfhosted-e2b.example` | 外连 PG + e2b；见 `docs/architecture-governance.md` |
| e2b interactive 叠加 | `env.e2b-interactive.example` | OVS / NAS / Observe 变量 |
| 生产 Linux | `env.production.example` | `up --release` 拉镜像 |
| 本地全栈 compose | `env.local.example` | `gateway.sh quick`（须 `CLAW_*_BACKEND=e2b`） |
| ~~稳定沙箱主机~~ | ~~`env.stable-dev-host.example`~~ | **已废弃** |
| ~~远程 pool 后端~~ | ~~`env.local-remote-backend.example`~~ | **已废弃** |

`compose-include.sh` 按 `CLAW_CONTAINER_RUNTIME` 解析 socket：**docker 只认** `/var/run/docker.sock`；**podman 不会在 macOS 上误回落到 docker.sock**。装真 Docker 的生产机可 `sudo touch /etc/containers/nodocker`，避免 podman 冒充 `docker` 命令。

`gateway.sh up` 会跑 **preflight**（socket / postgres 镜像 / Git 必填项）；**Docker 下不由脚本预建 compose 网络**（避免 `claw_default` 标签冲突）。

**单入口**：**`./deploy/stack/gateway.sh`**。日常起栈用 **`quick`**；改 Rust 网关镜像后用 **`pack-deploy`**（不要等 `podman build` 里 cargo，那会卡 `Updating crates.io index`）。

```bash
# 日常：gateway-admin dist + playground 镜像 + up + check（e2b worker，无 pool-daemon）
./deploy/stack/gateway.sh quick

# 只改 React 管理台（web/gateway-admin/src）：
./deploy/stack/gateway.sh admin-build   # 然后 quick 或 playground，并提交 dist/

# 改 rust 网关（http-gateway-rs）后：build + 重启
./deploy/stack/gateway.sh pack-deploy

# 改 e2b 沙箱里的 claw 二进制（dev，不走 CI / ACR）：
./deploy/stack/gateway.sh e2b-worker-deploy
# 唯一手册：deploy/e2b/WORKER-BUILD.md（amd64 + PG templateId + gateway 自动 reconcile）

# 怀疑缓存脏了：先 clean 或 pack-deploy --clean

# 仅清编译缓存（rust/target、.linux-artifacts；默认不删 claw-workspace）
./deploy/stack/gateway.sh clean

# 或拆开：
./deploy/stack/gateway.sh build          # 默认增量编译（--no-clean）
./deploy/stack/gateway.sh build --clean local   # 全量重编
./deploy/stack/gateway.sh pack-deploy      # 默认 --no-clean + 跳过 playground npm
./deploy/stack/gateway.sh restart

# 只重启、不重新编译（镜像已是新的才有效）
./deploy/stack/gateway.sh restart

# 宿主机单轮 solve（不经过 worker 容器）
./deploy/stack/gateway.sh solve-once-local
```

实现脚本在 **`deploy/stack/lib/`**（`pack-deploy.sh`、`build.sh`、`solve-once-local.sh` 等）。**不要**用 `build --in-container`（镜像内 cargo，慢且易超时）。`scripts/local-pack-deploy.sh` 等仅为兼容，转调 `gateway.sh`。

其中 `./deploy/stack/gateway.sh build` 通过 **`lib/build.sh`**：`linux-compile` 产出 **`http-gateway-rs` + `claw`**，再 **`Containerfile.gateway-rs.prebuilt`** **COPY** 预编译产物（镜像内不 cargo）。**e2b worker 模板**在 `deploy/e2b/`，不在 gateway 镜像内编译。

**线上部署（与 GitHub Actions 一致）**：打 tag `release-*` 触发 [`.github/workflows/claw-code-image.yaml`](../../.github/workflows/claw-code-image.yaml)。包名：**`claw-code`**、**`claw-gateway-playground`** 等（**同一 tag**）。服务器 **`./deploy/stack/gateway.sh up --release release-vX.Y.Z`**；**不要**在服务器跑 **`build`** / **`admin-build`**。Worker 执行在 **e2b**，非宿主机 pool。

**镜像仓库默认（国内）**：未设置 **`CLAW_IMAGE_PREFIX`** / **`CLAW_GHCR_PREFIX`** 且 **`GATEWAY_IMAGE`** 不含 `…/claw-code` 时，`./deploy/stack/gateway.sh up --release …` 默认从 **阿里云个人版 ACR**（`crpi-….personal.cr.aliyuncs.com/passionke`，可由 **`CLAW_ACR_IMAGE_PREFIX`** 覆盖）拼接镜像名；若要改用 GHCR，在根目录 **`.env`** 设 **`CLAW_IMAGE_REGISTRY=ghcr`**（默认前缀 **`ghcr.io/passionke`**，可由 **`CLAW_GHCR_DEFAULT_PREFIX`** 覆盖）。仍可直接设 **`CLAW_IMAGE_PREFIX=…`**（不要 `https://`），优先级最高。

**国内拉 GHCR 很慢**：同一 release tag 在 GHCR build 完成后由 **`mirror-to-acr`** 推到 **ACR**（见 [`claw-code-image.yaml`](../../.github/workflows/claw-code-image.yaml)）。拉取前 **`podman login`** / **`docker login`** 对应 registry。与 **`CLAW_IMAGE_PREFIX`** 等价的老变量名是 **`CLAW_GHCR_PREFIX`**。

**GHCR 握手超时 / 服务器拉不下来**：在能稳定访问镜像源的环境执行 **`./deploy/stack/lib/ship-release-tar-to-remote.sh release-v1.0.25`**（默认推到 **`admin@192.168.9.252` 的 `~`**）；本机若也拉不动 GHCR，可先设 **`CLAW_SHIP_REGISTRY_PREFIX=…`** 指向已能拉到的 ACR 前缀再跑脚本。远端 **`podman load -i`** / **`docker load -i`** 后，再在服务器上 **`CLAW_IMAGE_PREFIX=… ./deploy/stack/gateway.sh up --release …`**。

**同一套脚本、本地与线上共用**：`deploy/stack/lib/` 下的 `build.sh` / `up.sh` / `down.sh` / tap / `bench-pool-30s.sh` 由 **`gateway.sh`** 调用；它们通过 **`CLAW_CONTAINER_RUNTIME`** 选 CLI——默认 **`auto`**（PATH 里**有 podman 先用 podman**，否则 **docker**）。线上常只有 docker，无需改 `.env`；本机有 podman 也会自动走 podman。只有两台都装了且必须指定时，才设 **`CLAW_CONTAINER_RUNTIME=podman`** 或 **`docker`**。

更全的接口与本地调试见：`docs/http-gateway-rs-quickstart.md`（第二节已与本文对齐）。

---

## 1. 稳定路径（按顺序做）

### 1.1 环境

**Linux 线上首次**：宿主机须装 **Docker**（`e2b` + compose）。标准化一条命令（与网关镜像内 `docker.io` 包一致；默认配置 `docker.1ms.run` 拉取镜像；`CLAW_USE_DOCKER_IO=1` 跳过镜像加速）：

```bash
./deploy/stack/gateway.sh install-docker
```

装完后当前用户若还不能访问 `/var/run/docker.sock`，执行 `newgrp docker` 或重新登录。

```bash
# macOS 本地
cp deploy/stack/env.local.example .env
# Linux 线上
cp deploy/stack/env.production.example .env
```

**双模式**（`CLAW_DEPLOY_PROFILE`，详见 `docs/env-config.md`）：

| Profile | 环境 | 启动 |
| --- | --- | --- |
| `local` | macOS + podman，**e2b worker** | `gateway.sh quick` |
| `production` | Linux + docker，**e2b worker** | `gateway.sh up --release release-vX.Y.Z` |

在 **仓库根目录** `.env` 里至少保证：

| 变量 | 作用 |
| --- | --- |
| `CLAW_DEPLOY_PROFILE` | `local` 或 `production`（可按 OS 自动推断） |
| `CLAW_CLUSTER_ID` | 仓库根 `.env` **必填**；集群标识（Admin 只读展示） |
| `CLAW_GATEWAY_DATABASE_URL` | 网关 PG；与 `CLAW_CLUSTER_ID` 一起参与 `clusterHash` 校验（见 `docs/claw-tap-cluster-identity.md`） |
| `CLAUDE_TAP_MODE` | 本地建议 **`source`**（`../claude-tap` 可编辑安装）；**`native`/PyPI 0.0.7** 的 hash 算法与网关不一致会报 `clusterHash mismatch` |
| Admin → 全局推理（PG） | **clawTap** 端点、活跃 LLM 模型/API Key；solve 时 gateway 经 pool `Exec -e` 注入 worker（`OPENAI_BASE_URL` = clawTap） |
| `CLAW_LLM_PROXY` / `CLAW_TAP_PROXY_URL` | 仅影响本机 **tap 侧车** 或 compose 里 tap 容器地址；**不**再决定 worker 是否绕过 clawTap |
| `gateway.sh up --release <tag>` | `GATEWAY_IMAGE` 与 **`CLAW_DOCKER_IMAGE`** 同 tag（`claw-code`→`claw-gateway-worker`）；**`CLAW_RELAXED_PODMAN_IMAGE`** 同 tag（`→claw-gateway-worker-relaxed`）；勿在根 `.env` 写死 `:local` worker |
| `GATEWAY_HOST_PORT` | 宿主机端口，默认 `8088` |
| `GATEWAY_PLAYGROUND_HOST_PORT` | solve_async / 项目管理 UI，默认 `18765`（compose 服务 `gateway-playground`） |
| `PLAYGROUND_PUBLIC_GATEWAY_BASE` | 浏览器里 playground 默认网关，应与 `GATEWAY_HOST_PORT` 一致，如 `http://127.0.0.1:8088` |
| `PLAYGROUND_ADMIN_USER` / `PLAYGROUND_ADMIN_PASSWORD` | `/admin` 登录账号密码（默认 `admin` / `sunmi123`） |
| `CLAW_E2B_API_URL` / `CLAW_E2B_SANDBOX_URL` | e2b API（solve + interactive **必填**） |
| `CLAW_E2B_API_KEY` | e2b 认证 |
| `CLAW_GATEWAY_DATABASE_URL` | 必填（网关进程）；compose 内网关连 **`postgres:5432`**；宿主机映射默认 **`127.0.0.1:5433`**（`CLAW_GATEWAY_PG_HOST_PORT`，避开 sqlbot 常用 5432） |
| `project_config`（PG） | **必填（业务）**：规则 / MCP / skills / `CLAUDE.md` 在 Admin 或 API 写入 DB，网关物化到 `ds_<id>/home` |
| `git_sync_json`（PG，每 ds） | 可选：每项目单向 push 的 `gitUrl` / `gitRef` / token（见 `docs/project-config-model.md`） |
| `CLAW_PROJECTS_GIT_AUTHOR` | 可选：gitSync 未填 author 时的默认 commit 作者 |

`solve` 与 interactive 均走 **FC / e2b**（`CLAW_SOLVE_ISOLATION=e2b`、`CLAW_INTERACTIVE_BACKEND=e2b`）。

### 1.2 镜像

**本地开发（macOS）**：**首次** `podman run` 编译会慢（拉依赖，和 Rust 有关，不是网关逻辑慢）；**第二次起** 卷 `claw-cargo-registry` 缓存后明显变快。镜像打包只做 COPY（秒级）。`.env` 保留 `CLAW_USE_CN_CRATES_MIRROR=1`。

**Linux / CI**：镜像内完整编译，用同一 `gateway.sh build`（非 Darwin 路径）。

### 1.3 启动与检查

```bash
./deploy/stack/gateway.sh up
```

`gateway.sh up`（`lib/up.sh`）会：

- 生成 compose 所需 env（`deploy/stack/.claw-pool-workspace.env` 等历史文件名仍可能存在）。
- **`claw_compose`**：按 **`CLAW_CONTAINER_RUNTIME`** 调用 **`docker compose`** 或 **`podman compose`**。
- 使用 **`up -d --force-recreate`**。
- **不**启动宿主机 pool-daemon；solve 经 **e2b API**（见 `docs/http-gateway-container-pool.md`）。

检查：

```bash
curl -sS "http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/healthz"
curl -sS "http://127.0.0.1:${GATEWAY_PLAYGROUND_HOST_PORT:-18765}/"
podman ps   # 或  docker ps  — 应有 claw-gateway-rs、gateway-playground
```

### 1.4 停止

```bash
./deploy/stack/gateway.sh down
```

### 1.5 LLM 路由（clawTap 必选）

- Admin 配置 **clawTap**（探测 `/healthz` 的 `clusterHash` 须与 gateway 同一 PG 推导结果一致）。
- **每次 solve**：gateway 将 `OPENAI_BASE_URL` 设为 clawTap 基址（`Exec -e` 注入）；**无** cluster 不一致时直连 upstream 的降级路径。
- `GET /readyz` 在 `clawTapCluster.consistency=strict` 前返回 503；不一致为 `cluster_mismatch`，solve 被拒绝。

| `CLAW_LLM_PROXY` | 场景（tap **进程** 部署，与 solve 注入分离） |
| --- | --- |
| `local`（macOS 默认） | 本机 `gateway.sh tap-up` 起侧车；Admin clawTap 指向该地址 |
| `remote` | `CLAW_TAP_PROXY_URL` 指向集群共享 claude-tap；Admin clawTap 与之对齐 |

```bash
# local 开发：gateway.sh up / pack-deploy 在 CLAW_LLM_PROXY=local 时自动 tap-up
./deploy/stack/gateway.sh tap-down   # 仅停 tap（网关不动）
./deploy/stack/gateway.sh tap-up     # 仅起 tap（网关已 up 时补启）
```

**Mac 常见报错** `clawTap health fetch failed … host.containers.internal:8080`：宿主机 **native/source** tap 已挂。**推荐** docker tap：`CLAUDE_TAP_MODE=docker` + `CLAUDE_TAP_DOCKER_NETWORK=claw_default` + **`CLAW_PODMAN_NETWORK=claw_default`**（worker / gateway / tap **必须同网**，否则 worker 里 `OPENAI_BASE_URL=http://claw-claude-tap:8080` 解析失败 → **120s timeout**）。`gateway.sh build-tap && gateway.sh pool-reset && gateway.sh up`。

详见 `docs/claw-tap-cluster-identity.md`。`claude-tap` 为 OpenAI 兼容代理，不是 MCP。

**Live Viewer（`CLAUDE_TAP_LIVE_PORT`，默认 3000）与 `?session=`**（已对照上游 **`claude-tap` 0.1.52** 安装树：`claude_tap/live.py` 的 `GET /` 不读取 query；`viewer.html` 内也无对 `location.search` / `URLSearchParams` 的解析）：

- **`http://127.0.0.1:<live_port>/` 只展示当前这次 tap 进程绑定的 `trace_*.jsonl` 实时流**（`cli.py` 在**启动 tap 时的当前工作目录**下写 **`.traces/<日期>/trace_<HHMMSS>.jsonl`** 并交给 `LiveViewerServer`；常见为仓库根 `./.traces/`，取决于你从哪个目录执行 `gateway.sh tap-up`）。
- **URL 里的 `?session=…` 不会被 Live Viewer 用来筛选或定位 trace**；浏览器会把查询串发给服务器，但 tap 侧实现忽略之，因此「随便填一个 id（含网关 `/healthz` 返回的 `claw-session-id`）」**页面行为与不带 query 相同**，并不是「两个系统 id 没对齐才空白」这一种原因。
- **网关**的 `claw-session-id` / `/v1/solve` 的 `sessionId` 属于 **`http-gateway-rs` 与会话库**，与 tap 的 trace 文件命名**无契约绑定**；要对齐排障应分别看：网关 **`/healthz`** / 日志；tap **`.traces/` 目录**或 Viewer 里按日期/文件选的记录。

---

## 2. 设计约定（知道这些就够排障）

- **网关容器内**的池化路径必须是 Linux 里存在的路径；compose 把 `deploy/stack/claw-workspace` 挂到 **`/var/lib/claw/workspace`**，池绑定根目录与之一致。
- **会话 / 轮次 / 反馈（PostgreSQL）**：`podman-compose.yml` 启动 **`postgres`**（数据卷 **`./claw-postgres-data`**），网关通过 **`CLAW_GATEWAY_DATABASE_URL`** 连接。生产可将 URL 指向**独立 PG**（仅改连接串，无需与网关同 compose）。`/healthz` 的 **`gatewayDatabaseUrl`**（脱敏）与 **`sessionDatabaseBackend`** 可核对。
- **Compose 后端**：需要 `podman-compose` 时 `brew install podman-compose`；勿假定 `podman compose` 一定走 Docker 的 compose。

FC / e2b 设计与 env 契约见 `docs/http-gateway-container-pool.md`、`deploy/e2b/README.md`。

---

## 3. 常见问题（短）

| 现象 | 处理 |
| --- | --- |
| Admin `solve_async` **503** | 查 `CLAW_E2B_API_URL`、API key、e2b 模板；gateway 日志与 `deploy/e2b/README.md` |
| `podman ps` 看不到网关 | `podman ps -a \| grep claw-gateway-rs`，看 `podman logs claw-gateway-rs` |
| e2b worker 创建失败 | e2bserver 日志、NAS mount、`docs/e2b-nas-workspace.md` |
| observe / clawTap **502**、Admin 等 observe-tap-up | `deploy/docs/e2b-observe-tap-troubleshoot.md`；`observe-tap-up --reset` |
| 改 `.env` 不生效 | **`./deploy/stack/gateway.sh up`**（`--force-recreate`） |
| 改了 `rust/` 网关逻辑仍像旧的 | **`./deploy/stack/gateway.sh pack-deploy`** |

联通性：`./deploy/stack/gateway.sh check`。

---

## 4. 构建说明摘录

- 基础镜像仓库：默认 `CONTAINER_BASE_REGISTRY=docker.1ms.run`（`.env`）；`CLAW_USE_DOCKER_IO=1` 时用 `docker.io`。
- 国内可选：`CLAW_USE_CN_RUST_MIRROR=1`（仅影响 **首次** rustup 相关层；镜像已改为用 base 镜像自带 **stable**，不再 `rustup install nightly`，避免 nightly 每天更新导致反复下 `rust-std`）。宿主 `rust/.cargo/config.toml.example` 拷贝见 `.env.example` 注释。

---

## 5. 环境变量：只维护根 `.env`

网关 compose **只加载仓库根**的 `.env`。`deploy/stack/` 下由 **`gateway.sh up`** 生成的 `*.env` 为**中间物**，不要手改。

- **全量 env 清单：** `docs/env-config.md`
- **文档索引：** `docs/README.md`
- **人手 vs 生成物：** `docs/env-files.md`
