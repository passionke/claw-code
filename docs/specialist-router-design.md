# Specialist Router — 设计文档

Author: kejiqing

| 项 | 值 |
|----|-----|
| **Mind 路径** | `03-BIST / NeruoGate / Specialist Router` |
| **Mind 本文** | 子目录内同名文档 |
| **Git 真源** | `docs/specialist-router-design.md` |
| **系分** | [`specialist-router-system-analysis.md`](specialist-router-system-analysis.md) |
| **验收** | [`specialist-router-acceptance.md`](specialist-router-acceptance.md) |

## 变更记录

| 日期 | 作者 | 摘要 |
|------|------|------|
| 2026-08-14 | kejiqing | v1.0 初稿 |
| 2026-08-14 | kejiqing | v1.1 本期验收场景 7 嵌套 delegate |

---

## 1. 背景与目标

GPOS 经营助手现网（预发 271 / 生产 27）在**单个 project** 内混合：产品手册 KB、SQLBot 问数、闲聊。需要 **硬隔离 + 对外统一入口**：

- 用户 / BFF **只打 router** projId
- 手册、问数、未来 marketing 等 **独立 specialist project**
- 协调写在 **router CLAUDE / skill**，短期不做网关级编排代码
- v1：**hub-and-spoke**；Mesh 多 agent 协同 **留后续**

## 2. 架构总览

```text
用户/BFF ──sessionId──► Router (project_role=router)
                              │
                    意图判别 (CLAUDE + specialist-registry)
                              │
                    delegate_project (serial, passthrough)
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
           kb-qa          ops-analysis    (future…)
         (normal)           (normal)
```

**用户 SSE：始终只连 router 当轮 task**；specialist 正文经 passthrough 抄入 router stdout（见 [`live-report-contract.md`](live-report-contract.md)）。

## 3. Project 拆分（v1）

| project | project_role | 职责 | 有 | 无 |
|---------|--------------|------|-----|-----|
| **router**（新建） | `router` | 判意图、委托 | `delegate_project`、`self-introduction` | SQLBot、KB |
| **kb-qa**（新建） | `normal` | 产品手册 how-to | `product-manual-qa`、`/home/kb` | SQLBot |
| **ops-analysis**（271） | `normal` | 经营问数 | SQLBot、分析 skills | 手册 KB |

生产 27 暂不动；预发先切。

## 4. 核心设计决策

### 4.1 协调在 skill，不在网关编排

| 决策 | 说明 |
|------|------|
| 路由知识在 router | CLAUDE + `specialist-registry`；**不做** specialist 快问 |
| 只派必要一路 | 禁止经营题顺带查 KB |
| 混合问 | 同轮多次 `delegate_project`，**serial**；router 不写合并稿 |
| Mesh | 后续专章；v1 不实现 agent 间协商 |

三路意图从 [`gpos-assistant-prompt-content.md`](gpos-assistant-prompt-content.md) 迁移至 router。

### 4.2 `project_role=router`

- 对外入口唯一 role；物化注入 `delegate_project`
- specialist 保持 `normal`；仅被 delegate
- 与 `master` / `observation` **不同 role、不同生命周期**

### 4.3 Delegate 配对：同构 Master、异构运行时

| | Master | Router |
|---|--------|--------|
| 配置 | `PUT .../apprentices` | `PUT .../delegate-targets` |
| 存储 | `project_master_link` | `gateway_delegate_target` |
| 可挂多个 | 是 | 是 |
| 用途 | 质量观测 | 用户问答路由 |

**禁止**共用 `project_master_link` 或 master MCP 做用户路由。

### 4.4 Session：用户管续聊，绑定落 DB

- 用户 `sessionId` **只对 router**
- 模型 **不传** delegate `sessionId`
- `gateway_delegate_session_link`：`root_session_id` 锚定整棵树；按 `(parent_session_id, parent_proj_id, delegate_proj_id)` 复用 delegate sid
- **禁止**内存 KV

### 4.5 执行模式：serial / parallel 单一控制

- 字段 `delegateExecutionMode`：`serial`（默认）\| `parallel`
- **v1 仅 serial**；parallel 未来另案

### 4.6 公正路由

| 层 | 职责 |
|----|------|
| Admin allowlist | 能委托谁 |
| Router skill | 意图 → target、拆混合问 |
| Tool | 硬拒未登记 projId |
| Specialist 隔离 | kb 无 SQLBot、ops 无 KB |

## 5. 边界：不做什么

- 并行委托、Hub 多路合并（v1）
- kb 等其他 specialist 发起嵌套（v1 仅验 **ops→下游** 一条链）
- master/apprentice 跑用户流量
- 网关 capability 路由表
- router 与 specialist per-turn 快问
- Mesh 协同协议（v1）

## 6. 与相关文档关系

| 文档 | 关系 |
|------|------|
| [`project-config-model.md`](project-config-model.md) | project_role、Master 观测 |
| [`multi-agent-analysis.md`](multi-agent-analysis.md) | ops **内部**编排，非对外 router |
| [`gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md) | 回归改打 router |

## 7. 演进路线

| 阶段 | 内容 |
|------|------|
| **v1** | hub-and-spoke、serial、router role、**场景 1–7 验收（含 ops→marketing 嵌套）** |
| **v1+** | 更多 specialist 嵌套、多级链 |
| **未来** | `parallel`、`capability_manifest`、Mesh 协同 |
