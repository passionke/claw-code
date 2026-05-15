# Podman：网关（http-gateway-rs）稳定部署说明

Author: kejiqing

**稳定做法只有一条**：在仓库根目录准备好 `.env`，打好镜像，用 **`./deploy/stack/gateway.sh up`** 起服务。不要用「手写一长串 compose / 只挂单个 compose 文件 / 在容器里配 macOS 的 `/Users/...` 路径」这类容易翻车的玩法。

**单入口（推荐）**：只记一个命令 **`./deploy/stack/gateway.sh`**。实现脚本在 **`deploy/stack/lib/`**（由 gateway 调用；一般不要直接跑）。常用：

```bash
./deploy/stack/gateway.sh build
./deploy/stack/gateway.sh up
./deploy/stack/gateway.sh check
./deploy/stack/gateway.sh ps
```

其中 `./deploy/stack/gateway.sh build` 通过 **`lib/build.sh` 一次串联**：先 **`Containerfile.gateway-rs`**（`http-gateway-rs` + 宿主机用的 **`claw-pool-daemon`**），再 **`Containerfile.gateway-worker`**（池内 **`claw`**），共用同一套 **`rust/`** 与 base / rustup build-arg，避免「网关新、worker 旧」。

**线上部署（与 GitHub Actions 一致）**：打 tag `release-*` 触发 [`.github/workflows/claw-code-image.yaml`](../../.github/workflows/claw-code-image.yaml)，镜像推到 **`ghcr.io/<owner>/claw-code`** 与 **`ghcr.io/<owner>/claw-gateway-worker`**。服务器上 `.env` 填 **`GATEWAY_IMAGE`** / **`CLAW_DOCKER_IMAGE`**（或 Podman 前缀）为上述 tag，**不要**在服务器跑 **`./deploy/stack/gateway.sh build`**（用预构建镜像即可）。**`./deploy/stack/gateway.sh up`** 在起池前会**始终**从 **`GATEWAY_IMAGE`** 抽出 **`claw-pool-daemon`** 安装到 **`CLAW_POOL_DAEMON_BIN`**（未设置时默认为仓库内 **`rust/target/release/claw-pool-daemon`**；写到 **`/usr/local/bin/...`** 时需对 `install` 命令有写权限，常见为 `sudo` 跑 up 或事先 `sudo install …`），保证宿主机 daemon 与网关镜像**同版本**。也可单独执行 **`./deploy/stack/lib/install-pool-daemon-from-image.sh`**。校验构建见 [`gateway-image-ci.yml`](../../.github/workflows/gateway-image-ci.yml)。

**国内拉 GHCR 很慢**：同一 tag 可由 [`.github/workflows/claw-code-acr.yaml`](../../.github/workflows/claw-code-acr.yaml) 推到 **ACR**（`myregistry.azurecr.io/claw-code:<tag>` 等）。在仓库根 **`.env`** 里设 **`CLAW_IMAGE_PREFIX=myregistry.azurecr.io`**（不要 `https://`），并先 **`podman login myregistry.azurecr.io`**（或 docker login），再执行 **`./deploy/stack/gateway.sh up --release release-v…`**，脚本会从 ACR 拉取而不是 `ghcr.io`。与 **`CLAW_IMAGE_PREFIX`** 等价的老变量名是 **`CLAW_GHCR_PREFIX`**。

**GHCR 握手超时 / 服务器拉不下来**：在能稳定访问镜像源的环境执行 **`./deploy/stack/lib/ship-release-tar-to-remote.sh release-v1.0.25`**（默认推到 **`admin@192.168.9.252` 的 `~`**）；本机若也拉不动 GHCR，可先设 **`CLAW_SHIP_REGISTRY_PREFIX=…`** 指向已能拉到的 ACR 前缀再跑脚本。远端 **`podman load -i`** / **`docker load -i`** 后，再在服务器上 **`CLAW_IMAGE_PREFIX=… ./deploy/stack/gateway.sh up --release …`**。

**同一套脚本、本地与线上共用**：`deploy/stack/lib/` 下的 `build.sh` / `up.sh` / `down.sh` / tap / `bench-pool-30s.sh` 由 **`gateway.sh`** 调用；它们通过 **`CLAW_CONTAINER_RUNTIME`** 选 CLI——默认 **`auto`**（PATH 里**有 podman 先用 podman**，否则 **docker**）。线上常只有 docker，无需改 `.env`；本机有 podman 也会自动走 podman。只有两台都装了且必须指定时，才设 **`CLAW_CONTAINER_RUNTIME=podman`** 或 **`docker`**。

