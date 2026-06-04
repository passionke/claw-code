# Podman：网关（http-gateway-rs）稳定部署说明

Author: kejiqing

**稳定做法只有一条**：在仓库根目录准备好 `.env`，用 **`./deploy/stack/gateway.sh`** 起栈；不要用「手写一长串 compose / 只挂单个 compose 文件 / 在容器里配 macOS 的 `/Users/...` 路径」这类容易翻车的玩法。

**路线方针（维护优先，不搞多套叙事）**：

| 场景 | 容器引擎 | 镜像从哪来 | 入口命令 |
| --- | --- | --- | --- |
| **本地开发**（笔记本 / 研发机） | **Podman**（`auto` 时 PATH 里优先 podman） | **本机编译打包**：`gateway.sh quick`（日常）或 **`pack-deploy`**（改 Rust/网关镜像后；慢但可预期） | `./deploy/stack/gateway.sh quick` / `pack-deploy` |
| **线上** | **Docker**（`env.production.docker.example`） | **只拉 CI 打的 tag**（GHCR/ACR），服务器 **不跑 cargo 编网关** | `./deploy/stack/gateway.sh up --release release-v…` |

两套环境用 **同一份脚本树** `deploy/stack/lib/`；差别只在根 `.env`（模板见下表）。`deploy/podman/*.sh` 仅为旧路径 **exec 转发** 到 `deploy/stack/lib/`，新文档不再展开。

**`.env` 模板（与上表对应）**：

| 环境 | 模板 | 关键变量 |
| --- | --- | --- |
| 生产 Linux | `env.production.docker.example` | `CLAW_CONTAINER_RUNTIME=docker`，`CLAW_SOLVE_ISOLATION=docker_pool`，**不要** `CLAW_CONTAINER_SOCKET` |
| 本地 / rootless podman | `env.production.rootless.example` | `CLAW_CONTAINER_RUNTIME=podman`；Linux 可选手写 socket；**macOS** 一般留空（自动用 `podman machine` API sock） |

`compose-include.sh` 按 `CLAW_CONTAINER_RUNTIME` 解析 socket：**docker 只认** `/var/run/docker.sock`；**podman 不会在 macOS 上误回落到 docker.sock**。装真 Docker 的生产机可 `sudo touch /etc/containers/nodocker`，避免 podman 冒充 `docker` 命令。

`gateway.sh up` 会跑 **preflight**（socket / postgres 镜像 / Git 必填项）；**Docker 下不由脚本预建 compose 网络**（避免 `claw_default` 标签冲突）。

**单入口**：**`./deploy/stack/gateway.sh`**。日常起栈用 **`quick`**；改 Rust 网关镜像后用 **`pack-deploy`**（不要等 `podman build` 里 cargo，那会卡 `Updating crates.io index`）。

```bash
# 日常：host pool-daemon + gateway-admin dist + playground 镜像 + up + check
./deploy/stack/gateway.sh quick

# 只改 React 管理台（web/gateway-admin/src）：
./deploy/stack/gateway.sh admin-build   # 然后 quick 或 playground，并提交 dist/

# 改 rust 网关 / worker 镜像后：build + 重启（默认保留编译缓存，日志 deploy/stack/.build.log）
./deploy/stack/gateway.sh pack-deploy

# 怀疑缓存脏了：先 clean 或 pack-deploy --clean

# 仅清编译缓存（rust/target、.linux-artifacts；默认不删 claw-workspace）
./deploy/stack/gateway.sh clean

# 或拆开：
./deploy/stack/gateway.sh build          # 默认先 clean，再 podman run 编译 + 打镜像
./deploy/stack/gateway.sh build --no-clean local   # 增量编译
./deploy/stack/gateway.sh pack-deploy      # 默认 --no-clean + 跳过 playground npm
./deploy/stack/gateway.sh restart

# 只重启、不重新编译（镜像已是新的才有效）
./deploy/stack/gateway.sh restart

# 宿主机单轮 solve（不经过 worker 容器）
./deploy/stack/gateway.sh solve-once-local
```

