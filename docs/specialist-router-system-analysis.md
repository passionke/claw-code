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
| 2026-10-09 | kejiqing | v1.2 仅 router 发起；target 含 router；router_json.bodyRelay；防环；biz.delegate SSE |

---

## 1. 模块与代码触点

| 模块 | 路径 | 职责 |
|------|------|------|
| project_role | `migrations/*.sql`、`master_observer.rs` | CHECK 含 `router`；seed 纯 hub |
| delegate 配对 | `delegate_router.rs` | CRUD relation + 环检测 + bodyRelay |
| router_json | `migrations/9_project_router_json.sql`、`session_db` | `{ "bodyRelay": "passthrough\|progress" }` |
| session link | `session_db` / `delegate_router` | `gateway_delegate_session_link` |
| Admin API | `routes/fragments/delegate.rs` | `GET/PUT .../delegate-targets`；`resolve-session` |
| 物化 | `prepare_router_materialize_row` | registry 附录 + delegate tools |
| tool | `gateway-solve-turn::delegate_project_tool` | passthrough / progress 分支 |
| live | `live_report_hub` / `live_report_sse` | worker 抄正文；`biz.delegate.*` |

## 2. 数据模型

### 2.1 `project_config.project_role`

| role | 说明 |
|------|------|
| `router` | **唯一**委托发起方；纯 hub；可作 target；可直连入口 |
| `normal` / `knowledge_base` | specialist；**仅**作 target |
| `master` / `observation` | 观测拓扑；**不可**作 target |

### 2.2 `project_config.router_json`

```json
{ "bodyRelay": "passthrough" }
```

缺省 = `passthrough`。

### 2.3 Delegate edges（`project_relation` type `router_delegate`）

| 字段 | 说明 |
|------|------|
| `from_proj_id` | initiator（必须是 router） |
| `to_proj_id` | target：`normal` \| `knowledge_base` \| `router` |
| meta | `enabled`、`capabilityHint` |

PUT 时：`pg_advisory_xact_lock` + 有向图无环（只计 enabled）。

### 2.4 `gateway_delegate_session_link`

| 列 | 说明 |
|----|------|
| `root_session_id` | 本场入口 session（整树锚点） |
| `parent_session_id` | 发起委托方 session |
| `parent_proj_id` | 发起方 projId |
| `delegate_proj_id` / `delegate_session_id` | 目标 |

## 3. Admin API

### 3.1 Delegate targets

```
GET  /v1/projects/{routerProjId}/delegate-targets
PUT  /v1/projects/{routerProjId}/delegate-targets
```

**PUT body：**

```json
{
  "bodyRelay": "passthrough",
  "targets": [
    { "targetProjId": 272, "enabled": true, "label": "kb-qa", "capabilityHint": "产品手册 how-to" },
    { "targetProjId": 280, "enabled": true, "label": "ops-hub", "capabilityHint": "问数子 hub" }
  ]
}
```

校验：

- initiator `project_role=router`
- target `normal|knowledge_base|router` 且存在 config
- 委托图无环；禁止自环

**嵌套示例：** `PUT /v1/projects/{R2}/delegate-targets` 登记 ops/marketing；R1 只登记 R2。

### 3.2 Resolve session

`POST /v1/projects/{initiator}/delegate/resolve-session` 响应含 `bodyRelay`、`label`（供 worker）。

## 4. `delegate_project_tool`

| 字段 | 规则 |
|------|------|
| `projId` | 必填；allowlist + 允许角色 |
| `userPrompt` | 必填 |
| `extraSession` | 原样透传 |
| `sessionId` | **模型不传** |

### 执行

```text
1. resolve-session → bodyRelay, label, delegate_session_id
2. solve_async 子 turn
3. emit delegate.active（真实 child sessionId）
4. passthrough: 订子 live → emit_report_delta + 落盘
   progress: poll 终态 → reportPath 写引用桩（不抄 Hub）
5. emit delegate.clear
6. complete_router_turn 校验 reportPath 非空
```

引用桩格式：

```markdown
> 已委托 {label} 处理（projId=…, sessionId=…, turnId=…）
```

## 5. 物化

| role | 注入 |
|------|------|
| `router` | `delegate_project_tool`；`complete_router_turn`；`specialist-registry`；registry 附录 |
| `normal` | 各 specialist 原有 MCP/skills（**无** delegate tool） |

## 6. SSE / Live

- 客户端订 **入口** turn（[`live-report-contract.md`](live-report-contract.md) 路径 B′）
- `passthrough`：子 delta → 本层 stdout → Hub → `biz.report.delta`
- **新增** `biz.delegate.active` / `biz.delegate.clear`（两种模式都发；订阅时回放 active）
- payload：`{ sessionId, turnId, projId, label? }` — 前端点开另订子 turn
- serial：同轮 tool₂ 在 tool₁ 终态后开始
- 委派结束后 `complete_router_turn`；禁止复述

## 7. 预发拓扑（目标）

| role | 名称 |
|------|------|
| `router` | gpos-router（R1） |
| `router` | ops-hub（R2，可选） |
| `normal` | kb-qa |
| `normal` | ops-analysis / marketing |
