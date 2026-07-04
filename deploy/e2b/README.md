# e2b cloud sandbox (interactive + solve)

Author: kejiqing

**solve_async** 与 **interactive**（`terminal/start`, `agent/ws`, `ovs-*`）均在 **e2b（FC）MicroVM** 内执行。本地无 `claw-sandbox` / podman worker pool。

**Self-hosted e2b + NAS（10.8.0.x）：** [`docs/e2b-nas-workspace.md`](../../docs/e2b-nas-workspace.md)；env 模板 `deploy/stack/env.selfhosted-e2b.example`。

## Cost (NAS)

| Item | Value |
| --- | --- |
| NAS unit price | ¥0.001 / GB / hour |
| Planned capacity | 100 GB |
| Approx. yearly | **¥876 / year** |

e2b sandbox runtime is billed separately (MicroVM uptime; use sleep/wake to reduce cost).

## Prerequisites

1. e2b cloud sandbox enabled in **华北2 北京** + SLR + API Key (`e2b_…`)
2. NAS file system in **same region** (cn-beijing)
3. For e2b dynamic NAS mount: NAS VPC mount point + security group **2049/TCP**
4. Gateway / OVS: compose **NFS volume** mounts NAS inside containers; or run stack on Beijing ECS in VPC

## Template Build Guardrail

Worker / OVS / observe / nas-api 模板构建必须走 **e2b 标准构建路径**（SDK `Template.build` 上传）：

- **dev worker**：本机 `linux/amd64` compile + `COPY` → `gateway.sh e2b-worker-deploy`（见上文 **Dev 模式**）
- **release worker**：`FROM <CI claw-gateway-worker:release-vX>` 抽二进制再 upload，或 `from_image` 策略
- cloud worker: `from_image`，或 `file_context_path` + Dockerfile `COPY`

严禁使用临时 HTTP artifact server、`RUN curl http://host:port/...`、`dockerfile-http`、`CLAW_*_TEMPLATE_HTTP_*` 等非标准路径。模板构建链路必须由 e2b SDK 负责上传上下文或引用镜像，不允许依赖本机临时端口、内网 HTTP、手写 artifact server。Author: kejiqing

## Dev 模式：worker 模板（不走 CI）

日常改 `rusty-claude-cli`（e2b 沙箱内 `claw`）：

```bash
./deploy/stack/gateway.sh e2b-worker-deploy
```

**唯一手册：** [`WORKER-BUILD.md`](./WORKER-BUILD.md)（架构 amd64、PG 上报、gateway 自动 reconcile/续期）。

自托管 e2b（10.8.0.x）**全是 linux/amd64**；Mac 交叉编译，不要设 `CLAW_E2B_WORKER_ARCH=arm64`。

| 步骤 | 说明 |
|------|------|
| 交叉编译 | `linux/amd64` → `deploy/stack/.linux-artifacts/release/claw` |
| stage | `claw` + `ttyd` → `deploy/stack/.e2b-worker-bins/` |
| e2b SDK | `Template.build` → 写 PG `e2bWorker.templateId` |
| gateway | 启动 reconcile + renewal ticker 自动轮换 proj worker |

完整说明：[`WORKER-BUILD.md`](./WORKER-BUILD.md)。脚本：`deploy/stack/lib/e2b-worker-deploy.sh`。

OVS / observe / nas-api 模板仍按需单独 build（见下文 env 注释）；体积大或与 Rust 无关，不并入 `e2b-worker-deploy`。

### Observe 单例（clawTap / LLM 代理 + Live）

e2b 模式下 **不在宿主机起 tap**（`CLAUDE_TAP_MODE=off`）。observe 是 e2b 上的 **单例沙箱**（模板 `claw-observe`），由 `observe-tap-up` 创建并把 `clawTap` 写入 PG。

```bash
./deploy/stack/gateway.sh observe-tap-up --reuse    # 日常
./deploy/stack/gateway.sh observe-tap-up --reset    # 单例 502 / PG 脏数据 / 换模板后
```

**排障（502、PG 里仍是 252、`singleton_id` 报错等）：** [`deploy/docs/e2b-observe-tap-troubleshoot.md`](../docs/e2b-observe-tap-troubleshoot.md)

## Phase 0 — verify before gateway code path

### Step A — e2b API (no NAS)

From repo root (`.env` with `ALIYUN_E2B_TOKEN` or `CLAW_E2B_API_KEY`):

