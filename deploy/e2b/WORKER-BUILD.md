# e2b Worker 模板 — 唯一构建手册

Author: kejiqing

**自托管 e2b（10.8.0.x）worker 节点全是 `linux/amd64`。**

## 三件事（别每次从 0 到 1）

| 层 | 谁负责 | 做什么 |
|----|--------|--------|
| **1. 打包** | Admin 初始化 / 重打模板 | `bootstrap-templates-from-ci-tag.sh <tag>` 打 strict、relaxed、observe、nas-api |
| **2. 上报 PG** | 上述脚本（自动） | 内容变了才写新 `buildId`，并记下 `contentHash` |
| **3. 初始化 + 续期** | gateway 启动 / 运行时（自动） | 只切换 pin / 协议不一致的沙箱；切完才对外服务 |

**原则：** 组件烘焙内容没变就不打新模板，PG `buildId` 保持不变。运行中健康沙箱**不会**因远端写了新 `buildId` 被杀掉。下次 gateway 启动时，期望 `buildId` 与沙箱上已应用的 pin 不一致才切换；协议版本、项目 home、profile 变化或沙箱已死也会重建。切换失败则 gateway 进程退出。

## 一条通道

在 Gateway Admin：

- **集群 Init**「e2b 模板」步，或
- **全局配置 → e2b 平台 → 制作 / 升级模板**

填写 tag（如 `release-v1.8.11`）→ **发布模板** / **制作 / 升级模板**。

等价脚本（Admin 调的就是这一支，不要另开入口）：

```bash
./deploy/e2b/bootstrap-templates-from-ci-tag.sh release-v1.8.19
```

内部步骤：从 CI 镜像抽出 linux/amd64 `claw` 与 `claude-tap`，再对四个组件算内容哈希。与 PG 里已有的 `contentHash` 相同且已有 `buildId` 则跳过 `Template.build`。不同才构建，并写入 `templateId`、`buildId`、`contentHash`。

`gateway.sh e2b-worker-deploy` 与 `build-selfhosted-templates.sh` 已移除。

## PG 契约

构建成功后 PG `gateway_global_settings.settings_json`：

```json
{
  "e2bWorker": {
    "templateId": "tpl_…",
    "buildId": "uuid…",
    "contentHash": "sha256…",
    "alias": "claw-worker",
    "updatedAtMs": 1783…
  },
  "e2bWorkerRelaxed": {
    "templateId": "tpl_…",
    "buildId": "uuid…",
    "contentHash": "sha256…",
    "alias": "claw-worker-relaxed",
    "updatedAtMs": 1783…
  }
}
```

Gateway：

- strict：`load_e2b_worker_template_id()` → `PG e2bWorker.templateId` → env → `claw-worker`
- relaxed：`load_e2b_worker_relaxed_template_id()` → `PG e2bWorkerRelaxed.templateId` → env → `claw-worker-relaxed`

**strict vs relaxed**：strict 用于 solve 池；relaxed = `claw` + **curl/git/python3/pip** + **内置 OVS**，用于 OVS / interactive。  
e2b 模板 `claw-worker-relaxed` 与 CI 镜像 `claw-gateway-worker-relaxed` **工具包对齐**；OVS **必须**在 e2b 模板 bake（从 `CLAW_OVS_IMAGE` 抽 openvscode 树 + 装扩展）。Admin 发布在 gateway 内用 **registry HTTP 抽树**（无嵌套 podman）。不要单独跑 `build-claw-*-selfhosted.py`。

## Gateway 启动 / 运行时（不用手 reset 除非急）

`main.rs` 启动时：

- `ensure_e2b_singletons_on_startup` — nas-api / observe 先完成。期望 `buildId` ≠ 运行中 `appliedBuildId` 才重建；失败则 gateway 退出
- `reconcile_project_workers_on_startup` — 各 proj worker 槽，并发 8。期望 `buildId` 与沙箱已应用 pin 不一致、协议 / home / profile 变化、或沙箱已死才切换。全部成功后 gateway 才继续起来；失败则进程退出

运行时：

- TTL renew / 健康检查；**不会**因新 `buildId` 杀掉健康沙箱，也不会把记录里的 pin 改成尚未切换的 build
- 立刻换某一项目的 worker：Admin `POST …/e2b-worker/reset`，或等沙箱失活后重建

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
| **e2b 模板（strict、relaxed、observe、nas-api）** | Admin 发布，或 `./deploy/e2b/bootstrap-templates-from-ci-tag.sh <tag>` |

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
| `claw is not linux/amd64 ELF` | 抽出的 claw 不是 amd64 | 用 CI 镜像 tag 走 Admin 发布，不要在 arm64 上交叉编译 |
| OVS `missing_credentials` | worker 里旧 claw | Admin 发布后重启 gateway，或对该项目 reset |
| PG 无 `e2bWorker.templateId` | 发布未写 PG / 无 `CLAW_GATEWAY_DATABASE_URL` | 重跑 Admin 发布，查日志 `persisted e2bWorker.templateId` |
| relaxed 仍是旧 claw | 发布被跳过或 gateway 未重启 | 查 `contentHash` 是否真的变了；变了则重启 gateway 等启动切换 |
