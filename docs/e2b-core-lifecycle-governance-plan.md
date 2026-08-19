# e2b 核心组件生命周期治理计划（先计划后实施）

Author: kejiqing

**分支：** `fix/e2b-core-lifecycle-governance`

**状态：** 实施中（按需重建切片已落地，见 §8–§10）

---

## 1. 背景与问题定义

当前 Gateway 侧 e2b 核心组件（`nas-api`、`observe`、`project warm worker`）存在“配置态”和“运行态”语义不一致的问题，表现为：

- 管理界面显示在线，但组件实际不可用（如 `baseUrl` 存在但 sandbox 已停止）。
- 生命周期动作分散（配置、探活、续租、重建、状态展示不在同一控制面闭环）。
- worker warm 依赖人工触发，缺少统一期望态驱动的自动收敛。
- OVS 已迁移到 relaxed worker 内，但历史 singleton 语义仍可能造成认知与运维误导。

本计划目标是以最小架构边界变动，建立统一的“期望态-观测态-收敛动作”模型，避免继续在业务调用链上做临时兜底。

---

## 2. 架构边界与原则

### 2.1 变更边界（必须遵守）

- 仅改 `http-gateway-rs` 的控制面与状态面：
  - global settings / status API
  - singleton + worker reconcile
  - admin 展示契约
- 不把生命周期修复逻辑塞进 **`exec_gateway_solve_once` / `claw gateway-solve-once` 执行链路**。
- **允许**在 solve / interactive / OVS **执行前**、于 `prepare_e2b_worker_llm_material` **之前**调用统一门闸 `ensure_e2b_runtime_for_proj`（控制面触发，不是 exec 内 recreate）。
- 不引入第二条并行路径（single default path）。
- **不做** ZooKeeper / PG keeper 选主；多 Gateway 继续平等副本 + 已有 `pg_advisory_lock` 串行 create/kill。

### 2.2 原则

- 状态语义真实可核验（展示必须对应运行事实）。
- 生命周期管理统一入口（避免每个组件各自实现一套）。
- 失败可解释（`lastError` + `lastCheckedAtMs`），不静默。
- 先止血再收敛：先修状态语义，再做闭环控制器。

---

## 3. 目标状态（Target State）

为核心组件统一状态模型：

- `configured`: 是否已配置（静态配置态）
- `running`: sandbox 是否运行（平台态）
- `reachable`: 流量/探活是否可达（网络态）
- `healthy`: 组件健康结论（业务态）
- `lastCheckedAtMs`: 最近检查时间
- `lastError`: 最近失败原因（可直接给运维排查）

说明：

- UI 的主状态应使用 `healthy`（或等价字段），`configured` 仅作补充信息。
- 保留兼容字段时要标注 deprecated，避免旧语义继续误导。

---

## 4. 分阶段实施

## Phase A：状态语义纠偏（止血）

### A.1 后端状态扩展

- 扩展 nas-api / observe / worker 状态返回结构，加入统一状态字段。
- 现有 `online` 若保留，仅作为兼容字段；新增字段明确表示实时健康。

### A.2 实时检查接入

- nas-api：`/healthz` + sandbox running 检查。
- observe：对应 live/proxy 健康检查 + sandbox running 检查。
- worker：当前 warm worker 的 running/ready 视图。

### A.3 管理端文案收敛

- “在线”改为“健康/可用”。
- 状态明细可见最近检查时间和错误原因。

### A.4 验收标准

- 组件 sandbox 停止时，状态在一个检查周期内变为不健康。
- 状态响应包含可复核错误（例如 `410 Gone`）。
- 不再出现“配置存在但展示在线”的误导。

---

## Phase B：生命周期闭环（核心治理）

### B.1 统一期望态（Desired State）

- nas-api：required = true（核心依赖）
- observe：required = config 开关决定
- warm worker：每项目 `desiredWarmCount >= 1`（默认 1，可配置）

### B.2 统一 reconcile 控制器

控制器按固定节奏执行：

1. 拉取期望态
2. 采集实际态
3. 计算偏差
4. 执行动作：`noop / renew / recreate / rebind / scale-up`
5. 更新状态与事件

### B.3 失败处理与稳定性