实现脚本在 **`deploy/stack/lib/`**（`pack-deploy.sh`、`build.sh`、`solve-once-local.sh` 等）。**不要**用 `build --in-container`（镜像内 cargo，慢且易超时）。`scripts/local-pack-deploy.sh` 等仅为兼容，转调 `gateway.sh`。

其中 `./deploy/stack/gateway.sh build` 通过 **`lib/build.sh` 一次串联**：先 **`Containerfile.gateway-rs`**（`http-gateway-rs` + 宿主机用的 **`claw-pool-daemon`**），再 **`Containerfile.gateway-worker`**（池内 **`claw`**），共用同一套 **`rust/`** 与 base / rustup build-arg，避免「网关新、worker 旧」。

**线上部署（与 GitHub Actions 一致）**：打 tag `release-*` 触发 [`.github/workflows/claw-code-image.yaml`](../../.github/workflows/claw-code-image.yaml)，镜像推到 **`ghcr.io/<owner>/claw-code`**、**`claw-gateway-worker`**、**`claw-gateway-playground`**（**同一 tag**；playground 镜像内 **CI 多阶段构建** `gateway-admin`，含 `dist/assets/*.js`）。服务器 **`./deploy/stack/gateway.sh up --release release-vX.Y.Z`** 会写 **`deploy/stack/.claw-image-release.env`**（含 **`GATEWAY_PLAYGROUND_IMAGE`**），**不要**在服务器跑 **`build`** / **`admin-build`** / **`admin-reload`**（无需 Node/npm）。**`/admin` 白屏**多为旧 playground 镜像缺 JS：拉 **含本修复之后** 的 release tag 并 `up --release` 重建 `gateway-playground`。**`./deploy/stack/gateway.sh up`** 用同一 **`GATEWAY_IMAGE`** 起 compose 服务 **`claw-pool-daemon`**（挂容器引擎 socket + 工作区宿主机路径），**不再**向宿主机 `install` 二进制。横向扩容：每台机器 **`up --release <tag>` + 根目录 `.env`** 即可。校验/发布镜像见 [`claw-code-image.yaml`](../../.github/workflows/claw-code-image.yaml)（GHCR）与 [`claw-code-acr.yaml`](../../.github/workflows/claw-code-acr.yaml)（ACR）。

**镜像仓库默认（国内）**：未设置 **`CLAW_IMAGE_PREFIX`** / **`CLAW_GHCR_PREFIX`** 且 **`GATEWAY_IMAGE`** 不含 `…/claw-code` 时，`./deploy/stack/gateway.sh up --release …` 默认从 **阿里云个人版 ACR**（`crpi-….personal.cr.aliyuncs.com/passionke`，可由 **`CLAW_ACR_IMAGE_PREFIX`** 覆盖）拼接镜像名；若要改用 GHCR，在根目录 **`.env`** 设 **`CLAW_IMAGE_REGISTRY=ghcr`**（默认前缀 **`ghcr.io/passionke`**，可由 **`CLAW_GHCR_DEFAULT_PREFIX`** 覆盖）。仍可直接设 **`CLAW_IMAGE_PREFIX=…`**（不要 `https://`），优先级最高。

**国内拉 GHCR 很慢**：同一 tag 可由 [`.github/workflows/claw-code-acr.yaml`](../../.github/workflows/claw-code-acr.yaml) 推到 **ACR**。拉取前 **`podman login`** / **`docker login`** 对应 registry。与 **`CLAW_IMAGE_PREFIX`** 等价的老变量名是 **`CLAW_GHCR_PREFIX`**。

**GHCR 握手超时 / 服务器拉不下来**：在能稳定访问镜像源的环境执行 **`./deploy/stack/lib/ship-release-tar-to-remote.sh release-v1.0.25`**（默认推到 **`admin@192.168.9.252` 的 `~`**）；本机若也拉不动 GHCR，可先设 **`CLAW_SHIP_REGISTRY_PREFIX=…`** 指向已能拉到的 ACR 前缀再跑脚本。远端 **`podman load -i`** / **`docker load -i`** 后，再在服务器上 **`CLAW_IMAGE_PREFIX=… ./deploy/stack/gateway.sh up --release …`**。

