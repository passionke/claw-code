# Relaxed Worker 验收契约（OVS 退出后）

> **ARCHIVED — OVS 已全面退出（2026-09-27）。** 本文仅作历史取证；现行架构见 `docs/architecture-governance.md`。`mode=relaxed` 现为宽松 worker，不再提供 OpenVSCode / `ovs/workspace`。

Author: kejiqing

## 现行验收（2026-09-27）

| ID | 断言 |
|----|------|
| INV-A | 无对 `:3000/ovs` / OpenVSCode 的运行时依赖 |
| INV-B | 无 `metadata.clawRole=ovs-singleton` 存活 sandbox；无 ovs-singleton ensure/reset 路径 |
| INV-C | `GET /v1/projects/{id}/ovs/workspace` **不可用**（404 或路由已移除） |
| INV-D | relaxed 项目 `ensure_worker` 成功（`claw-worker-relaxed`） |
| INV-E | relaxed 项目 home `/claw_ds` **可写**（rw bind）；strict 仍为 ro |

## 架构摘要（现行）

- **strict：** `claw-worker`；home `/claw_ds` 只读
- **relaxed：** `claw-worker-relaxed` = claw + curl/git/python3/pip（**permissions / tools-only**）；home `/claw_ds` 可写
- **已废除：** 独立 `ovs-singleton`、relaxed 内置 OpenVSCode、`ovs/workspace` 产品面

详见 [`architecture-governance.md`](../architecture-governance.md) §3–§4、[`e2b-nas-workspace.md`](../e2b-nas-workspace.md)。

## 验证层级（现行）

| 层级 | 命令 | 说明 |
|------|------|------|
| L0 | `cargo test -p claw-e2b-sandbox-client nas_paths` | home ro/rw 契约单测 |
| L1 | `./deploy/stack/lib/verify-e2b-nas-inject.sh` | NAS bind |
| L2 | `./deploy/stack/lib/verify-relaxed-worker.sh` | ensure_worker；`ovs/workspace` → 404；无 ovs-singleton |

历史 OVS E2E 脚本（`verify-e2b-ovs-e2e.sh` 等）已删除，勿再引用。

## 环境变量

| 变量 | 默认 | 用途 |
|------|------|------|
| `CLAW_ALLOW_RELAXED_WORKER` | 须显式允许 | 启用 `mode=relaxed` |
| `CLAW_E2B_TEMPLATE_RELAXED` / PG `e2bWorkerRelaxed` | `claw-worker-relaxed` | relaxed 模板 |

---

## 历史正文（OVS 内置期，已失效）

以下为 2026-07 前后「relaxed 内置 OVS」验收原文，**不再作为现行契约**：

- 当时：strict `ovs/workspace` → 403；relaxed `workspaceFolder == "/claw_ds"`；OVS 与 claw 同 sandbox；废除独立 ovs-singleton。
- 验证曾依赖 `verify-relaxed-worker-ovs.sh` / `verify-e2b-ovs-e2e.sh` / `verify-ovs-claw-e2e.sh`。