更全的接口与本地调试见：`docs/http-gateway-rs-quickstart.md`（第二节已与本文对齐）。

---

## 1. 稳定路径（按顺序做）

### 1.1 环境

```bash
cp .env.example .env
```

在 **仓库根目录** `.env` 里至少保证：

| 变量 | 作用 |
| --- | --- |
| `CLAW_CONTAINER_RUNTIME` | `auto`（默认）或 `podman` / `docker`；与线上/本地无关，按需覆盖 |
| `OPENAI_API_KEY` / `OPENAI_BASE_URL` | 模型 |
| `GATEWAY_HOST_PORT` | 宿主机端口，默认 `8088` |
| `CLAW_PODMAN_IMAGE` / `CLAW_DOCKER_IMAGE` | worker 镜像名（与 `CLAW_SOLVE_ISOLATION` 前缀一致） |
| `CLAW_GATEWAY_SESSION_DB` | 可选；不设时 compose 默认已挂 **`./claw-gateway-sessions`** 对应容器内路径（见 `podman-compose.yml`） |
| `CLAW_PROJECTS_GIT_URL` | **必填**：`ds_*` 项目镜像仓库（SSH 或 HTTPS），代码无默认值 |
| `CLAW_PROJECTS_GIT_BRANCH` | **必填**：例如 `main` |
| `CLAW_PROJECTS_GIT_AUTHOR` | **必填**：`git commit --author`，例如 `kejiqing <kejiqing@local>` |
| `CLAW_PROJECTS_GIT_TOKEN` | 可选；若 URL 为无凭据的 `https://...`（不含 `user@`）则**必填**（Fine-grained PAT 等），否则网关启动即退出 |

`solve` 始终走 **容器池**（`podman_pool` 或 `docker_pool`）；未设置 `CLAW_SOLVE_ISOLATION` 时与 compose 默认一致为 **`podman_pool`**。

### 1.2 镜像

**本地开发**：改 `rust/` 后**一次**编网关 + worker（同一套 base / rustup 参数，避免只新网关、worker 还是旧的）：

```bash
./deploy/stack/gateway.sh build
```

等价：`./deploy/stack/lib/build.sh`（可选 tag 参数，默认 `local` → `claw-gateway-rs:local` 与 `claw-gateway-worker:local`）。**在 macOS 上**同一步还会 **`cargo build --release` 宿主 `claw-pool-daemon`**（池逻辑改在 `rust/` 里时，只 `restart` 不够，须先 `build` 或手编 daemon，否则宿主机仍跑旧二进制）。

### 1.3 启动与检查

```bash
./deploy/stack/gateway.sh up
```

`gateway.sh up`（`lib/up.sh`）会：

- 生成 `deploy/stack/.claw-pool-workspace.env`（其中 **`CLAW_POOL_WORK_ROOT_HOST=/var/lib/claw/workspace`**，与容器内工作目录一致；不要在容器场景下写 macOS `/Users/...`）。
- 合并 **`podman-compose.pool-rpc.yml`**：宿主机起 **`claw-pool-daemon`（TCP）**，网关只连 RPC；**不再支持**在网关容器内挂 Podman API socket 起 worker。
- **`claw_compose`**：按 **`CLAW_CONTAINER_RUNTIME`** 调用 **`docker compose`** 或 **`podman compose`**（`podman` 时若装了 **`podman-compose`** 会用作后端，减轻 macOS 混用问题）。
- 使用 **`up -d --force-recreate`**，避免只改 env 文件却沿用旧容器环境。

检查：

```bash
curl -sS "http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/healthz"
# 与当前 CLAW_CONTAINER_RUNTIME 一致（auto 时与 build/up 相同）：
podman ps   # 或  docker ps
```

`/healthz` 里 **`"containerPool": true`** 表示网关已加载池句柄（当前实现下恒为 true）。池化正常时，宿主机上还能看到 **`claw-worker-*`** 池内 worker（旧版本曾用 `claw-gw-*`，清理脚本仍会顺带删掉）。

### 1.4 停止

```bash
./deploy/stack/gateway.sh down
```

