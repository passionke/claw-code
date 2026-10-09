# Specialist Router — 验收文档

Author: kejiqing

| 项 | 值 |
|----|-----|
| **Mind 路径** | `03-BIST / NeruoGate / Specialist Router` |
| **Mind 本文** | https://mind.maxiot-inc.com/documents/a90f9316-4884-4bc0-9cae-b0662fabada8 |
| **Git 真源** | `docs/specialist-router-acceptance.md` |
| **系分** | [`specialist-router-system-analysis.md`](specialist-router-system-analysis.md) |

## 变更记录

| 日期 | 作者 | 摘要 |
|------|------|------|
| 2026-08-14 | kejiqing | v1.0 六场景 + 全局不变量 |
| 2026-08-14 | kejiqing | v1.2 Mind 迁入 NeruoGate/Specialist Router 子目录；本机验收 runbook |
| 2026-08-20 | kejiqing | v1.4 harness restore：tool result 含 message；SSE=task=PG；撤 output yield |
| 2026-10-09 | kejiqing | v1.5 场景7 改 router 嵌套；场景8深嵌套；场景9 progress；normal 拒发起 |

---

v1 以本节为 **验收真源**。实现服务于可观测断言，不以内部复杂度为准。

## 1. 全局不变量

| 维度 | 标准 | 验证 |
|------|------|------|
| 用户 session | 续聊同一 `sessionId` → 入口 router；新开 = 新 sid | DB / `GET /v1/sessions` |
| 模型不传 sid | 各级 `delegate_project_tool` 均无 `sessionId` | tool 日志 |
| root 锚点 | **所有** link 行 `root_session_id` = 本场入口 session（含嵌套跳） | SQL |
| sid 复用 | 同 `(parent_session_id, parent_proj_id, delegate_proj_id)` 复用 delegate sid | 多轮对比 |
| allowlist | 每级发起方仅可 delegate 其 `delegate-targets` 内 proj | 负例 |
| 发起方 | **仅** `project_role=router` | PUT / resolve-session |
| 无环 | enabled 委托图无环 | PUT 负例 |
| extraSession | 各级 **原样**透传 | 各层 turn JSON |
| SSE 订阅 | **只**订入口 task（展开另订除外） | 抓包 |
| SSE passthrough | 嵌套时下游 delta **链式**出现在入口流 | 单连接顺序 |
| SSE progress | 入口无正文 delta；有 `biz.delegate.active` | 场景 9 |
| executionMode | v1 serial | 时间线 |
| tool result | `reportPath` + ids；**不含**内嵌 `message` | jsonl |

## 2. 场景（1–9）

| # | 场景 | 示例 | delegate 链 | sid | SSE |
|---|------|------|-------------|-----|-----|
| 1 | 单意图固定 proj 续聊 | 多轮手册问 | R1→kb ×1 | `S_kb` 稳定 | 每轮一块 |
| 2 | 双意图跨轮交替 | T1 手册/T2 问数 | R1→kb/ops | 各稳定 | 每轮单块 |
| 3 | 单意图/轮、多 agent 任意序 | kb→ops→kb… | R1→target | 各稳定 | 无系统随机 |
| 4 | 混合一轮 | 手册+问数同句 | R1→kb → R1→ops | 两 sid | 一条 SSE 两块 |
| 5 | 混合持续 | T1 混合→追问 | 2/1/1 | 复用 | 续聊不断链 |
| 6 | 混合↔单意图交叉 | 2/1/1/2 | 全程 `S_R1` | 块数匹配 |
| **7** | **嵌套（router hub）** | 问数+营销 | R1→R2→ops，再 R2→marketing | 见 §2.1 | 链式 passthrough |
| **8** | **深嵌套** | R1→R2→R3→leaf | 三跳 | root=`S_R1`；直连 R2 另测 | 全文链到 R1 |
| **9** | **progress** | R1 progress，R2 passthrough | R1→R2→leaf | root=`S_R1` | R1 无正文，有 active→R2 |

### 2.1 场景 7：嵌套（router→router→specialist）

**拓扑：**

```text
用户 SSE ← R1 stdout ← R1.delegate 订 R2 live
                         ↑
               R2 stdout ← R2.delegate 订 ops / marketing live
```

**前置：**

- R1、R2 均为 `project_role=router`；ops/marketing 为 `normal`
- R1 targets 含 R2；R2 targets 含 ops、marketing
- **不**在 ops 上开 `delegate_project_tool`

**同轮断言：** link ≥2；root 均为 `S_R1`；嵌套行 parent=`S_R2`；用户只订 R1；链式正文。

### 2.2 场景 8：深嵌套 + 直连正交

- `R1→R2→R3→leaf`，默认 passthrough；所有 link 的 root = `S_R1`
- **另案**：BFF 直连 R2 → root/SSE 锚 `S_R2`（与「R2 也被 R1 挂过」无关）

### 2.3 场景 9：progress

- R1 `bodyRelay=progress`；R2 `passthrough`
- R1 SSE：无 leaf 正文；有 `biz.delegate.active`（指向 R2）；`done` / 重连快照 = 引用桩
- 另订 R2 turn：可见 leaf 正文

## 3. 负例

| 用例 | 期望 |
|------|------|
| projId=999 未登记 | tool 失败 |
| enabled=false | 同上 |
| target=master/observation | 拒绝 |
| initiator=`normal` PUT / resolve | 拒绝 |
| `A→B` 已有，再登记 `B→A` | PUT 环错误 |
| userPrompt 空 | 拒绝 |
| 模型传 sessionId | 拒绝 |

## 4. 最小回归清单

1. 场景 1–6（passthrough 回归）  
2. **场景 7**：R1→R2→ops/marketing  
3. **场景 8**：三跳 + R2 直连正交  
4. **场景 9**：progress 引用桩 + active 事件  
5. 负例：normal 发起、环图、未登记 projId  

配合 [`gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md)。

**本机验收：** [`scripts/gpos-router-split/README-local-test.md`](../scripts/gpos-router-split/README-local-test.md)

## 5. 本期不验

- parallel / Hub 合并  
- master 与用户路由交叉  
- specialist 快问 / Mesh  
- 按边配置 bodyRelay  
