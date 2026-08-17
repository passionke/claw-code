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

---

v1 以本节为 **验收真源**。实现服务于可观测断言，不以内部复杂度为准。

## 1. 全局不变量

| 维度 | 标准 | 验证 |
|------|------|------|
| 用户 session | 续聊同一 `sessionId` → router；新开 = 新 sid | DB / `GET /v1/sessions` |
| 模型不传 sid | 各级 `delegate_project` 均无 `sessionId` | tool 日志 |
| root 锚点 | **所有** link 行 `root_session_id` = 用户 router session（含嵌套跳） | SQL |
| sid 复用 | 同 `(parent_session_id, parent_proj_id, delegate_proj_id)` 复用 delegate sid | 多轮对比 |
| sid 隔离 | 不同 target / 不同 parent 上下文不得串 sid | 多行 link 对比 |
| allowlist | 每级发起方仅可 delegate 其 `delegate-targets` 内 proj | 负例 |
| extraSession | 各级 **原样**透传 | 各层 turn JSON |
| userPrompt | 单意图原问；混合问子问 | specialist 日志 |
| SSE 订阅 | **只**订 router task | 抓包 |
| SSE 内容 | 嵌套时下游 delta **链式**出现在 router 流；同级无交错 | 单连接顺序 |
| executionMode | v1 serial：每级 tool 顺序完成 | 时间线 |

## 2. 场景（1–7）

| # | 场景 | 示例 | delegate 链 | sid | SSE |
|---|------|------|-------------|-----|-----|
| 1 | 单意图固定 proj 续聊 | 多轮手册问 | router→kb ×1 | `S_kb` 稳定 | 每轮一块 |
| 2 | 双意图跨轮交替 | T1 手册/T2 问数/T3 手册 | router→kb/ops 各 1 | `S_kb`,`S_ops` 各稳定 | 每轮单块 |
| 3 | 单意图/轮、多 agent 任意序 | T1 kb→T2 ops→T3 kb… | router→target ×1/轮 | 各 target sid 稳定 | 无系统随机 |
| 4 | 混合一轮 | 手册+问数同句 | router→kb → router→ops（serial） | 两 target 各 sid | 一条 SSE 两块 |
| 5 | 混合持续 | T1 混合→T2 追问问数→T3 追问手册 | 2/1/1 | 复用 T1 sid | 续聊不断链 |
| 6 | 混合↔单意图交叉 | T1 混合→…→T4 再混合 | 2/1/1/2 | 全程 `S_router` | 块数匹配 |
| **7** | **嵌套 delegate** | 用户问数 → router→ops → ops 再→marketing（同轮） | router→ops→marketing | 见 **§2.1** | 用户仍只连 router；marketing 正文经 ops 链式 passthrough |

场景 2 vs 3：2 为两 proj 严格交替；3 为多 agent 任意顺序。均非 gateway 随机派单。

### 2.1 场景 7：嵌套 delegate（本期必验）

**拓扑（示例）：**

```text
用户 SSE ← router stdout ← router.delegate_project 订 ops live
                              ↑
                    ops stdout ← ops.delegate_project 订 marketing live
                                      ↑
                              marketing solve
```

**前置：**

- ops（271）物化 `delegate_project`；kb **不**开
- `PUT /v1/projects/{ops}/delegate-targets` 登记 marketing（`project_role=normal`）
- router 的 `delegate-targets` 仍含 ops；**不**要求 router 直接登记 marketing

**同轮断言：**

| 项 | 期望 |
|----|------|
| link 行数 | ≥2：` (S_router, router, ops)→S_ops`；`(S_ops, ops, marketing)→S_marketing` |
| root | 两行 `root_session_id` 均为 `S_router` |
| parent | 嵌套行 `parent_session_id=S_ops`（非 S_router） |
| SSE | 单连接；先/后块顺序由 ops skill 决定；**无** marketing task 对用户暴露 |
| 墙钟 | marketing 阻塞 ops，ops 阻塞 router（serial 叠乘，接受） |
| 负例 | ops 未登记 marketing → ops 层 tool 失败；router SSE 可见错误；无 orphan marketing turn |

**续聊（场景 7+）：** 第二轮仍走 ops→marketing 时，`S_ops` / `S_marketing` 分别复用。

## 3. 负例

| 用例 | 期望 |
|------|------|
| projId=999 未登记 | tool 失败；无 specialist turn |
| enabled=false | 同上 |
| target=master/observation | 拒绝 |
| userPrompt 空 | 拒绝 |
| 模型传 sessionId | 忽略或拒绝（实现写死） |
| router 改 extraSession | specialist 收到与 BFF 一致 |

## 4. 最小回归清单

1. 场景 1：3 轮 kb，`S_kb` 不变  
2. 场景 3：kb→ops→kb，`S_kb`/`S_ops` 稳定  
3. 场景 4：混合一句，单 SSE ≥2 段  
4. 场景 5：混合后 ops 追问，`S_ops` 同 T1  
5. 场景 6：2/1/1/2 块数  
6. **场景 7**：router→ops→marketing 同轮；assert link 两行、root 均为 `S_router`、parent 嵌套行=`S_ops`、单 SSE 链式正文  
7. 负例：未登记 projId  

配合 [`gpos-intent-routing-regress.md`](gpos-intent-routing-regress.md) 意图冒烟。

**本机验收（不碰预发）：** [`scripts/gpos-router-split/README-local-test.md`](../scripts/gpos-router-split/README-local-test.md)

## 5. 本期不验

- parallel / Hub 合并  
- master 与用户路由交叉  
- specialist 快问  
- Mesh 协同  
- kb 发起嵌套 delegate（本期仅 **ops→下游** 一条嵌套链路必验）  