### 1.5 带 claude-tap 的一体脚本

```bash
./deploy/stack/gateway.sh tap-up
./deploy/stack/gateway.sh tap-down
```

`claude-tap` 在宿主机跑，只做 API 代理/抓包，不是 MCP。

**Live Viewer（`CLAUDE_TAP_LIVE_PORT`，默认 3000）与 `?session=`**（已对照上游 **`claude-tap` 0.1.52** 安装树：`claude_tap/live.py` 的 `GET /` 不读取 query；`viewer.html` 内也无对 `location.search` / `URLSearchParams` 的解析）：

- **`http://127.0.0.1:<live_port>/` 只展示当前这次 tap 进程绑定的 `trace_*.jsonl` 实时流**（`cli.py` 在**启动 tap 时的当前工作目录**下写 **`.traces/<日期>/trace_<HHMMSS>.jsonl`** 并交给 `LiveViewerServer`；常见为仓库根 `./.traces/`，取决于你从哪个目录执行 `gateway.sh tap-up`）。
- **URL 里的 `?session=…` 不会被 Live Viewer 用来筛选或定位 trace**；浏览器会把查询串发给服务器，但 tap 侧实现忽略之，因此「随便填一个 id（含网关 `/healthz` 返回的 `claw-session-id`）」**页面行为与不带 query 相同**，并不是「两个系统 id 没对齐才空白」这一种原因。
- **网关**的 `claw-session-id` / `/v1/solve` 的 `sessionId` 属于 **`http-gateway-rs` 与会话库**，与 tap 的 trace 文件命名**无契约绑定**；要对齐排障应分别看：网关 **`/healthz`** / 日志；tap **`.traces/` 目录**或 Viewer 里按日期/文件选的记录。

---

## 2. 设计约定（知道这些就够排障）

- **网关容器内**的池化路径必须是 Linux 里存在的路径；compose 把 `deploy/stack/claw-workspace` 挂到 **`/var/lib/claw/workspace`**，池绑定根目录与之一致。
- **`CLAW_GATEWAY_SESSION_DB`（会话表 SQLite）**：`podman-compose.yml` 默认把库文件放在 **`/var/lib/claw/gateway-sessions/gateway-sessions.sqlite`**，并 **`./claw-gateway-sessions` → `/var/lib/claw/gateway-sessions`** 绑定到宿主机，**容器重建不丢** `sessionId` ↔ 工作区路径映射。不设该变量时网关会把库落在 **`CLAW_WORK_ROOT/gateway-sessions.sqlite`**（与 workspace 同卷时同样持久；compose 显式卷是为了路径清晰、便于单独备份）。生产也可把 `CLAW_GATEWAY_SESSION_DB` 指到任意**已挂载**的绝对路径（见根目录 `.env.example`）。`/healthz` 的 **`sessionDbPath`** 可核对当前使用的文件路径。
- **Compose 后端**：需要 `podman-compose` 时 `brew install podman-compose`；勿假定 `podman compose` 一定走 Docker 的 compose。

远程 Docker / `docker_pool` 与 env 前缀对照仍见文末表格；细节设计见 `docs/http-gateway-container-pool.md`。

---

## 3. 常见问题（短）

| 现象 | 处理 |
| --- | --- |
| `podman ps` 看不到网关 | 可能已退出：`podman ps -a \| grep claw-gateway-rs`，看 `podman logs claw-gateway-rs` |
| 只有 `claw-gateway-rs` 没有 `claw-worker-*` | 是否打了 **worker 镜像**；宿主机 **`claw-pool-daemon`** 是否在跑（`gateway.sh up` 会起）；网关容器能否 **`CLAW_POOL_DAEMON_TCP`** 连到宿主上的 daemon（`CLAW_POOL_DAEMON_TCP_HOST` / 端口）；`CLAW_SOLVE_ISOLATION` 是否为 **`podman_pool`** / **`docker_pool`**（拼写错误会导致网关启动失败） |
| 启动报 canonicalize `/Users/...` | 容器内不能拿 macOS 路径当 `CLAW_POOL_WORK_ROOT_HOST`；用 **`./deploy/stack/gateway.sh up`** 生成 env（`CLAW_POOL_WORK_ROOT_HOST=/var/lib/claw/workspace`） |
| 改 `.env` 不生效 | 必须用 **`./deploy/stack/gateway.sh up`**（带 `--force-recreate`），不要指望无重建的 `up` |
| 改了 `rust/` 里 worker（`claw`）或网关逻辑，solve 仍像旧的 | **`./deploy/stack/gateway.sh build`** 会**同时**重建 **`claw-gateway-rs`** 与 **`claw-gateway-worker`**；只 `up` 不 `build` 会继续用旧镜像 |
| `http://localhost:3000/?session=…` 没有预期内容 | 见上文 **Live Viewer**：stock tap **不解析** `session` query；且须有经 **tap 代理端口**（`CLAUDE_TAP_PORT`，默认 8080）的 **OpenAI 兼容 API** 流量写入当前 `trace_*.jsonl` 后 Live 才有数据；仅打网关 **`/healthz`** 不会进 tap trace |