```bash
set -a && source .env && set +a
export E2B_API_KEY="${CLAW_E2B_API_KEY:-${ALIYUN_E2B_TOKEN}}"
export E2B_DOMAIN="${CLAW_E2B_DOMAIN:-cn-beijing.e2b.fc.aliyuncs.com}"

python3 -m venv /tmp/fc-quickstart-venv
source /tmp/fc-quickstart-venv/bin/activate
pip install e2b-code-interpreter -q
python3 deploy/e2b/quickstart.py
```

Pass: prints `hello from fc` and a `sandbox_id`.

### Step B — Gateway + OVS 直挂 NAS（无需 Mac 宿主机 mount）

在 repo 根 `.env`：

```bash
NAS_BASE_URL=xxx.cn-beijing.nas.aliyuncs.com
CLAW_E2B_NAS_EXPORT=/claw-workspace
CLAW_USE_NAS_VOLUME=auto   # NAS_BASE_URL 已设时默认开启；=0 退回本地 bind
```

`./deploy/stack/gateway.sh up` 生成 compose NFS volume（`deploy/stack/.claw-workspace-volume.yml`），**Podman 在 Gateway/OVS 容器内直接挂 NAS**。

验收：

```bash
./deploy/stack/gateway.sh up
podman exec claw-gateway-rs sh -c 'echo ok > /var/lib/claw/workspace/.probe'
podman exec claw-openvscode-server ls -la /home/workspace/.probe
```

**solve podman pool** 仍用本机 `deploy/stack/claw-workspace` 作 worker bind（与 Gateway/OVS 的 NAS 树分离，直到 solve 迁远程 pool）。

### Step C — e2b interactive（方案 A：NAS 注入 claw/ttyd，无需自定义 template）

OVS `@claw` 需要沙箱内有 **`claw`** 与 **`ttyd`**。因 e2b builder / ACR EE 路径不可行，采用 **官方 `code-interpreter-v1` + NAS 启动时拷贝二进制**。

#### 1. 一次性：把工具装到 NAS

在 gateway 能写 NAS 的机器上（234 ECS 已挂 `/mnt` 时 `CLAW_NAS_HOST_MOUNT=/mnt`）：

```bash
./deploy/e2b/install-nas-fc-tools.sh
```

产物：`{work_root}/.claw-e2b-tools/claw` + `ttyd`（默认 `CLAW_E2B_NAS_TOOLS_REL=.claw-e2b-tools`）。

#### 2. `.env`（交互 e2b 模式）

```bash
CLAW_INTERACTIVE_BACKEND=e2b
CLAW_E2B_TEMPLATE=code-interpreter-v1
NAS_BASE_URL=xxx.cn-beijing.nas.aliyuncs.com   # 或 CLAW_E2B_NAS_SERVER
CLAW_E2B_NAS_EXPORT=/                         # NAS 上 workspace 根，234 为 /
CLAW_E2B_NAS_TOOLS_REL=.claw-e2b-tools
# 不需要 CLAW_E2B_NAS_VOLUME_NAME —— 官方 code-interpreter-v1 无法在控制台绑 NAS
```

Gateway 创建沙箱时通过 API **`nasConfig`**（`hostMountRoot` + `relPath` → `mountDir`）：

| 逻辑 relPath | guest |
|--------------|-------|
| `proj_N/home` | `/claw_ds` |
| `proj_N/sessions/{segment}` | `/claw_host_root` |
| ``（export 根） | `/claw_ws` |

Gateway 在 **`CLAW_NAS_HOST_MOUNT`** 上 mkdir session 树；e2b 只做本机 bind。**不再** sandbox 内 `mount.nfs4`；**不再** `volumeMounts` / `CLAW_E2B_NAS_VOLUME_NAME`。

> **为何控制台绑不了？** `code-interpreter-v1` 是**官方只读模板**，没有「存储挂载 / NAS volume 名称」编辑项。这是预期行为。

> **VPC 要求（234 实测）：** 仅传 `nasConfig` 创建沙箱会 **201 成功**，但若 template **未开 VPC**，容器内 **不会出现** `/claw_host_root`（挂载被静默忽略）。需要 **自建沙箱模板**（Code Interpreter 镜像 + **同 NAS 的 VPC/交换机/安全组** + NAS 访问 execution role），**不必**在模板上绑 NAS volume 名：

| 控制台步骤 | 说明 |
| --- | --- |
| 函数计算 → 云沙箱 → **创建沙箱函数/模板** | 勿改官方 `code-interpreter-v1` |
| 镜像 | Code Interpreter（与官方相同） |
| 网络 | **允许访问 VPC**，选 NAS 挂载点所在 VPC + vSwitch |
| 安全组 | 放行 **2049/TCP** 到 NAS |
| 执行角色 | 含 NAS 访问权限 |
| 存储 | **不用**在模板上绑 NAS（由 gateway `nasConfig` 按 session 动态挂） |

