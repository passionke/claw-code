# e2b Worker 模板 — 唯一构建手册

Author: kejiqing

**自托管 e2b（10.8.0.x）worker 节点全是 `linux/amd64`。**

## 三件事（别每次从 0 到 1）

| 层 | 谁负责 | 做什么 |
|----|--------|--------|
| **1. 打包** | 人 / CI | `e2b-worker-deploy` → 拿 amd64 `claw` + 打 e2b **strict + relaxed** 模板 |
| **2. 上报 PG** | 构建脚本（自动） | 写 `e2bWorker.templateId` + `buildId`（及 relaxed 同构） |
| **3. 初始化 + 续期** | gateway 启动 / 运行时（自动） | 读 PG → reconcile worker / singleton → TTL renewal ticker |

**原则：** rebuild 只更新 PG（稳定 `templateId` + 新 `buildId`）。运行中健康沙箱**不会**因远端升级自动换镜像；换新镜像靠 **gateway 重启**、**手工 reset**，或沙箱失活后重建。

改完 `rusty-claude-cli` 才需要 **1**；**2、3 不用手搓**。

## 一条命令

**amd64 CI / Linux 主机（交叉编译）：**

```bash
./deploy/stack/gateway.sh e2b-worker-deploy
```

**Mac arm64（用 CI 镜像里的 amd64 claw，勿走 qemu 编译）：**

```bash
./deploy/stack/gateway.sh e2b-worker-deploy --from-ci-image release-v1.7.19
```

内部步骤：

1. 获得 linux/amd64 `claw`（编译，或从 `claw-code:<tag>` 抽出）
2. stage → `deploy/stack/.e2b-worker-bins/`
3. `Template.build(alias=claw-worker)` → 写 PG `e2bWorker.templateId` + `buildId`
4. `Template.build(alias=claw-worker-relaxed)`（**同一份 claw** + OVS）→ 写 PG `e2bWorkerRelaxed.templateId` + `buildId`

其它：

- `--skip-compile` 复用已有 `.linux-artifacts/release/claw`（须 amd64 ELF）
- `--strict-only` 只打 strict（默认 strict+relaxed）

## PG 契约

构建成功后 PG `gateway_global_settings.settings_json`：

```json
{
  "e2bWorker": {
    "templateId": "tpl_…",
    "buildId": "uuid…",
    "alias": "claw-worker",
    "updatedAtMs": 1783…
  },
  "e2bWorkerRelaxed": {
    "templateId": "tpl_…",
    "buildId": "uuid…",
    "alias": "claw-worker-relaxed",
    "updatedAtMs": 1783…
  }
}
```

Gateway：

- strict：`load_e2b_worker_template_id()` → `PG e2bWorker.templateId` → env → `claw-worker`
- relaxed：`load_e2b_worker_relaxed_template_id()` → `PG e2bWorkerRelaxed.templateId` → env → `claw-worker-relaxed`

**strict vs relaxed**：strict 用于 solve 池；relaxed = `claw` + **curl/git/python3/pip** + **内置 OVS**，用于 OVS / interactive。  
e2b 模板 `claw-worker-relaxed` 与 CI 镜像 `claw-gateway-worker-relaxed` **工具包对齐**（OVS 由 e2b 模板 bake；沙箱更新必须走本手册「一条命令」）。

单独只打 relaxed（少见；一般用上面一条命令）：

```bash
# 须已有 stage 好的 claw，或由脚本从 CLAW_E2B_WORKER_IMAGE 抽出
CLAW_E2B_TEMPLATE_COPY_DIR=deploy/stack/.e2b-worker-bins \
  .venv-fc/bin/python3 deploy/e2b/build-claw-worker-relaxed-selfhosted.py
```

`build-selfhosted-templates.sh worker` 仍是 strict→relaxed，与 `e2b-worker-deploy` 同序。

## Gateway 启动 / 运行时（不用手 reset 除非急）

`main.rs` 启动时：

- `ensure_e2b_singletons_on_startup` — ovs / observe / nas-api 单例（`image_refresh`：PG `buildId` ≠ `appliedBuildId` 时 recreate）
- `reconcile_project_workers_on_startup` — 各 proj worker；**仅启动窗口**可因 `buildId` 差换镜像

运行时：

- TTL renew / 健康检查；**不会**因远端 rebuild 写了新 `buildId` 就自动 kill 健康沙箱
- 换新镜像：gateway 重启、Admin `POST …/e2b-worker/reset`，或沙箱失活后重建

急用（手工重建）：

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
| **e2b 沙箱内 `claw`（strict + relaxed）** | `./deploy/stack/gateway.sh e2b-worker-deploy` |
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
| `claw is not linux/amd64 ELF` | 用了 arm64 产物 | Mac 用 `--from-ci-image release-v…`；或删 `.linux-artifacts/release/claw` 后在 amd64 上重编 |
| arm64 Mac 直接 `e2b-worker-deploy` 退出 | 默认禁止 qemu 交叉编译 | 用 `--from-ci-image`；勿 `--force-compile` 除非调试 |
| `image not known` (compile) | 缺 amd64 compile 镜像 | 脚本会自动 `podman build --platform linux/amd64` → `claw-rust-compile:1.88-bookworm-amd64` |
| OVS `missing_credentials` | worker 里旧 claw | 跑本手册「一条命令」+ reset 或等 reconcile |
| PG 无 `e2bWorker.templateId` | 旧构建脚本 / 无 `CLAW_GATEWAY_DATABASE_URL` | 重跑 deploy，查构建日志 `persisted e2bWorker.templateId` |
| relaxed 仍是旧 claw | 只打了 strict / 未轮换 | 默认 `e2b-worker-deploy` 含 relaxed；查 `e2bWorkerRelaxed.templateId` 后 restart / reset |
