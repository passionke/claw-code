# Podman：网关（http-gateway-rs）稳定部署说明

Author: kejiqing

**稳定做法只有一条**：在仓库根目录准备好 `.env`，打好镜像，用 **`./deploy/podman/up.sh`** 起服务。不要用「手写一长串 compose / 只挂单个 compose 文件 / 在容器里配 macOS 的 `/Users/...` 路径」这类容易翻车的玩法。

**单入口（推荐）**：只记一个命令 **`./deploy/podman/gateway.sh`**。它封装了 `build/up/down/check/tap`，常用：

```bash
./deploy/podman/gateway.sh build
./deploy/podman/gateway.sh up
./deploy/podman/gateway.sh check
./deploy/podman/gateway.sh ps
```

其中 `./deploy/podman/gateway.sh build` 会连续构建 **gateway + worker**，避免“网关镜像新、worker 镜像旧”导致续聊行为不一致。

**线上部署（与 GitHub 打包一致）**：打 tag `release-*` 触发 [`.github/workflows/claw-code-image.yaml`](../../.github/workflows/claw-code-image.yaml)，镜像推到 **`ghcr.io/<owner>/claw-code`** 与 **`ghcr.io/<owner>/claw-gateway-worker`**。服务器上 `.env` 只填 **`GATEWAY_IMAGE`** / **`CLAW_DOCKER_IMAGE`**（或 Podman 前缀）为上述 tag，**不要**在服务器跑 `build.sh`。宿主机池守护进程与网关**同版本**：网关镜像内已带 **`/usr/local/bin/claw-pool-daemon`**，执行一次 **`sudo ./deploy/podman/install-pool-daemon-from-image.sh`** 安装到本机，并设 **`CLAW_POOL_DAEMON_SKIP_BUILD=1`**、**`CLAW_POOL_DAEMON_BIN`**（见 `.env.example`），再 **`./deploy/podman/up.sh`**。校验构建见 [`gateway-image-ci.yml`](../../.github/workflows/gateway-image-ci.yml)。

**同一套脚本、本地与线上共用**：`build.sh` / `up.sh` / `down.sh` / tap / `bench-pool-30s.sh` 通过 **`CLAW_CONTAINER_RUNTIME`** 选 CLI——默认 **`auto`**（PATH 里**有 podman 先用 podman**，否则 **docker**）。线上常只有 docker，无需改 `.env`；本机有 podman 也会自动走 podman。只有两台都装了且必须指定时，才设 **`CLAW_CONTAINER_RUNTIME=podman`** 或 **`docker`**。

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
| `PODMAN_HOST_SOCK` | 仅 **`CLAW_POOL_HOST_DAEMON=0`** 的 legacy 路径需要（网关内 `podman` 连宿主 API） |
| `OPENAI_API_KEY` / `OPENAI_BASE_URL` | 模型 |
| `GATEWAY_HOST_PORT` | 宿主机端口，默认 `8088` |
| `CLAW_PODMAN_IMAGE` / `CLAW_DOCKER_IMAGE` | worker 镜像名（与 `CLAW_SOLVE_ISOLATION` 前缀一致） |
| `CLAW_GATEWAY_SESSION_DB` | 可选；不设时 compose 默认已挂 **`./claw-gateway-sessions`** 对应容器内路径（见 `podman-compose.yml`） |
| `CLAW_PROJECTS_GIT_URL` | **必填**：`ds_*` 项目镜像仓库（SSH 或 HTTPS），代码无默认值 |
| `CLAW_PROJECTS_GIT_BRANCH` | **必填**：例如 `main` |
| `CLAW_PROJECTS_GIT_AUTHOR` | **必填**：`git commit --author`，例如 `kejiqing <kejiqing@local>` |
| `CLAW_PROJECTS_GIT_TOKEN` | 可选；若 URL 为无凭据的 `https://...`（不含 `user@`）则**必填**（Fine-grained PAT 等），否则网关启动即退出 |

可选：`CLAW_SOLVE_ISOLATION=inprocess` 关闭容器池（只有一个网关容器、无 `claw-gw-*` worker；适合只想快速试 HTTP）。

### 1.2 镜像

**本地开发**才在仓库内构建：

```bash
./deploy/podman/build.sh
```

容器池还需要 worker 镜像（在**仓库根**执行）：

