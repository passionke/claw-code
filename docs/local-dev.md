# 本地开发（懒人版）

Author: kejiqing

## 推荐路径（FC + 外连 PG）

```bash
cp deploy/stack/env.selfhosted-e2b.example .env   # 编辑 CLAW_CLUSTER_ID、FC keys、PG URL
./deploy/stack/gateway.sh quick
```

前提：e2bserver 与 PostgreSQL 已就绪（见 [`architecture-governance.md`](architecture-governance.md)）。

## 一条命令（macOS 全本地 compose）

在**仓库根目录**：

```bash
./deploy/stack/gateway.sh quick
```

会做：

1. **`web/gateway-admin`**：`npm ci && vite build` → `dist/`
2. 快速重建 `claw-gateway-playground` 镜像
3. `up` → `check`（**无** host pool-daemon）

`CLAW_INTERACTIVE_BACKEND` / `CLAW_SOLVE_ISOLATION` 须为 **`fc`**（`env-profile.sh` 默认）。

## 只改根目录 `.env`

```bash
./deploy/stack/gateway.sh up
```

会 `source .env` 并 `--force-recreate` gateway 容器。**不必**为改 env 单独 `pack-deploy`。

## 改 Rust 网关后（`http-gateway-rs`，容器内）

```bash
./deploy/stack/gateway.sh pack-deploy
```

## 改 e2b worker 里的 `claw`（dev 模式，**不走 CI**）

solve / terminal 在 **e2b MicroVM** 里跑 `claw`，不在 gateway 镜像里。改 `rusty-claude-cli`（`claw` 二进制）后：

```bash
./deploy/stack/gateway.sh e2b-worker-deploy
```

**不要**再为日常开发走：`push → GitHub CI → ACR pull → build 模板` 那条链。

### 本机 arm64 worker 节点（`10.8.0.2`）

Mac 已注册为 e2b **arm64 worker** 时，在 `.env` 设：

```bash
CLAW_E2B_WORKER_ARCH=arm64
CLAW_E2B_DEV_WORKER_HOST=10.8.0.2
```

`e2b-worker-deploy` 会 **原生编 linux/arm64**（无 amd64 模拟），模板上传到 e2b API；调度由 e2b 派到本机 worker 节点。生产节点 `10.8.0.1` 仍可用 `CLAW_E2B_WORKER_ARCH=amd64`。

### 这条命令做什么

| 步骤 | 在哪 | 说明 |
|------|------|------|
| 1 | 本机 podman | 编 `claw`（默认 **linux/arm64** on Apple Silicon） |
| 2 | 本机 | stage `claw` + curl `ttyd.aarch64`（或 `x86_64`）→ `.e2b-worker-bins/` |
| 3 | e2b API | `Template.build` 上传，注册别名 `claw-worker` |

Mac 是 **模板发布客户端**；沙箱在 e2b worker 节点（如 `10.8.0.2`）上跑，不是 gateway 容器里跑。

### 前提

- WireGuard `10.8.0.0/24`，能访问 `CLAW_E2B_API_URL`
- `.env`：`CLAW_E2B_API_KEY`、`CLAW_INTERACTIVE_BACKEND=e2b`
- arm64 dev：**快**（原生编译）；amd64 交叉编才慢

### 常用选项

```bash
# 已编好 claw，只重打模板
./deploy/stack/gateway.sh e2b-worker-deploy --skip-compile

./deploy/stack/gateway.sh e2b-worker-deploy --no-verify
```

### Gateway 怎么认出模板

Build 注册 **别名** `claw-worker`；Gateway `POST /sandboxes` 带 `templateID: claw-worker`。e2b 按调度把沙箱放到已注册的 worker 节点（arm64 / amd64）。

### dev vs release

| 模式 | 何时用 | 架构 | 命令 |
|------|--------|------|------|
| **dev（本机 worker）** | 日常改 `claw` | `arm64`（Mac） | `e2b-worker-deploy` + `CLAW_E2B_WORKER_ARCH=arm64` |
| **release** | 生产 `10.8.0.1` | `amd64` | `CLAW_E2B_WORKER_ARCH=amd64` 或 CI `from_image` |

OVS / observe / nas-api 模板变更频率低，仍按需单独 build，见 [`deploy/e2b/README.md`](../deploy/e2b/README.md)。

更细：`deploy/stack/lib/e2b-worker-deploy.sh`、`deploy/e2b/build-claw-worker-selfhosted.py`。

## 其它命令

| 命令 | 作用 |
|------|------|
| `./deploy/stack/gateway.sh e2b-worker-deploy` | **dev**：本机编 `claw` → 上传 e2b 模板（arm64/amd64 由 `CLAW_E2B_WORKER_ARCH` 定） |
| `./deploy/stack/gateway.sh playground` | 仅起 host 调试页 |
| `./deploy/stack/gateway.sh admin-build` | 只构建 React Admin `dist/` |
| `./deploy/stack/gateway.sh down` | 停 gateway + playground |
| `./deploy/stack/gateway.sh ps` | 看容器 |
| `./deploy/stack/gateway.sh help` | 帮助 |

实现脚本在 `deploy/stack/lib/`；**不要**在 `rust/` 子目录里直接跑 `gateway.sh`。

## 磁盘清理

| 路径 | 清理 |
|------|------|
| `rust/target/debug` | `gateway.sh clean --debug-only` |
| `rust/target` 全部 | `gateway.sh clean` |
| `deploy/stack/.linux-artifacts` | 随 `clean` 删除 |

## 常见坑

- **`zsh: no such file or directory: ./deploy/stack/gateway.sh`** — 先 `cd` 到仓库根。
- **solve 503 / e2b 错误** — 查 `CLAW_E2B_API_URL`、模板是否已 build；改 `claw` 用 `e2b-worker-deploy`，见上文 **dev 模式** 或 `deploy/e2b/README.md`。
- **Admin 界面旧** — `gateway.sh admin-build` 或 `quick`；浏览器强制刷新。

更多：`deploy/stack/README.md`、`docs/README.md`。
