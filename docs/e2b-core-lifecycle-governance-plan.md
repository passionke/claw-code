# e2b 核心组件生命周期治理计划（先计划后实施）

Author: kejiqing

**分支：** `fix/e2b-core-lifecycle-governance`

**状态：** Draft（待评审）

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
- 不把生命周期修复逻辑塞进 solve 执行链路。
- 不引入第二条并行路径（single default path）。

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