**同一套脚本、本地与线上共用**：`deploy/stack/lib/` 下的 `build.sh` / `up.sh` / `down.sh` / tap / `bench-pool-30s.sh` 由 **`gateway.sh`** 调用；它们通过 **`CLAW_CONTAINER_RUNTIME`** 选 CLI——默认 **`auto`**（PATH 里**有 podman 先用 podman**，否则 **docker**）。线上常只有 docker，无需改 `.env`；本机有 podman 也会自动走 podman。只有两台都装了且必须指定时，才设 **`CLAW_CONTAINER_RUNTIME=podman`** 或 **`docker`**。

更全的接口与本地调试见：`docs/http-gateway-rs-quickstart.md`（第二节已与本文对齐）。

---

## 1. 稳定路径（按顺序做）

### 1.1 环境

```bash
# macOS 本地
cp deploy/stack/env.local.example .env
# Linux 线上
cp deploy/stack/env.production.example .env
```

**双模式**（`CLAW_DEPLOY_PROFILE`，详见 `docs/env-config.md`）：

| Profile | 环境 | 启动 |
| --- | --- | --- |
| `local` | macOS + podman，`podman_pool` | `gateway.sh pack-deploy local` → `up` |
| `production` | Linux + docker，`docker_pool` | `gateway.sh up --release release-vX.Y.Z`（仅 CI 镜像） |

在 **仓库根目录** `.env` 里至少保证：

| 变量 | 作用 |
| --- | --- |
| `CLAW_DEPLOY_PROFILE` | `local` 或 `production`（可按 OS 自动推断） |
| `CLAW_CLUSTER_ID` | 仓库根 `.env` **必填**；集群标识（Admin 只读展示） |
| `CLAW_GATEWAY_DATABASE_URL` | 网关 PG；与 `CLAW_CLUSTER_ID` 一起参与 `clusterHash` 校验（见 `docs/claw-tap-cluster-identity.md`） |
| `CLAUDE_TAP_MODE` | 本地建议 **`source`**（`../claude-tap` 可编辑安装）；**`native`/PyPI 0.0.7** 的 hash 算法与网关不一致会报 `clusterHash mismatch` |
| Admin → 全局推理（PG） | **clawTap** 端点、活跃 LLM 模型/API Key；solve 时 gateway 经 pool `Exec -e` 注入 worker（`OPENAI_BASE_URL` = clawTap） |
| `CLAW_LLM_PROXY` / `CLAW_TAP_PROXY_URL` | 仅影响本机 **tap 侧车** 或 compose 里 tap 容器地址；**不**再决定 worker 是否绕过 clawTap |
| `gateway.sh up --release <tag>` | `GATEWAY_IMAGE` 与 **`CLAW_DOCKER_IMAGE`** 同 tag（`claw-code`→`claw-gateway-worker`）；勿在根 `.env` 写死 `:local` worker |
| `GATEWAY_HOST_PORT` | 宿主机端口，默认 `8088` |
| `GATEWAY_PLAYGROUND_HOST_PORT` | solve_async / 项目管理 UI，默认 `18765`（compose 服务 `gateway-playground`） |
| `PLAYGROUND_PUBLIC_GATEWAY_BASE` | 浏览器里 playground 默认网关，应与 `GATEWAY_HOST_PORT` 一致，如 `http://127.0.0.1:8088` |
| `PLAYGROUND_ADMIN_USER` / `PLAYGROUND_ADMIN_PASSWORD` | `/admin` 登录账号密码（默认 `admin` / `sunmi123`） |
| `CLAW_PODMAN_IMAGE` / `CLAW_DOCKER_IMAGE` | worker 镜像名（与 `CLAW_SOLVE_ISOLATION` 前缀一致） |
| `CLAW_GATEWAY_DATABASE_URL` | 必填（网关进程）；compose 内网关连 **`postgres:5432`**；宿主机映射默认 **`127.0.0.1:5433`**（`CLAW_GATEWAY_PG_HOST_PORT`，避开 sqlbot 常用 5432） |
| `project_config`（PG） | **必填（业务）**：规则 / MCP / skills / `CLAUDE.md` 在 Admin 或 API 写入 DB，网关物化到 `ds_<id>/home` |
| `git_sync_json`（PG，每 ds） | 可选：每项目单向 push 的 `gitUrl` / `gitRef` / token（见 `docs/project-config-model.md`） |
| `CLAW_PROJECTS_GIT_AUTHOR` | 可选：gitSync 未填 author 时的默认 commit 作者 |