- 幂等动作设计，重复执行不破坏状态。
- 连续失败阈值 + 退避，避免抖动重建。
- 所有失败写入 `lastError` 与操作事件日志。

### B.4 验收标准

- 缺失 singleton 能自动重建并回写新 `sandboxId/baseUrl`。
- warm worker 缺口可自动补齐，无需人工点击“强制重建”。
- 生命周期异常有可追溯状态与日志。

---

## Phase C：OVS 退场收口（单路径）

### C.1 语义收口

- OVS 明确声明：由 relaxed worker 内置提供，不再作为独立 singleton 管理目标。

### C.2 API/UI 收口

- OVS singleton ensure/reset 入口标注 deprecated 或转为只读说明。
- 管理界面不再暗示“需要维护 OVS singleton”。

### C.3 验收标准

- 用户理解路径唯一：OVS = relaxed worker 能力。
- 代码中无活跃 OVS singleton 生命周期分支。

---

## 5. 任务拆分与提交建议

1. 状态模型扩展（不改业务行为）
2. nas-api/observe 实时探活接入
3. 管理端文案与状态展示调整
4. singleton + worker 统一 reconcile
5. OVS singleton 语义退场与文档同步

建议每一步独立提交，便于回滚与评审。

---

## 6. 风险与回滚

### 风险

- 旧前端依赖 `online` 语义可能出现兼容问题。
- 高频探活可能带来额外请求压力。
- 短时网络波动导致误判重建。

### 应对

- 兼容窗口保留旧字段并标注 deprecated。
- 增加探活最小间隔/缓存窗口（例如 15-30s）。
- 连续失败阈值 + 退避策略，避免抖动。

### 回滚

- Phase A/B/C 均可按提交粒度回滚。
- 保证回滚后仍保留旧管理能力，不影响现网 solve 主流程。

---

## 7. 完成定义（Definition of Done）

满足以下条件才算治理完成：

- 展示状态与运行事实一致（无“假在线”）。
- 组件故障可解释（状态含可复核错误证据）。
- 生命周期可自愈（singleton/worker 缺失能自动收敛）。
- OVS 管理路径单一清晰（无历史双路径歧义）。

---

## 8. 按需重建（On-demand Rebuild）— 已拍板架构

### 8.1 职责切分

| 层 | 职责 |
|----|------|
| **e2b 平台** | create / kill / GET / POST timeout；TTL 到期停机；不理解 clawRole |
| **Gateway** | 唯一 e2b 控制面：PG 期望态、sandboxId、续租、探活、重建 |
| **Worker 沙箱** | 执行面；**禁止**调 e2b API、不续自己的租、不重建任何人 |

### 8.2 触发点（同一套 ensure/reconcile，多条入口）

| 触发 | 对象 | 说明 |
|------|------|------|
| 启动 | 系统 nas-api / observe；worker 全量 best-effort | 失败则 Gateway 不应对外服务 |
| 600s 巡检 | **仅**系统 nas-api、系统 observe | 无人请求时保集群核；**已加** role advisory lock |
| **即将使用时** | nas-api +（系统或项目）observe + 将租 worker slot | `ensure_e2b_runtime_for_proj` + `acquire_slot` 探活 |
| Admin reset | 人工强制 | 换镜像 / 急救 |

**不在** async enqueue 时 ensure（避免卡 60s 创建）；**不在** `exec_solve` 里 recreate。

### 8.3 多 Gateway（同 `CLAW_CLUSTER_ID` + 共享 PG）

- TTL 续租：每机 60s ticker，双机幂等冗余，**不做 keeper**。
- create/kill：**必须**持 `pg_advisory_lock`（`cluster:role`、项目 observe、`cluster:proj:slot`）。
- 后到者 wait → 读 PG/e2b → running 则 **adopt**，不重复 create。
- 短窗 last-ok 缓存（15s）减轻双机同时探活；**不替代锁**。

### 8.4 探活容忍（统一决策函数）

实现：`gateway_e2b_lifecycle_decision.rs` — `ProbeVerdict` / `LifecycleAction` / `decide_lifecycle_action`。

