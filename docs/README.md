# Claw 文档索引（e2b-only）

Author: kejiqing

**当前唯一支持的 worker 路径：** solve / interactive / OVS / Observe / NAS 写盘均经 **e2b（FC）沙箱**；本地栈仅 **gateway + playground**，外连独立 PG 与 e2bserver。

**架构总纲：** [`architecture-governance.md`](architecture-governance.md)

---

## 入门

| 文档 | 用途 |
|------|------|
| [`architecture-governance.md`](architecture-governance.md) | 目标拓扑、NAS 不变量、e2b singleton、部署命令、迁移 checklist |
| [`local-dev.md`](local-dev.md) | 本地一条命令 `gateway.sh quick` |
| [`deploy/stack/README.md`](../deploy/stack/README.md) | 运维手册：起停、镜像、排障 |
| [`env-config.md`](env-config.md) | 根 `.env` 变量清单（`local` / `production` profile） |
| [`env-files.md`](env-files.md) | 人手维护 vs 脚本生成路径 |

**Env 模板（复制到仓库根 `.env`）：**

| 场景 | 模板 |
|------|------|
| 自托管 e2b + 外连 PG（推荐） | `deploy/stack/env.selfhosted-e2b.example` |
| e2b interactive 叠加项 | `deploy/stack/env.e2b-interactive.example` |
| macOS 全本地 compose | `deploy/stack/env.local.example` |
| Linux 线上 CI 镜像 | `deploy/stack/env.production.example` |

---

## 架构与边界

| 文档 | 用途 |
|------|------|
| [`boundaries-claw-stack.md`](boundaries-claw-stack.md) | 组件职责、不变量、改哪里 |
| [`http-gateway-container-pool.md`](http-gateway-container-pool.md) | **e2b worker 编排**（solve 经 e2b，非宿主机 pool） |
| [`e2b-nas-workspace.md`](e2b-nas-workspace.md) | NAS 路径、host bind、e2b 挂载契约 |
| [`persistence-model.md`](persistence-model.md) | jsonl 运行时 vs `gateway_turns` 终态 |
| [`live-report-contract.md`](live-report-contract.md) | stdout hooks、live SSE（FC 路径） |
| [`pool-registry.md`](pool-registry.md) | `claw_pool` 表（**历史/兼容**；无 `:9944` RPC） |
| [`deploy/SERVICES.md`](../deploy/SERVICES.md) | 各 deploy 子目录边界与构建隔离 |

---

## e2b / OVS / NAS

| 文档 | 用途 |
|------|------|
| [`deploy/e2b/README.md`](../deploy/e2b/README.md) | e2b API 验收、模板构建、成本 |
| [`ovs-chat/FC-OVS-SINGLETON-DESIGN.md`](ovs-chat/FC-OVS-SINGLETON-DESIGN.md) | OVS e2b singleton |
| [`ovs-chat/FC-OVS-TEMPLATE-BUILD.md`](ovs-chat/FC-OVS-TEMPLATE-BUILD.md) | `claw-ovs` 模板 |
| [`ovs-chat/FC-TAP-SINGLETON-DESIGN.md`](ovs-chat/FC-TAP-SINGLETON-DESIGN.md) | Observe tap singleton |
| [`ovs-chat/FC-OVS-E2E-FAILURES.md`](ovs-chat/FC-OVS-E2E-FAILURES.md) | 已知 e2b 排障 |
| [`ovs-chat/INTEGRATION.md`](ovs-chat/INTEGRATION.md) | OVS + claw-vscode 集成 |

---

## Gateway API / 运维

| 文档 | 用途 |
|------|------|
| [`http-gateway-rs-quickstart.md`](http-gateway-rs-quickstart.md) | API 速查 |
| [`http-gateway-rs-api.md`](http-gateway-rs-api.md) | 接口详表 |
| [`deploy-ops-truth.md`](deploy-ops-truth.md) | 脚本实际行为（与 README 对照） |
| [`project-config-model.md`](project-config-model.md) | PG `project_config` |
| [`gateway-solve-preflight.md`](gateway-solve-preflight.md) | 首轮 solve preflight（现状） |
| [`ovs-chat/PREFLIGHT-SPI-PLAN.md`](ovs-chat/PREFLIGHT-SPI-PLAN.md) | **Preflight SPI 插件化**（`feat/preflight-spi` 设计稿） |
| [`claw-tap-cluster-identity.md`](claw-tap-cluster-identity.md) | clawTap `clusterHash` |
| [`langfuse-otel.md`](langfuse-otel.md) | OTEL span 命名 |

**Deploy 子文档：** `deploy/stack/docs/` — GitLab CI、集群验收（已按 e2b-only 更新引用）。

---

## 已废弃（勿按此操作）

| 文档 / 路径 | 说明 |
|-------------|------|
| [`claw-sandbox-system-design.md`](claw-sandbox-system-design.md) | 重定向 |
| `sandbox/docs/system-design.md` | 宿主机 `claw-sandbox` 已移除 |
| [`deploy/stack/docs/stable-sandbox-host.md`](../deploy/stack/docs/stable-sandbox-host.md) | 94 稳定 pool 主机 |
| [`deploy/stack/docs/local-dev-remote-backend.md`](../deploy/stack/docs/local-dev-remote-backend.md) | 模式 B 远程 pool |
| `deploy/stack/docs/host-pool-daemon.md` | **已删除** |
| `env.stable-dev-host.example` / `env.local-remote-backend.example` | 历史模板 |

旧变量（`CLAW_POOL_*`、`CLAW_SANDBOX_*`、`podman_pool`、`docker_pool`）在根 `.env` 可保留但**不再生效**；见 `architecture-governance.md` §7。