`solve` 始终走 **容器池**（`podman_pool` 或 `docker_pool`）；未设置 `CLAW_SOLVE_ISOLATION` 时与 compose 默认一致为 **`podman_pool`**。

### 1.2 镜像

**本地开发（macOS）**：**首次** `podman run` 编译会慢（拉依赖，和 Rust 有关，不是网关逻辑慢）；**第二次起** 卷 `claw-cargo-registry` 缓存后明显变快。镜像打包只做 COPY（秒级）。`.env` 保留 `CLAW_USE_CN_CRATES_MIRROR=1`。

**Linux / CI**：镜像内完整编译，用同一 `gateway.sh build`（非 Darwin 路径）。

### 1.3 启动与检查

```bash
./deploy/stack/gateway.sh up
```

`gateway.sh up`（`lib/up.sh`）会：

- 生成 `deploy/stack/.claw-pool-workspace.env`（其中 **`CLAW_POOL_WORK_ROOT_HOST=/var/lib/claw/workspace`**，与容器内工作目录一致；不要在容器场景下写 macOS `/Users/...`）。
- 合并 **`podman-compose.pool-rpc.yml`**：compose 内 **`claw-pool-daemon`（TCP）**，网关只连 RPC；**不再支持**在网关容器内挂 Podman API socket 起 worker。
- **`claw_compose`**：按 **`CLAW_CONTAINER_RUNTIME`** 调用 **`docker compose`** 或 **`podman compose`**（`podman` 时若装了 **`podman-compose`** 会用作后端，减轻 macOS 混用问题）。
- 使用 **`up -d --force-recreate`**，避免只改 env 文件却沿用旧容器环境。
- **启动硬门禁（必过）**：preflight 会递归校验 `CLAW_POOL_WORK_ROOT_BIND_SRC`（默认 `deploy/stack/claw-workspace`）下 **`ds_*` 等业务目录** 的 owner 是否为 `CLAW_WORKER_UID:CLAW_WORKER_GID`（默认 `1000:1000`）。**跳过** `.claw-pool-slot/`（pool 做 `mount --make-rshared` 时可能短暂为 root，与 session 目录不是同一类）。维护：`sudo chown -R 1000:1000 ./deploy/stack/claw-workspace/ds_*`；`up --release` 会删旧 `.claw-pool-slot`。

检查：

```bash
curl -sS "http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/healthz"
# 可选：async 调试页 + /admin（与 gateway 同 up/down）
curl -sS "http://127.0.0.1:${GATEWAY_PLAYGROUND_HOST_PORT:-18765}/"
# 与当前 CLAW_CONTAINER_RUNTIME 一致（auto 时与 build/up 相同）：
podman ps   # 或  docker ps
```