```bash
set -a && [ -f .env ] && . ./.env && set +a
. ./deploy/podman/compose-include.sh
REG="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
CLI="$(claw_container_runtime_cli)"
"${CLI}" build \
  --build-arg "RUST_BASE_IMAGE=${REG}/library/rust:1.88-bookworm" \
  --build-arg "DEBIAN_BASE_IMAGE=${REG}/library/debian:bookworm-slim" \
  -f deploy/podman/Containerfile.gateway-worker \
  -t claw-gateway-worker:local .
```

### 1.3 启动与检查

```bash
./deploy/podman/up.sh
```

`up.sh` 会：

- 生成 `deploy/podman/.claw-pool-workspace.env`（其中 **`CLAW_POOL_WORK_ROOT_HOST=/var/lib/claw/workspace`**，与容器内工作目录一致；不要在容器场景下写 macOS `/Users/...`）。
- 默认 **`CLAW_POOL_HOST_DAEMON=1`**：合并 **`podman-compose.pool-rpc.yml`**，宿主机起 `claw-pool-daemon`（TCP），网关只连 RPC。
- **`CLAW_POOL_HOST_DAEMON=0`**：合并 **`podman-compose.podman-api.yml`**（宿主 Podman API socket 挂进网关）。
- **`claw_compose`**：按 **`CLAW_CONTAINER_RUNTIME`** 调用 **`docker compose`** 或 **`podman compose`**（`podman` 时若装了 **`podman-compose`** 会用作后端，减轻 macOS 混用问题）。
- 使用 **`up -d --force-recreate`**，避免只改 env 文件却沿用旧容器环境。

检查：

```bash
curl -sS "http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}/healthz"
# 与当前 CLAW_CONTAINER_RUNTIME 一致（auto 时与 build/up 相同）：
podman ps   # 或  docker ps
```

`/healthz` 里应有 `"containerPool": true`（池模式）或 `false`（`inprocess`）。池化正常时，宿主机上还能看到 `claw-gw-*` worker。

### 1.4 停止

```bash
./deploy/podman/down.sh
```

### 1.5 带 claude-tap 的一体脚本

```bash
./deploy/podman/start-with-tap.sh
./deploy/podman/stop-with-tap.sh
```

`claude-tap` 在宿主机跑，只做 API 代理/抓包，不是 MCP。

---

## 2. 设计约定（知道这些就够排障）

- **网关容器内**的池化路径必须是 Linux 里存在的路径；compose 把 `deploy/podman/claw-workspace` 挂到 **`/var/lib/claw/workspace`**，池绑定根目录与之一致。
- **`CLAW_GATEWAY_SESSION_DB`（会话表 SQLite）**：`podman-compose.yml` 默认把库文件放在 **`/var/lib/claw/gateway-sessions/gateway-sessions.sqlite`**，并 **`./claw-gateway-sessions` → `/var/lib/claw/gateway-sessions`** 绑定到宿主机，**容器重建不丢** `sessionId` ↔ 工作区路径映射。不设该变量时网关会把库落在 **`CLAW_WORK_ROOT/gateway-sessions.sqlite`**（与 workspace 同卷时同样持久；compose 显式卷是为了路径清晰、便于单独备份）。生产也可把 `CLAW_GATEWAY_SESSION_DB` 指到任意**已挂载**的绝对路径（见根目录 `.env.example`）。`/healthz` 的 **`sessionDbPath`** 可核对当前使用的文件路径。
- **Podman API**：`podman-compose.podman-api.yml` 挂载的是 **socket 所在目录**（不是只挂单个 `.sock` 文件），避免 macOS 上 `statfs … operation not supported`。
- **Compose 后端**：需要 `podman-compose` 时 `brew install podman-compose`；勿假定 `podman compose` 一定走 Docker 的 compose。

远程 Docker / `docker_pool` 与 env 前缀对照仍见文末表格；细节设计见 `docs/http-gateway-container-pool.md`。

---

## 3. 常见问题（短）