| 层 | 规则 |
|----|------|
| 短窗缓存 | last-ok **15s** 内 → ReuseSkipProbe |
| 同次重试 | 3 次（请求路径 sleep 1s） |
| NotRunning | **立刻** Recreate（持锁；worker 看 in_use → Defer） |
| RunningUnreachable | 连续 **2** 次 ensure 失败才 Recreate；第一次 → FailNoKill + 503 |
| force | Admin reset / 启动 `image_refresh` 且 pin 不一致 → 绕过连续失败 |

---

## 9. 切片顺序与不变量（实施记录）

### 9.1 切片顺序（禁止跳步）

```text
0 表征测试     singleton_image_action / needs_recreate / probe_with_retries
1 决策函数接入  gateway_e2b_lifecycle_decision
2 补锁         启动 ensure、600s 巡检、项目 observe advisory lock
3 容忍度       NotRunning 立刻 / RunningUnreachable 连续 2 次
4 请求门闸     ensure_e2b_runtime_for_proj → material 之前
5 worker 探活  acquire_slot warm hit + ensure_warm_worker_running
6 interactive  terminal / OVS / agent 开租前同一 ensure
```

每片门禁：

```bash
cd rust && cargo test -p http-gateway-rs --lib
```

### 9.2 不变量（每片合并前后须成立）

- 每 cluster 仍 1×nas-api + 1×observe；多实例 reap 到 lex 最小 winner
- `image_refresh=false` 时健康沙箱不因新 `buildId` 被杀
- Gateway shutdown **不杀** worker / singleton
- TTL ticker 仍在；续租失败仍只 warn
- worker `in_use` 时 rotate/reset **不 kill**
- `/readyz` 仍只聚合**系统** nas-api + 系统 observe
- Admin `POST .../e2b-singletons/{component}/ensure|reset` JSON 契约不变
- `poolSize=0` 仍是用完 retire
- solve 仍走同一 `exec_solve`，**exec 内零 recreate diff**

### 9.3 代码落点

| 能力 | 模块 |
|------|------|
| 决策函数 | `gateway_e2b_lifecycle_decision.rs` |
| 门闸 | `ensure_e2b_runtime_for_proj` in `gateway_e2b_singleton_lifecycle.rs` |
| solve 接线 | `solve_pool.rs`（material 前） |
| interactive | `session_terminal_api.rs`、`session_agent_api.rs`、`session_ovs_api.rs` |
| worker 热路径 | `e2b_proj_worker_registry.rs` — `ensure_warm_worker_running` |
| 项目 observe 锁 | `session_db.rs` — `with_e2b_project_observe_lock` |

---

## 10. 环境验收清单（对照运行中 release，非宿主机 checkout）

**证据优先级：** 运行中容器/镜像 tag → healthz/readyz/Admin JSON → 日志；不以宿主机 git 树为真相。

在目标环境（单机即可；双机为加分项）合并后执行：

### 10.1 集群核健康

1. `GET /readyz` → 200；`e2bCore.nas_api` / observe 与改前一致为健康。
2. Admin `GET .../e2b-singletons`：在一次 **ensure**（非 reset）后 **sandboxId 不变**（adopt，非误杀重建）。

### 10.2 请求路径

3. 健康路径一次 `/v1/solve` 或 async solve 成功；日志无意外 `singleton recreate` / `proj worker rotate`。
4. Terminal / OVS workspace 开租前经同一门闸；死 nas-api 时返回 503 而非 exec 才爆。

### 10.3 强制与容忍

5. 人工 `POST .../reset` 仍能换新 sandboxId（force 路径未被容忍度误伤）。
6. **容忍度**（预发可选）：对 **running** 沙箱制造一次 HTTP 超时 → 日志应为 retry/FailNoKill，**不应**紧接 kill+create；真删沙箱（NotRunning）才 recreate。

### 10.4 多 Gateway（可选）

7. 双机同时 solve：仅一次 create；后到 adopt PG sandboxId。
8. A 重建 worker 后 B warm hit 探活 → adopt 新 id，不对 dead id exec。

### 10.5 记录模板

| 项 | 环境 | 镜像 tag | 结果 | 证据 |
|----|------|----------|------|------|
| readyz | | | | |
| ensure 不换 id | | | | |
| solve | | | | |
| reset | | | | |

本地无 e2b 环境时：以 `cargo test -p http-gateway-rs --lib` 全绿 + 上表在预发/252/94 填证。