`/healthz` 里 **`"containerPool": true`** 表示网关已加载池句柄（当前实现下恒为 true）。池化正常时，宿主机上还能看到 **`claw-worker-*`** 池内 worker（旧版本曾用 `claw-gw-*`，清理脚本仍会顺带删掉）。

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
# local 开发：侧车 tap（若 Admin 指向本机 tap）
./deploy/stack/gateway.sh tap-up
./deploy/stack/gateway.sh tap-down
```

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

远程 Docker / `docker_pool` 与 env 前缀对照仍见文末表格；细节设计见 `docs/http-gateway-container-pool.md`。

---

## 3. 常见问题（短）

| 现象 | 处理 |
| --- | --- |
| `podman ps` 看不到网关 | 可能已退出：`podman ps -a \| grep claw-gateway-rs`，看 `podman logs claw-gateway-rs` |
| 只有 `claw-gateway-rs` 没有 `claw-worker-*` | 是否打了 **worker 镜像**；**`claw-pool-daemon` 日志**是否 `spawn docker: No such file or directory`（`docker_pool` 要求 **gateway 镜像内带 `docker.io`**，见 `Containerfile.gateway-rs`；`release-v1.2.3` 及更早无此包须换新 tag）；`docker logs claw-pool-daemon`；`CLAW_POOL_DAEMON_TCP_HOST=claw-pool-daemon`（勿用 `host.docker.internal` 指 pool） |
| `ensure_warm_failed` / `mount --make-rshared … permission denied`（或 `must be superuser`） | **`docker_pool` 须在能改宿主机 mount 的进程里跑 `mount(8)`**：Linux 默认 **compose `claw-pool-daemon` 需 `privileged: true`**（见 `podman-compose.pool-rpc.yml`）；宿主机 daemon 须 **root**。非 privileged 容器内即使用 root 也会 `permission denied`。勿仅靠 `CLAW_POOL_HOST_DAEMON=1` + 普通用户 |
| preflight 让 `chown 1000:1000` 全树，pool 仍报 mount 权限 | **两件事**：session/`ds_*` 要 1000；**mount 要 privileged pool**（见上条）。**不要**指望把整个 workspace chown 后 pool 就能 mount；`.claw-pool-slot` 已从 preflight 排除，`release up` 会 `rm -rf` 旧 slot 树 |
| solve 报 `session workspace ownership…` / 容器内 `Cannot connect to the Docker daemon …` | **已改为**：配置了 **`CLAW_POOL_RPC_HOST_WORK_ROOT`** 时，`prepare_gateway_session` 通过 **pool RPC** 让 **`claw-pool-daemon`** 做 session `chown`（网关容器**不必**挂 `docker.sock`）。请 **网关 + pool daemon 同版本升级**（含 RPC `chown_session_host`）。若未配 `CLAW_POOL_RPC_HOST_WORK_ROOT` 仍走网关内 `docker`/`podman` 兜底，则需引擎 sock |
| 启动报 canonicalize `/Users/...` | 容器内不能拿 macOS 路径当 `CLAW_POOL_WORK_ROOT_HOST`；用 **`./deploy/stack/gateway.sh up`** 生成 env（`CLAW_POOL_WORK_ROOT_HOST=/var/lib/claw/workspace`） |
| 改 `.env` 不生效 | 必须用 **`./deploy/stack/gateway.sh up`**（带 `--force-recreate`），不要指望无重建的 `up` |
| 改了 `rust/` 里 worker（`claw`）或网关逻辑，solve 仍像旧的 | **`./deploy/stack/gateway.sh build`** 会**同时**重建 **`claw-gateway-rs`** 与 **`claw-gateway-worker`**；只 `up` 不 `build` 会继续用旧镜像 |
| `http://localhost:3000/?session=…` 没有预期内容 | 见上文 **Live Viewer**：stock tap **不解析** `session` query；且须有经 **tap 代理端口**（`CLAUDE_TAP_PORT`，默认 8080）的 **OpenAI 兼容 API** 流量写入当前 `trace_*.jsonl` 后 Live 才有数据；仅打网关 **`/healthz`** 不会进 tap trace |
| solve 成功但泳道无 worker 段 / claw-tap session 错位 | Phase 2 须 **`bind-propagation=rslave`** + `guest/` **rshared**（见 `docs/http-gateway-container-pool.md` §2.2.1）；`podman inspect claw-worker-*` 应见 `prop=rslave`；acquire 后 `podman exec … cat /claw_host_root/gateway-solve-task.json` 的 `sessionId` 须与当前 task 一致 |

