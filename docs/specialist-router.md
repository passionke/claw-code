# Specialist Router 文档索引

Author: kejiqing

多 Agent 硬隔离 + Router 委托架构的**权威文档入口**。实现与评审以本目录为准。

## Mind 同步（协作副本）

| 项 | URL |
|----|-----|
| **NeruoGate 根** | https://mind.maxiot-inc.com/folders/3f2db9e2-d9d3-4135-b705-402ec3f9521f |
| **子目录 Specialist Router** | https://mind.maxiot-inc.com/folders/b384d013-9534-4027-9196-41aefd458036 |
| **本文（Mind）** | https://mind.maxiot-inc.com/documents/d286952c-f1e2-478e-944d-6dea94bc0c85 |
| **设计文档** | 子目录内同名文档（根目录旧副本已废弃） |
| **系分文档** | 子目录内同名文档 |
| **验收文档** | https://mind.maxiot-inc.com/documents/a90f9316-4884-4bc0-9cae-b0662fabada8 |

> **Git 为真源**；Mind 为协作副本。Router/delegate 文档放在 **NeruoGate / Specialist Router** 子目录，勿直接堆在 NeruoGate 根。变更时先改本地，再 `update_document` 同步子目录内文档。

## 本地文档

| 文档 | 路径 | 用途 |
|------|------|------|
| **设计** | [`specialist-router-design.md`](specialist-router-design.md) | 架构边界、角色、路由原则、与 Master/Mesh 关系 |
| **系分** | [`specialist-router-system-analysis.md`](specialist-router-system-analysis.md) | 表结构、API、tool 流程、物化、代码触点 |
| **验收** | [`specialist-router-acceptance.md`](specialist-router-acceptance.md) | 六场景 + sid/参数/SSE 断言 |

## 维护约定

1. **变更设计** → 先改本地 `docs/specialist-router-*.md`，再 `update_document` 同步 **NeruoGate / Specialist Router** 子目录内同名文档。
2. **Mind 上评审结论** → 回合写入本地（Mind 不作 git 真源）。
3. 每篇文首 **变更记录** 表追加一行（日期 / 作者 / 摘要）。
4. 相关但不在本专题内的契约：
   - [`live-report-contract.md`](live-report-contract.md) — SSE / passthrough
   - [`project-config-model.md`](project-config-model.md) — project_role / Master
   - [`gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md) — 意图回归
   - [`gpos-assistant-prompt-content.md`](gpos-assistant-prompt-content.md) — 三路意图迁移源

## 版本

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-08-14 | kejiqing | v1.2 本机验收 runbook；Mind 迁入 NeruoGate 子目录 |
