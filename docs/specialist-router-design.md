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
| 2026-10-09 | kejiqing | v1.2 router 嵌套 hub；仅 router 发起；bodyRelay；防环 |

---

## 1. 背景与目标

GPOS 经营助手现网（预发 271 / 生产 27）在**单个 project** 内混合：产品手册 KB、SQLBot 问数、闲聊。需要 **硬隔离 + 对外统一入口**：

- 用户 / BFF **打入口 router** projId（也可直连任一 router；入口与被关联正交）
- 手册、问数、未来 marketing 等 **独立 specialist project**（`normal` / `knowledge_base`）
- 多级委托 = **router 套 router**（纯 hub）；`normal` **不能**发起委托
- 协调写在 **router CLAUDE / skill**，短期不做网关级编排代码
- v1：**hub-and-spoke + router 嵌套**；Mesh 多 agent 协同 **留后续**

## 2. 架构总览

```text
用户/BFF ──sessionId──► R1 (project_role=router, 纯 hub)
                              │
                    意图判别 (CLAUDE + specialist-registry)
                              │
                    delegate_project_tool (serial)
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
           kb-qa            R2 (router hub)  (future…)
         (normal)               │
                          ┌─────┴─────┐
                          ▼           ▼
                     ops-analysis  marketing
                       (normal)     (normal)
```

**用户 SSE：始终只连本场入口 turn**；下级正文由 worker `delegate_project_tool` **抄进本层 stdout**（见 [`live-report-contract.md`](live-report-contract.md) 路径 B′）。`bodyRelay=passthrough`（默认）链式冒泡；`progress` 只发 `biz.delegate.*`，正文在子 turn。

## 3. Project 拆分（v1）

| project | project_role | 职责 | 有 | 无 |
|---------|--------------|------|-----|-----|
| **router**（入口 / 子 hub） | `router` | 判意图、委托 | `delegate_project_tool`、`complete_router_turn`、`self-introduction` | SQLBot、KB、业务 MCP |
| **kb-qa** | `normal` | 产品手册 how-to | `product-manual-qa`、`/home/kb` | SQLBot、delegate |
| **ops-analysis** | `normal` | 经营问数 | SQLBot、分析 skills | 手册 KB、delegate |

生产 27 暂不动；预发先切。

## 4. 核心设计决策

### 4.1 协调在 skill，不在网关编排

| 决策 | 说明 |
|------|------|
| 路由知识在 router | CLAUDE + `specialist-registry`；**不做** specialist 快问 |
| 只派必要一路 | 禁止经营题顺带查 KB |
| 混合问 | 同轮多次 `delegate_project_tool`，**serial**；router 不写合并稿 |
| Mesh | 后续专章；v1 不实现 agent 间协商 |

### 4.2 `project_role=router`

- **纯 hub**：种子清空 MCP/KB；只做委托
- **唯一发起方**：仅 `router` 可 `PUT/resolve` delegate；`normal` 拒
- **可作 target**：另一 router 可挂本 router（嵌套）
- **入口正交**：被挂为 target **不**封锁 BFF 直连
- 与 `master` / `observation` **不同 role、不同生命周期**

### 4.3 Delegate 配对

| | Master | Router |
|---|--------|--------|
| 配置 | `PUT .../apprentices` | `PUT .../delegate-targets`（含顶层 `bodyRelay`） |
| 存储 | `project_master_link` | `project_relation` / `router_delegate` + `project_config.router_json` |
| 可挂多个 | 是 | 是 |
| 用途 | 质量观测 | 用户问答路由 |

**禁止**共用 `project_master_link` 或 master MCP 做用户路由。  
**委托图无环**：PUT 时 advisory lock + 有向图检测。

### 4.4 Session：用户管续聊，绑定落 DB

- 用户 `sessionId` **对入口 router**（直连哪层，root 就锚哪层）
- 模型 **不传** delegate `sessionId`
- `gateway_delegate_session_link`：`root_session_id` 锚定整棵树
- **禁止**内存 KV

### 4.5 `bodyRelay`（冒泡开关）

| 值 | 行为 |
|----|------|
| `passthrough`（默认） | 下级 `biz.report.delta` 抄进本层 Hub + `reportPath` |
| `progress` | 不抄正文；发 `biz.delegate.active/clear`；`reportPath` 只写引用桩 |

配置在 **发起方 router**（`router_json.bodyRelay`），作用于其全部 target。模型不传。

### 4.6 执行模式：serial / parallel

- **v1 仅 serial**；parallel 未来另案

### 4.7 公正路由

| 层 | 职责 |
|----|------|
| Admin allowlist | 能委托谁 + 无环 |
| Router skill | 意图 → target、拆混合问 |
| Tool | 硬拒未登记 projId |
| Specialist 隔离 | kb 无 SQLBot、ops 无 KB |

## 5. 边界：不做什么

- 并行委托、Hub 多路合并（v1）
- `normal` 发起委托
- master/apprentice 跑用户流量
- 网关 capability 路由表
- router 与 specialist per-turn 快问
- Mesh 协同协议（v1）
- 按边配置 `bodyRelay` / tool 参数覆盖

## 6. 与相关文档关系

| 文档 | 关系 |
|------|------|
| [`project-config-model.md`](project-config-model.md) | project_role、router_json |
| [`live-report-contract.md`](live-report-contract.md) | 路径 B′ + `biz.delegate.*` |
| [`multi-agent-analysis.md`](multi-agent-analysis.md) | ops **内部**编排，非对外 router |
| [`gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md) | 回归改打 router |

## 7. 演进路线

| 阶段 | 内容 |
|------|------|
| **v1** | hub-and-spoke、serial、router role、场景 1–7 |
| **v1.2** | router 嵌套、防环、bodyRelay、场景 8–9 |
| **未来** | `parallel`、`capability_manifest`、Mesh 协同 |