联通性脚本：`./deploy/stack/gateway.sh check`。

简易池压测（30s、每秒 3 次 `solve_async`，并采样 **`claw-worker-*`** 数量）：`./deploy/stack/gateway.sh bench 'http://127.0.0.1:8088'`。

---

## 4. 构建说明摘录

- 基础镜像仓库：默认 `CONTAINER_BASE_REGISTRY=docker.1ms.run`（`.env`）；`CLAW_USE_DOCKER_IO=1` 时用 `docker.io`。
- 国内可选：`CLAW_USE_CN_RUST_MIRROR=1`（仅影响 **首次** rustup 相关层；镜像已改为用 base 镜像自带 **stable**，不再 `rustup install nightly`，避免 nightly 每天更新导致反复下 `rust-std`）。宿主 `rust/.cargo/config.toml.example` 拷贝见 `.env.example` 注释。

---

## 5. Local Podman vs remote Docker（对照）

| 场景 | `CLAW_SOLVE_ISOLATION` | 运行时 CLI | 环境前缀 | 与网关的衔接 |
| --- | --- | --- | --- | --- |
| 本仓库 compose（默认） | `podman_pool` | `podman`（宿主机 `claw-pool-daemon`） | `CLAW_PODMAN_*` | 合并 **`podman-compose.pool-rpc.yml`**；默认 `CLAW_POOL_DAEMON_TCP_HOST=host.containers.internal`；session **特权 chown** 由 **pool RPC** 在 daemon 执行 |
| 线上 Docker（推荐与默认脚本对齐） | `docker_pool` | `docker`（宿主机或旁路容器里的 daemon） | `CLAW_DOCKER_*` | 同上，但 `.env` 改 `CLAW_POOL_DAEMON_TCP_HOST=host.docker.internal`（Linux 已用 `podman-compose.pool-rpc.yml` 的 `extra_hosts`） |
| 网关内嵌池（备选） | `docker_pool` / `podman_pool` | `docker` / `podman` 在**网关容器**内 | 同上 | **不设** `CLAW_POOL_DAEMON_TCP`：走进程内 `DockerPoolManager`；需 sock 挂载 + 镜像带对应 CLI（`Containerfile.gateway-rs`：`podman` + `docker.io`） |

**会话与磁盘**：每次 solve 仍绑定 **一个 worker 容器 + 会话工作区**（目录名由网关分配并记入 PostgreSQL）；**续聊**靠 body `sessionId` + 会话库，见 `docs/http-gateway-rs-api.md`。池化细节仍见 `docs/http-gateway-container-pool.md` §2。本仓库 **`gateway.sh up` compose 栈**只使用 **宿主机 `claw-pool-daemon` + TCP RPC**；若运行时不存在 `CLAW_POOL_DAEMON_TCP`，网关会退回 **进程内 `PoolManager`**（下表「网关内嵌池」一行）。

线上只有 Docker 时 **`CLAW_CONTAINER_RUNTIME` 可不写**（`auto` 会选 docker）；仍用同一套 `deploy/stack/podman-compose*.yml`（文件名历史原因）。

Worker 镜像名：`CLAW_PODMAN_IMAGE` 与 `CLAW_DOCKER_IMAGE` 二选一；池大小等同名前缀变量，见 `docs/http-gateway-container-pool.md`。

---

## 6. 环境变量：只维护根 `.env`

网关 compose **只加载仓库根**的 `.env`。每 `dsId` 的工具白名单在 **Admin → Tools** / `project_config.allowed_tools_json`（PG），**不**读 `CLAW_ALLOWED_TOOLS`。`deploy/stack/` 下由 **`gateway.sh up`** / **`lib/compose-include.sh`** 生成的 `*.env` 为**中间物**，每次脚本会覆盖，**不要手改**。

- **全量 env 清单 + 双模式**：`docs/env-config.md`
- **人手 vs 生成物**、禁止 `deploy/stack/.env`：`docs/env-files.md`
