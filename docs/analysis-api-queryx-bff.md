# Boss 报表分析 · QueryX 风格 BFF 系分接口（设计稿）

本文档描述面向前端 / 业务方的 **QueryX 风格** HTTP 接口约定，及其与 **claw-code HTTP 网关**（Rust `http-gateway-rs`）内部接口的对应关系。网关 Base URL 与路径前缀以实际部署为准；分析类路径以 `/api/v1/analysis` 为前缀；**准入**为独立资源 `/api/v1/admittance`。

**上一版文档（钉钉）：** [https://alidocs.dingtalk.com/i/nodes/14lgGw3P8vvea74PhQNo43mk85daZ90D](https://alidocs.dingtalk.com/i/nodes/14lgGw3P8vvea74PhQNo43mk85daZ90D)

## 固定业务维度（本对接场景）

| 字段 | 取值 | 说明 |
|------|------|------|
| `tenant_code` | `GPOS` | 租户编码，本项目中与钉钉 / 业务侧约定一致 |
| `solution_code` | `restaurant` | 解决方案编码，本对接写死 |
| `biz_type` | `BOSS_REPORT` | 业务类型，Boss 报表场景 |

上述字段在请求中**必须携带**（Query 或 Body 按各接口约定），便于网关日志、路由与多租户隔离策略统一落标。

---

## 公共参数（QueryX 风格）

### 分析链路（async / report / status）

以下参数在三条分析接口上**语义一致**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `tenant_code` | string | 是 | 固定 `GPOS` |
| `solution_code` | string | 是 | 固定 `restaurant` |
| `biz_type` | string | 是 | 固定 `BOSS_REPORT` |
| `store_id` | string | 是 | 门店等业务主键；BFF 在调网关时将其纳入扩展上下文（如 `extraSession`），调用方仅传本字段 |
| `sessionId` | string | 见各接口 | 逻辑会话 ID，与异步任务 ID 串联，见下文「sessionId 串联逻辑」 |

### 准入（admittance）

准入接口**仅**携带租户与业务维度及门店，**不要求** `sessionId`（尚无会话）。

---

## 接口零（升级）：`GET /api/v1/admittance`

| 项目 | 说明 |
|------|------|
| **用途** | 在进入分析流程前，查询当前租户 + 解决方案 + 业务类型 + 门店是否**允许使用** Boss 报表分析能力（产品/合同/灰度等规则由业务决定）。 |
| **与 claw 网关** | **无直接一一路由**；建议由 **BFF 或独立策略服务**实现（可调主数据、配置中心、运营白名单等）。若未来下沉到网关，再补充映射表。 |

**Query 参数（全部为 QueryString，设计约定）**

| 参数 | 必填 | 说明 |
|------|------|------|
| `tenant_code` | 是 | 本对接场景为 `GPOS` |
| `solution_code` | 是 | 本对接场景为 `restaurant` |
| `biz_type` | 是 | 本对接场景为 `BOSS_REPORT` |
| `store_id` | 是 | 门店 ID；准入规则通常按门店或门店所属组织判定 |

**缺参行为（设计约定）**  

任一 Query 缺失或为空字符串时，返回 **400**，响应体建议为统一错误结构（如 `{ "detail": "..." }`），便于前端表单校验。

**成功响应（JSON）**

| 字段 | 类型 | 说明 |
|------|------|------|
| `admittance` | boolean | **`true`** 表示准入，可继续调用 `POST /api/v1/analysis/async` 等；**`false`** 表示不准入，前端应展示无权/未开通等文案，**不应**再发起分析请求。 |

**可选扩展（设计留白，实现阶段再定）**  

- 是否在 `false` 时附带 `reason` / `reason_code`（枚举）供埋点与文案。  
- 是否返回 `effective_until`（权益截止时间）等；若增加字段，建议版本化或保持向后兼容。

**与后续接口的调用顺序（建议 UX）**  

1. 页面进入或点击「开始分析」前：`GET /api/v1/admittance` → 仅当 `admittance === true` 再展示输入框并允许调用 async。  
2. `admittance === false` 时，不创建会话、不调 async，避免无效任务与计费。

---

## sessionId 串联逻辑

1. **首次分析**  
   客户端调用 `POST /api/v1/analysis/async` 时，可不传 `sessionId`（或由 BFF 不传体字段 `sessionId`）。  
   - BFF 解析 `dsId` 后，将请求转发为网关 `POST /v1/solve_async`；若客户端也未带头，网关会生成新的会话 ID。  
   - 网关约定：**`taskId` 与 `sessionId` 为同一值**（见 `docs/http-gateway-rs-api.md`）。  
   - 客户端应以响应中的 **`taskId` / `sessionId` / 响应头 `claw-session-id`** 之一作为后续轮询与续聊的唯一键（三者一致时取其一即可）。

2. **同店续聊 / 多轮**  
   下一次 `async` 或报表请求须带上**上一次返回的 `sessionId`**，并保持与首聊相同的 **`store_id`**（及同一租户 / 解决方案 / 业务类型），以便 BFF **解析出与首聊相同的 `dsId`** 再调网关。  
   - BFF 映射为网关请求体 `sessionId`：表示显式续聊，复用 SQLite 中 `(sessionId, dsId)` 对应工作区与对话历史。  
   - 若传入的 `sessionId` 在库中不存在：网关返回 **400**（`unknown sessionId (no session history for this dsId)`）。

3. **与任务状态查询**  
   - 轮询 `GET /api/v1/analysis/status` 时，使用 **`task_id` = 上次 `async` 返回的 `taskId`**（即与 `sessionId` 同值）。  
   - 拉取清洗报表 `GET /api/v1/analysis/report` 时，使用同一 **`task_id`**（对应网关 `GET /v1/biz_advice_report?task_id=...`）。

4. **并发约束**  
   同一 `sessionId` 上若仍有 `queued` / `running` 的异步任务，再次提交 `solve_async` 会得到 **409**；前端应等待 `status` 为终态后再发起新的 async。

5. **请求头（可选增强）**  
   与网关一致时，可透传 `claw-session-id` / `x-request-id` 与体字段 `sessionId` 对齐；冲突时网关返回 **400**。BFF 层应保证三者与业务约定的 `sessionId` 一致。

---

## 接口一：`POST /api/v1/analysis/async`

| 项目 | 说明 |
|------|------|
| **用途** | 异步提交 Boss 报表分析（自然语言问数 / 推理） |
| **映射 claw 网关** | `POST /v1/solve_async`（文档亦称 resolve_async 语义）；网关请求体仍含 `dsId`，**由 BFF 根据租户与门店等解析或配置映射写入，不对外暴露**。 |

**建议请求体（JSON）——对外契约**

调用方**不填 `dsId`**。除 QueryX 公共字段外，仅需业务可见字段；`dsId` 仅在 BFF → 网关链路中出现。

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `tenant_code` / `solution_code` / `biz_type` | string | 是 | 见上表固定值 |
| `store_id` | string | 是 | 与 BFF 侧 **`dsId` 解析规则**绑定（同一门店须稳定映射到同一 `dsId`）；BFF 调网关时附带门店等上下文，**不在对外体中暴露** `extraSession` |
| `sessionId` | string | 否 | 续聊必填；首聊省略则由网关生成 |
| `question` | string | 是 | 用户自然语言问题，非空；BFF 映射为网关请求体字段 **`userPrompt`** |

**BFF 侧（设计约定，非对外字段）**  

- 根据 `tenant_code`、`solution_code`、`biz_type`、`store_id`（及可选内部配置表）解析出整数 **`dsId`（≥ 1）**，再组装网关 `POST /v1/solve_async` 请求体。  
- 将 **`question` → `userPrompt`** 写入网关体。  
- **`model` / `timeoutSeconds` / `extraSession`**：对外接口**不提供**；若网关或下游仍需要，由 BFF 使用部署侧默认模型、超时及内部组装的 `extraSession`（如含 `store_id`、租户标识等），**不**由前端传参控制。  
- 若无法解析 `dsId`（未知门店、未绑定数据源等）：BFF 应对外返回 **4xx** 及明确 `detail`，**不**调用网关。

**成功响应（示意）**  

与网关 `solve_async` 对齐：`taskId`、`requestId`、`status`（如 `queued`）、`pollUrl` 等；其中 **`taskId` 即后续 `status` / `report` 使用的任务键，且等于会话 `sessionId`**。

---

## 接口二：`GET /api/v1/analysis/report`

| 项目 | 说明 |
|------|------|
| **用途** | 在异步任务 **成功** 后，获取清洗后的业务报告（去除中间过程与工具轨迹） |
| **映射 claw 网关** | `GET /v1/biz_advice_report?task_id=<taskId>` |

**Query 参数（QueryX 风格）**

| 参数 | 必填 | 说明 |
|------|------|------|
| `tenant_code` | 是 | `GPOS` |
| `solution_code` | 是 | `restaurant` |
| `biz_type` | 是 | `BOSS_REPORT` |
| `store_id` | 是 | 与发起分析时一致，便于 BFF 校验与审计 |
| `sessionId` | 建议 | 与当时异步会话一致，便于日志关联（网关仅以 `task_id` 取报告） |
| `task_id` | 是 | 等于 `async` 返回的 `taskId` |

**行为说明**  

- 网关要求任务状态为 **`succeeded`**；否则 **400**。  
- 返回字段与网关一致：`taskId`、`sourceRequestId`、`sourceDsId`、`sourceStatus`、`reportText`、`reportJson` 等。

---

## 接口三：`GET /api/v1/analysis/status`

| 项目 | 说明 |
|------|------|
| **用途** | 查询异步分析任务状态与结果 |
| **映射 claw 网关** | `GET /v1/tasks/{task_id}` |

**Query 参数（QueryX 风格）**

| 参数 | 必填 | 说明 |
|------|------|------|
| `tenant_code` | 是 | `GPOS` |
| `solution_code` | 是 | `restaurant` |
| `biz_type` | 是 | `BOSS_REPORT` |
| `store_id` | 是 | 业务侧门店，用于 BFF 审计 |
| `sessionId` | 建议 | 与 `task_id` 同源会话，便于串联 |
| `task_id` | 是 | 与 `async` 返回的 `taskId` 相同 |

**响应扩展字段**

| 字段 | 类型 | 说明 |
|------|------|------|
| `current_task_desc` | string | 网关 `GET /v1/tasks/{task_id}` 的 **`currentTaskDesc`**（BFF 映射为本字段）：agent 通过 `report_progress` 写的用户向进度句；排队态由网关生成；**不**暴露 SQLBot/MCP 等内部工具名 |

> **实现说明**：权威来源为 `http-gateway-rs` 任务轮询；可选 `GET /v1/sessions/{sessionId}/execution?ds_id=` 获取 `progress` / `progressHistory` / `queue`。

---

## 与网关文档对照

权威网关行为与字段细节以仓库内 **`docs/http-gateway-rs-api.md`** 为准；本文侧重 **QueryX 对外契约** 与 **session / task 串联**。

---

## 修订记录

| 日期 | 说明 |
|------|------|
| 2026-05-16 | `current_task_desc` 由网关 `currentTaskDesc` 提供（`report_progress` + 排队/兜底） |
| 2026-05-12 | 初稿：async / report / status 三条接口、固定租户与业务维度、`sessionId` 串联、`current_task_desc` 约定 |
| 2026-05-12 | 标明设计稿性质；新增 `GET /api/v1/admittance`（入参四元组、返回 `admittance`）、与分析及调用顺序说明 |
| 2026-05-12 | 对外 `POST /api/v1/analysis/async`：`userPrompt` 改为 `question`；不再暴露 `model` / `timeoutSeconds` / `extraSession`，由 BFF 内部默认或组装 |
| 2026-05-12 | 文首补充上一版钉钉系分文档链接 |