| 现象 | 处理 |
| --- | --- |
| `podman ps` 看不到网关 | 可能已退出：`podman ps -a \| grep claw-gateway-rs`，看 `podman logs claw-gateway-rs` |
| 只有 `claw-gateway-rs` 没有 `claw-gw-*` | 是否打了 **worker 镜像**；网关镜像是否含 `podman`；`PODMAN_HOST_SOCK` 是否对。 **macOS**：网关跑在容器里时，挂载的 Podman API socket 往往在容器内 **无法 dial**（`connection refused` / `operation not supported`），worker 起不来——`/healthz` 仍可能 `containerPool: true`（表示配置了池）。要验证池是否真在工作：`podman exec claw-gateway-rs sh -lc 'podman --url "$CONTAINER_HOST" version'`。失败时请用 **`CLAW_SOLVE_ISOLATION=inprocess`**，或把网关跑在 **Linux 宿主机**（或能直连 Podman API 的环境） |
| 启动报 canonicalize `/Users/...` | 容器内不能拿 macOS 路径当 `CLAW_POOL_WORK_ROOT_HOST`；用 **`./deploy/podman/up.sh`** 生成 env，或设 `inprocess` |
| 改 `.env` 不生效 | 必须用 **`up.sh`**（带 `--force-recreate`），不要指望无重建的 `up` |

联通性脚本：`./deploy/podman/check-connectivity.sh`。

简易池压测（30s、每秒 3 次 `solve_async`，并采样 `claw-gw-*` 数量）：`./deploy/podman/bench-pool-30s.sh 'http://127.0.0.1:8088'`。

---

## 4. 构建说明摘录

- 基础镜像仓库：默认 `CONTAINER_BASE_REGISTRY=docker.1ms.run`（`.env`）；`CLAW_USE_DOCKER_IO=1` 时用 `docker.io`。
- 国内可选：`CLAW_USE_CN_RUST_MIRROR=1`，以及宿主 `rust/.cargo/config.toml.example` 拷贝（见 `.env.example` 注释）。

---

## 5. Local Podman vs remote Docker（对照）

| 场景 | `CLAW_SOLVE_ISOLATION` | 运行时 CLI | 环境前缀 | 与网关的衔接 |
| --- | --- | --- | --- | --- |
| 本仓库 compose（默认） | `podman_pool` | `podman`（宿主机 `claw-pool-daemon`） | `CLAW_PODMAN_*` | `CLAW_POOL_HOST_DAEMON=1` → TCP RPC；默认 `CLAW_POOL_DAEMON_TCP_HOST=host.containers.internal` |
| 线上 Docker（推荐与默认脚本对齐） | `docker_pool` | `docker`（宿主机或旁路容器里的 daemon） | `CLAW_DOCKER_*` | 同上，但 `.env` 改 `CLAW_POOL_DAEMON_TCP_HOST=host.docker.internal`（Linux 已用 `podman-compose.pool-rpc.yml` 的 `extra_hosts`）或填 compose 服务名 |
| 网关内嵌池（备选） | `docker_pool` / `podman_pool` | `docker` / `podman` 在**网关容器**内 | 同上 | **不设** `CLAW_POOL_DAEMON_TCP`：走进程内 `DockerPoolManager`；需 sock 挂载 + 镜像带对应 CLI（当前 `Containerfile.gateway-rs` 仅装 `podman`） |

**会话与磁盘**：每次 solve 仍绑定 **一个 worker 容器 + 会话工作区**（目录名由网关分配并记入 SQLite）；**续聊**靠 body `sessionId` + 会话库，见 `docs/http-gateway-rs-api.md`。池化细节仍见 `docs/http-gateway-container-pool.md` §2。变的只是「谁执行 `docker run`」：宿主机 daemon（默认脚本）vs 网关进程。

线上只有 Docker 时 **`CLAW_CONTAINER_RUNTIME` 可不写**（`auto` 会选 docker）；仍用同一套 `deploy/podman/podman-compose*.yml`（文件名历史原因）。

Worker 镜像名：`CLAW_PODMAN_IMAGE` 与 `CLAW_DOCKER_IMAGE` 二选一；池大小等同名前缀变量，见 `docs/http-gateway-container-pool.md`。

---

## 6. `gateway-allowlist.env`

`podman-compose.yml` 在根 `.env` 之后加载 `deploy/podman/gateway-allowlist.env`。默认**不再**写入 `CLAW_ALLOWED_TOOLS`：网关侧空列表表示**允许全部内置工具**（含 `Skill` 等）。若需收紧白名单，在此文件或根 `.env` 中设置逗号分隔的工具名。