联通性脚本：`./deploy/stack/gateway.sh check`。

简易池压测（30s、每秒 3 次 `solve_async`，并采样 **`claw-worker-*`** 数量）：`./deploy/stack/gateway.sh bench 'http://127.0.0.1:8088'`。

---

## 4. 构建说明摘录

- 基础镜像仓库：默认 `CONTAINER_BASE_REGISTRY=docker.1ms.run`（`.env`）；`CLAW_USE_DOCKER_IO=1` 时用 `docker.io`。
- 国内可选：`CLAW_USE_CN_RUST_MIRROR=1`，以及宿主 `rust/.cargo/config.toml.example` 拷贝（见 `.env.example` 注释）。

---

## 5. Local Podman vs remote Docker（对照）

| 场景 | `CLAW_SOLVE_ISOLATION` | 运行时 CLI | 环境前缀 | 与网关的衔接 |
| --- | --- | --- | --- | --- |
| 本仓库 compose（默认） | `podman_pool` | `podman`（宿主机 `claw-pool-daemon`） | `CLAW_PODMAN_*` | 合并 **`podman-compose.pool-rpc.yml`**；默认 `CLAW_POOL_DAEMON_TCP_HOST=host.containers.internal` |
| 线上 Docker（推荐与默认脚本对齐） | `docker_pool` | `docker`（宿主机或旁路容器里的 daemon） | `CLAW_DOCKER_*` | 同上，但 `.env` 改 `CLAW_POOL_DAEMON_TCP_HOST=host.docker.internal`（Linux 已用 `podman-compose.pool-rpc.yml` 的 `extra_hosts`）或填 compose 服务名 |
| 网关内嵌池（备选） | `docker_pool` / `podman_pool` | `docker` / `podman` 在**网关容器**内 | 同上 | **不设** `CLAW_POOL_DAEMON_TCP`：走进程内 `DockerPoolManager`；需 sock 挂载 + 镜像带对应 CLI（当前 `Containerfile.gateway-rs` 仅装 `podman`） |

**会话与磁盘**：每次 solve 仍绑定 **一个 worker 容器 + 会话工作区**（目录名由网关分配并记入 SQLite）；**续聊**靠 body `sessionId` + 会话库，见 `docs/http-gateway-rs-api.md`。池化细节仍见 `docs/http-gateway-container-pool.md` §2。本仓库 **`gateway.sh up` compose 栈**只使用 **宿主机 `claw-pool-daemon` + TCP RPC**；若运行时不存在 `CLAW_POOL_DAEMON_TCP`，网关会退回 **进程内 `PoolManager`**（下表「网关内嵌池」一行）。

线上只有 Docker 时 **`CLAW_CONTAINER_RUNTIME` 可不写**（`auto` 会选 docker）；仍用同一套 `deploy/stack/podman-compose*.yml`（文件名历史原因）。

Worker 镜像名：`CLAW_PODMAN_IMAGE` 与 `CLAW_DOCKER_IMAGE` 二选一；池大小等同名前缀变量，见 `docs/http-gateway-container-pool.md`。

---

## 6. 环境变量：只维护根 `.env`

网关 compose **只加载仓库根**的 `.env`。`CLAW_ALLOWED_TOOLS`（solve 工具白名单）只在该文件中配置；不设或留空表示允许全部内置工具。`deploy/stack/` 下由 **`gateway.sh up`** / **`lib/compose-include.sh`** 生成的 `*.env` 为**中间物**，每次脚本会覆盖，**不要手改**。
