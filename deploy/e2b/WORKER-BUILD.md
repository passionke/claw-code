# e2b Worker 模板 — 唯一构建手册

Author: kejiqing

**自托管 e2b（10.8.0.x）worker 节点全是 `linux/amd64`。** Mac 开发机用 podman **交叉编译** amd64，不要编 arm64 再上传。

## 三件事（别每次从 0 到 1）

| 层 | 谁负责 | 做什么 |
|----|--------|--------|
| **1. 打包** | 人 / CI（偶尔） | `e2b-worker-deploy` → 编 `claw` + 打 e2b 模板 |
| **2. 上报 PG** | 构建脚本（自动） | `build-claw-worker-selfhosted.py` 写 `settings_json.e2bWorker.templateId` |
| **3. 初始化 + 续期** | gateway 启动 / 运行时（自动） | 读 PG → reconcile worker / singleton → TTL renewal ticker |

改完 `rusty-claude-cli` 才需要 **1**；**2、3 不用手搓**。

## 一条命令（改 claw 二进制后）

```bash
# 仓库根，已 merge env.selfhosted-e2b.example → .env
./deploy/stack/gateway.sh e2b-worker-deploy
```

内部步骤：

1. `linux/amd64` 交叉编译 `claw`（`CLAW_LINUX_COMPILE_PLATFORM` 由 `e2b-worker-arch.sh` 固定）
2. stage `claw` + `ttyd` → `deploy/stack/.e2b-worker-bins/`
3. `Template.build(alias=claw-worker)` 上传到 `CLAW_E2B_API_URL`
4. **写 PG** `e2bWorker.templateId` + `updatedAtMs`（与 ovs/observe/nas-api 同构）

可选：`--skip-compile` 复用 `deploy/stack/.linux-artifacts/release/claw`（须为 **amd64** ELF）。

## PG 契约

构建成功后 PG `gateway_global_settings.settings_json`：

```json
{
  "e2bWorker": {
    "templateId": "tpl_…",
    "alias": "claw-worker",
    "updatedAtMs": 1783…
  }
}
```

Gateway 读 `load_e2b_worker_template_id()`：`PG templateId` → env `CLAW_E2B_TEMPLATE` → `claw-worker`。

## Gateway 启动 / 运行时（不用手 reset 除非急）

`main.rs` 启动时：

- `ensure_e2b_singletons_on_startup` — ovs / observe / nas-api 单例
- `reconcile_project_workers_on_startup` — 各 proj worker 与 PG `templateId` 对齐，**mismatch 自动轮换**

运行时：

- `E2bProjWorkerRegistry::spawn_renewal_ticker` — 周期性 reconcile + TTL renew（默认 600s tick）

**新模板上传后**：重启 gateway **或** 等 renewal ticker；proj worker `templateContract` 不匹配会 `rotated_out` 并起新沙箱。

急用（不等 ticker）：

```bash
curl -X POST http://127.0.0.1:8088/v1/projects/1/e2b-worker/reset
```

## 验收

```bash
# OVS @claw agent/ws（须 gateway-interactive-once 或新 claw 正常）
./deploy/stack/lib/verify-ovs-claw-e2e.sh

# 全链路（OVS singleton + agent WS）
CLAW_INTERACTIVE_BACKEND=e2b CLAW_OVS_BACKEND=e2b \
  ./deploy/stack/lib/verify-e2b-ovs-e2e.sh
```

worker 内版本对齐：

```bash
# 从 gateway 容器 exec 进 proj worker，claw --version Git SHA 应与 gateway 一致
```

## 与 gateway 镜像的关系

| 改什么 | 命令 |
|--------|------|
| `http-gateway-rs` | `./deploy/stack/gateway.sh pack-deploy` |
| **e2b 沙箱内 `claw`** | `./deploy/stack/gateway.sh e2b-worker-deploy` |
| ovs / observe / nas-api 模板 | 各自 `build-claw-*-selfhosted.py`（也会写 PG） |

**不要**指望 `pack-deploy` 更新 worker 里的 claw — 那是两个镜像/模板链路。

## 环境变量（自托管）

`.env` 来自 `deploy/stack/env.selfhosted-e2b.example`：

```bash
CLAW_E2B_WORKER_ARCH=amd64          # 必须 amd64
CLAW_E2B_API_URL=http://10.8.0.1:3000
CLAW_E2B_TEMPLATE=claw-worker
CLAW_GATEWAY_DATABASE_URL=postgres://…  # PG persist 需要
CLAW_CLUSTER_ID=local-dev
```

## 故障

| 现象 | 原因 | 处理 |
|------|------|------|
| `claw is not linux/amd64 ELF` | 用了 arm64 产物 | 删 `.linux-artifacts/release/claw`，重跑 `e2b-worker-deploy` |
| `image not known` (compile) | 缺 amd64 compile 镜像 | 脚本会自动 `podman build --platform linux/amd64` → `claw-rust-compile:1.88-bookworm-amd64` |
| OVS `missing_credentials` | worker 里旧 claw | 跑本手册「一条命令」+ reset 或等 reconcile |
| PG 无 `e2bWorker.templateId` | 旧构建脚本 / 无 `CLAW_GATEWAY_DATABASE_URL` | 重跑 deploy，查构建日志 `persisted e2bWorker.templateId` |
