# Specialist Router — 系分文档

Author: kejiqing

| 项 | 值 |
|----|-----|
| **Mind 路径** | `03-BIST / NeruoGate / Specialist Router` |
| **Mind 本文** | 子目录内同名文档 |
| **Git 真源** | `docs/specialist-router-system-analysis.md` |
| **验收** | [`specialist-router-acceptance.md`](specialist-router-acceptance.md) |

## 变更记录

| 日期 | 作者 | 摘要 |
|------|------|------|
| 2026-08-14 | kejiqing | v1.0 初稿 |
| 2026-08-14 | kejiqing | v1.1 嵌套 delegate 物化与 allowlist（ops initiator） |

---

## 1. 模块与代码触点

| 模块 | 路径 | 职责 |
|------|------|------|
| project_role | `migrations/*.sql`、`master_observer.rs` | 扩展 CHECK 含 `router` |
| delegate 配对 | 新 `delegate_target.rs` 或 `master_observer` 旁新模块 | CRUD `gateway_delegate_target` |
| session link | `session_db.rs` + 新 repository | CRUD `gateway_delegate_session_link` |
| Admin API | `routes/fragments/` | `GET/PUT .../delegate-targets` |
| 物化 | `project_config_apply.rs` | `role=router` 注入 `delegate_project_tool`；registry 附录 |
| tool | `tools/` 或 `gateway-solve-turn` | `delegate_project_tool` 执行器 |
| live passthrough | `live_report_hub.rs` 等 | 订 specialist live → 抄 router stdout |

## 2. 数据模型

### 2.1 `project_config.project_role`

```sql
CHECK (project_role IN ('normal', 'master', 'observation', 'router'))
```

| role | 说明 |
|------|------|
| `router` | 对外入口；必物化 `delegate_project_tool` |
| `normal` | specialist；可作 delegate target；**ops 另开** `delegate_project_tool` 供嵌套 |
| `master` / `observation` | 观测拓扑；**不可**作 target |

可选列：`delegate_execution_mode TEXT DEFAULT 'serial'`（或 JSON 配置字段）。

### 2.2 `gateway_delegate_target`

| 列 | 类型 | 说明 |
|----|------|------|
| `cluster_id` | TEXT | PK 之一 |
| `router_proj_id` | BIGINT | PK 之一；**initiator**（router 或 ops 等） |
| `target_proj_id` | BIGINT | PK 之一 |
| `enabled` | BOOLEAN | 默认 true |
| `label` | TEXT | 可选 |
| `capability_hint` | TEXT | 可选；物化进 registry |
| `created_at_ms` / `updated_at_ms` | BIGINT | 审计 |

**PK：** `(cluster_id, router_proj_id, target_proj_id)`

### 2.3 `gateway_delegate_session_link`

| 列 | 说明 |
|----|------|
| `root_session_id` | 用户 router session（整树锚点） |
| `parent_session_id` | 发起委托方 session |
| `parent_proj_id` | 发起委托方 projId |
| `delegate_proj_id` | 目标 projId |
| `delegate_session_id` | 绑定 sid |
| `cluster_id` | 隔离 |
| timestamps | 审计 |

**PK：** `(parent_session_id, parent_proj_id, delegate_proj_id)`

首跳 router→specialist：`parent_session_id = root_session_id`。

## 3. Admin API

### 3.1 Delegate targets

```
GET  /v1/projects/{routerProjId}/delegate-targets
PUT  /v1/projects/{routerProjId}/delegate-targets
```

**PUT body：**

```json
{
  "targets": [
    { "targetProjId": 272, "enabled": true, "label": "kb-qa", "capabilityHint": "产品手册 how-to" },
    { "targetProjId": 271, "enabled": true, "label": "ops-analysis", "capabilityHint": "经营问数" }
  ]
}
```

校验：

- initiator 已物化 `delegate_project_tool` 且在其 `delegate-targets` 行中
- 各 target `project_role=normal` 且存在 stable

**嵌套示例：** `PUT /v1/projects/271/delegate-targets` 登记 marketing；router 不必直接登记 marketing。

### 3.2 Project role

复用现有 `PUT /v1/projects/{id}/role`，接受 `router`。

## 4. `delegate_project_tool`

### 4.1 入参（与 `SolveRequest` 同形）

```json
{
  "name": "delegate_project_tool",
  "arguments": {
    "projId": 271,
    "userPrompt": "…",
    "extraSession": { "store_id": "…", "tenant_code": "GPOS", … }
  }
}
```

| 字段 | 规则 |
|------|------|
| `projId` | 必填；allowlist + normal role |
| `userPrompt` | 必填 |
| `extraSession` | 原样透传 |
| `sessionId` | **模型不传**；tool 查 DB 注入 |

### 4.2 执行流程

```text
1. assert projId in gateway_delegate_target (initiator=当前 solve proj, enabled)
2. assert target.project_role == normal
3. lookup/insert gateway_delegate_session_link → delegate_session_id
   （嵌套时 parent_session_id = 上级 delegate sid；root 仍 = 用户 router session）
4. POST /v1/solve_async …
5. passthrough live …
```

### 4.3 userPrompt 改写

| 场景 | 处理 |
|------|------|
| 单意图 | 尽量原问 |
| 混合问 | 只提取该路子问 |
| 门店 | 走 extraSession，不堆 prompt |

## 5. 物化

| role | 注入 |
|------|------|
| `router` | `delegate_project_tool`；`specialist-registry` skill；从 `gateway_delegate_target` 生成 registry 附录 |
| `normal` | 各 specialist 原有 MCP/skills |
| `master` | `claw-master-observer`（不变） |

router **无** SQLBot MCP、**无** KB 挂载。

## 6. SSE / Live

- 客户端只订 **router** turn live（[`live-report-contract.md`](live-report-contract.md) 路径 B）
- passthrough：specialist `report.delta` → router stdout ingest → 同一 Hub 广播
- serial：同轮 tool₂ 在 tool₁ 终态后开始；禁止双路同时抄同一 stdout

## 7. 实施顺序

1. 迁移：`router` role + 两表
2. Admin API：`delegate-targets`
3. 预发拆 project + router skill
4. `delegate_project_tool`（维护者实现）
5. BFF/评测改打 router；跑验收矩阵

## 8. 预发拓扑（目标）

| projId | role | 名称 |
|--------|------|------|
| TBD | `router` | gpos-router |
| TBD | `normal` | kb-qa |
| 271 | `normal` | ops-analysis（去 KB） |

BFF projId → router。