`.env` 改用自建模板名，例如 `CLAW_E2B_TEMPLATE=claw-e2b-vpc-v1`。

#### 3. 运行时行为

每次 e2b 沙箱 `terminal/start` 或 solve，`e2b_exec.py` 在脚本前执行 `e2b-nas-bootstrap.sh`：从 NAS 拷到 `/claw_host_root/.claw/bin` 并加入 `PATH`，再启动 ttyd / `claw gateway-solve-once`。

#### 4. 验收

```bash
./deploy/stack/lib/verify-e2b-ovs-e2e.sh
```

---

### （备选，当前不推荐）自定义 claw-worker template + ACR EE

<details>
<summary>SDK builder / 自建北京 ACR EE — 234 实测 blocked</summary>

依据 [FC 自定义模板文档](https://help.aliyun.com/zh/functioncompute/fc/custom-template) 与 2026-06-19 在 **234 ECS** 上的构建实验：

### 推荐路径：`builder` + 杭州 ACR worker 镜像（**当前被 e2b 地域/仓库设计卡住**）

| 步骤 | 说明 |
| --- | --- |
| 源镜像 | `crpi-….cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12`（含 claw + ttyd） |
| 构建模式 | `X-E2B-Template-Build-Mode: builder` |

**234 实测三种 dest 行为（2026-06-19）：**

| dest 配置 | 结果 |
| --- | --- |
| **不指定** `DEST_IMAGE_REF` | builder 在 **杭州** 跑，产物推回 **杭州 ACR**（`…-fce2b-<buildId>`）→ 模板在 **北京** 创建沙箱时报 **`Image and function must be in the same region`** |
| **指定** `fc-e2b-registry.cn-beijing.cr.aliyuncs.com/passionke/claw-worker:…` | 转换 OK，push 阶段 **`401 Unauthorized`** —— 这是 **FC 平台托管仓库**，普通账号 **没有** push token（不是「去控制台设密码」能解决的） |
| **指定** 杭州 ACR 为 dest | e2b builder 平面访问 `cr.cn-hangzhou.aliyuncs.com` **超时** |

结论：**SDK builder 路径目前没有「零 ACR EE、零平台权限」的可走通方案。** 阿里云文档里的「自建北京 ACR EE + VPC」是官方正路，但贵且折腾。

### 实际可行路径（按推荐顺序）

#### 路径 A — NAS 启动时注入 claw/ttyd（**推荐，不买 EE**）

- 模板继续用官方 `code-interpreter-v1`（或任意 ready 模板）
- 在 NAS 固定路径放一份 `claw` + `ttyd`（与 gateway 共用 NAS）
- `terminal/start` 时在沙箱内 `cp` 到可写目录并 `PATH` 注入，再跑现有 `START_TTYD_SH_*`
- **无需** 自定义 template、**无需** push 任何镜像

（待实现：gateway e2b 启动脚本 + NAS 一次性上传工具二进制。）

#### 路径 B — 自建 **北京 ACR EE** + VPC（官方文档正路）

- 与 e2b 同地域、绑 VPC/vSwitch
- 推送 layered 镜像或让 builder dest 指向 **你自己的** `xxx-vpc.cn-beijing.cr.aliyuncs.com/...`
- 费用与运维成本由你承担

#### 路径 C — 等 e2b 产品侧支持

- 向阿里云提需求：builder 产物应自动落入 **账号下北京 fc-e2b 命名空间**，不应要求用户 push `fc-e2b-registry/…/passionke/…`

### ~~一次性准备：fc-e2b-registry 推送凭证~~（已证伪，勿走）

> **`fc-e2b-registry.cn-beijing.cr.aliyuncs.com` 是平台公共/托管仓库，用户不可能持有 push token。** 之前文档此处有误，已删除。

### 构建命令（仅当已有 **北京 ACR EE** dest 时）

```bash
cd /root/work/claw-code   # 或本机 repo 根
set -a && source .env && set +a

python3 -m venv .venv-fc
.venv-fc/bin/pip install e2b==2.26.0 e2b-code-interpreter python-dotenv

export CLAW_E2B_TEMPLATE=claw-worker-v1-prod
export CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image
export CLAW_E2B_TEMPLATE_BUILD_MODE=builder
export CLAW_E2B_TEMPLATE_SOURCE_REGISTRY_TYPE=acr
export CLAW_E2B_TEMPLATE_FROM_IMAGE=crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12
export CLAW_E2B_TEMPLATE_DEST_IMAGE_REF=fc-e2b-registry.cn-beijing.cr.aliyuncs.com/passionke/claw-worker:release-v1.6.12
export CLAW_E2B_TEMPLATE_SKIP_CACHE=0

./deploy/e2b/build-claw-worker-template.sh
```

成功时脚本会 create sandbox 并验证 `command -v ttyd`、`command -v claw`。

### e2b 控制台 NAS（legacy，已移除）

`CLAW_E2B_NAS_VOLUME_NAME` / template `volumeMounts` 已硬切移除；统一 `nasConfig` bind。

### 已验证不可行 / 勿走的路径

| 方式 | 234 结果 |
| --- | --- |
| `dockerfile` + COPY 25MB `claw` | `FileUploadException: Failed to get file upload link` |
| `dockerfile-http` + RUN curl / 临时 artifact HTTP server | **禁止使用**；不走 e2b 标准上下文上传，且依赖本机临时端口，已在脚本中硬拒绝 |
| `fromTemplate` | `fromTemplate is not supported` |
| 重建同名 `claw-worker-v1` | `409: template rebuild is not supported` |
| dest 指向杭州 ACR | e2b builder 平面访问 `cr.cn-hangzhou.aliyuncs.com` **超时** |
| 本机 push `fc-e2b-registry/…/claw-worker` | `requested access to the resource is denied`（无 push 权限） |
| 仅用官方 `code-interpreter-v1`（无 NAS bootstrap） | 沙箱可起，**无** ttyd/claw —— 方案 A 用 NAS bootstrap 解决 |

</details>

---

## Gateway env (interactive e2b mode)

See `deploy/stack/env.e2b-interactive.example`. Key variables:

| Variable | Role |
| --- | --- |
| `CLAW_INTERACTIVE_BACKEND=e2b` | Use e2b instead of podman pool for interactive |
| `CLAW_E2B_API_KEY` | e2b / E2B API key (fallback: `ALIYUN_E2B_TOKEN`) |
| `CLAW_E2B_API_URL` | Default `https://api.cn-beijing.e2b.fc.aliyuncs.com` |
| `CLAW_E2B_DOMAIN` | Default `cn-beijing.e2b.fc.aliyuncs.com` |
| `CLAW_E2B_TEMPLATE` | **`code-interpreter-v1`**（方案 A 默认） |
| `CLAW_E2B_NAS_TOOLS_REL` | NAS 上工具目录名，默认 `.claw-e2b-tools` |
| `CLAW_USE_NAS_VOLUME` | `auto`（有 `NAS_BASE_URL` 即 compose NFS 直挂） |
| `CLAW_E2B_NAS_EXPORT` | NAS export 子路径（234 多为 `/`） |
| `CLAW_E2B_NAS_VOLUME_NAME` | **可选/legacy**：仅自建 template + 控制台预注册 volume 时用 |
| `CLAW_E2B_EXEC_HELPER` | Default `deploy/e2b/e2b_exec.py` |

Template build-only (not runtime):

| Variable | Role |
| --- | --- |
| `CLAW_E2B_TEMPLATE_BUILD_STRATEGY` | `from_image` (default) or `dockerfile` |
| `CLAW_E2B_TEMPLATE_BUILD_MODE` | `builder` (default) or `direct` |
| `CLAW_E2B_TEMPLATE_FROM_IMAGE` | Source worker image |
| `CLAW_E2B_TEMPLATE_DEST_*` | 仅 **自建北京 ACR EE** 时需要；**不是** fc-e2b-registry |

## Rust client

`rust/crates/claw-e2b-sandbox-client/` — minimal E2B REST (`POST /sandboxes`, `DELETE /sandboxes/{id}`) + Python envd exec helper.

Interactive routing: `http-gateway-rs` → `InteractiveSandboxBackend` (`podman` | `fc`).

## E2E verify

```bash
./deploy/stack/lib/verify-e2b-ovs-e2e.sh
```

Requires `CLAW_INTERACTIVE_BACKEND=e2b`, NAS tools installed, gateway up, and LLM configured for full OVS chat.

## References

- [e2b sandbox overview](https://help.aliyun.com/zh/functioncompute/fc/what-is-a-fc-sandbox)
- [SDK quickstart](https://help.aliyun.com/zh/functioncompute/fc/create-your-first-cloud-sandbox-via-the-sdk)
- [Custom template](https://help.aliyun.com/zh/functioncompute/fc/custom-template)
- [Dynamic NAS mount](https://help.aliyun.com/zh/functioncompute/fc/user-guide/dynamically-mount-a-file-storage-nas)
- Repo plan: `docs/boundaries-claw-stack.md` (FC interactive section)
