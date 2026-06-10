# 本地开发（懒人版）

## 路线方针（与线上一致脚本、不同 `.env`）

| | 本地（本文） | 线上 |
| --- | --- | --- |
| 引擎 | **Podman**（`CLAW_CONTAINER_RUNTIME=podman` 或 `auto`） | **Docker**（见 `deploy/stack/env.production.docker.example`） |
| 镜像 | **本机编译**：`gateway.sh quick` / `pack-deploy` | **仅 CI 镜像**：`gateway.sh up --release …`，服务器不 cargo 编网关 |
| 入口 | 只认 `./deploy/stack/gateway.sh` | 同上 |

不要怕 `pack-deploy` / `quick` 慢；**怕的是**再维护一套 `deploy/podman` 手写 compose。兼容脚本已 **转发** 到 `deploy/stack/lib/`。

## 一条命令

在**仓库根目录**：

```bash
./deploy/stack/gateway.sh quick
```

会做：

1. 构建 **host** `claw-sandbox`（`sandbox/` 或 `gateway.sh build` → `.linux-artifacts`）
2. **`web/gateway-admin`**：`npm ci && vite build` → `dist/`（`lib/build-gateway-admin.sh`）
3. 快速重建 `claw-gateway-playground` 镜像（含 admin dist + solve_async）
4. `pool-reset` → `up` → `check`

## 只改根目录 `.env`（池网络、INTERNAL_*、模型 key 等）

```bash
./deploy/stack/gateway.sh up
```

会 `source .env`、重建 pool worker（新 `podman run --network` / 挂载的 `worker.env`）。**不必**为改 env 单独 `pack-deploy`。排查用 `gateway.sh ps` / `logs`，**不要**手搓 `podman exec` 起栈。

## 改 Rust 网关 / 全量镜像后

```bash
./deploy/stack/gateway.sh pack-deploy
```

## 其它

| 命令 | 作用 |
|------|------|
| `./deploy/stack/gateway.sh playground` | 仅起 host 调试页（会先 `admin-build`） |
| `./deploy/stack/gateway.sh admin-build` | 只构建 React Admin `dist/`（改 `web/gateway-admin/src` 时用） |
| `./deploy/stack/gateway.sh admin-reload` | **仅本地**：`CLAW_GATEWAY_ADMIN_LOCAL_BUILD=1` + Node≥18；线上用 CI `claw-gateway-playground` 镜像，勿在服务器编译 |
| `./deploy/stack/gateway.sh down` | 停 gateway + pool |
| `./deploy/stack/gateway.sh ps` | 看容器 |
| `./deploy/stack/gateway.sh help` | 帮助 |

实现脚本在 `deploy/stack/lib/`；**不要**在 `rust/` 子目录里直接跑 `gateway.sh`（cwd 错误）。

## 磁盘：几十 G 的编译产物怎么清

| 路径 / 资源 | 典型大小 | 清理方式 |
|-------------|----------|----------|
| `rust/target/debug` | 最大 | **`./deploy/stack/gateway.sh clean --debug-only`** |
| `rust/target` 全部 | debug+release | `./deploy/stack/gateway.sh clean` |
| `deploy/stack/.linux-artifacts` | 数百 MB～数 GB | 随 `clean` 一起删 |
| Podman 卷 `claw-cargo-registry` / `claw-cargo-git` | 数 GB | `./deploy/stack/gateway.sh clean --podman-compile-cache` |
| Podman 镜像 `claw-gateway-*` | 可达数十 GB | `./deploy/stack/gateway.sh clean --prune-claw-images`（慎用） |

## 常见坑

- **`zsh: no such file or directory: ./deploy/stack/gateway.sh`** — 先 `cd` 到仓库根 `claw-code`。
- **`clawExitCode=125`** — 先 **`gateway.sh quick`**，再 `podman ps -a \| grep claw-worker` 应为 **Up**。
- **Admin 界面旧 / 缺功能** — 跑 **`gateway.sh admin-reload`**（或 `quick` 重建 playground 镜像）；浏览器 **强制刷新**（Cmd+Shift+R）。仅 `restart` 无效：镜像内 `admin-dist` 是构建时 COPY 的。
- **无 Node/npm** — 依赖仓库里已提交的 `dist/`，或安装 Node 18+ 后再 `gateway.sh admin-build`。

Author: kejiqing
