# HTTP Gateway RS API

本文档是 Rust 网关对外 HTTP 接口清单，保持与运行代码一致。

Base URL 示例：`http://127.0.0.1:18088`

## Health

- `GET /healthz`
  - 用途：健康检查与关键运行配置回显
  - 回显字段含 `sessionDbPath`：网关 **SQLite** 会话索引文件路径（`sessionId` ↔ 工作目录相对路径）。由环境变量 `CLAW_GATEWAY_SESSION_DB` 指定（推荐宿主机持久路径或 volume 挂载）；未设置时默认为 `CLAW_WORK_ROOT/gateway-sessions.sqlite`（与 workspace 同卷时需保证 `CLAW_WORK_ROOT` 已持久挂载）。

## Solve

- `POST /v1/solve`
  - 用途：同步执行一次 solve
  - 会话 ID 约定：
    - **有效 `sessionId`**：请求体可选字段 `sessionId`（非空）优先；否则使用请求头 `claw-session-id`；再否则 `x-request-id`；皆无则网关生成 UUID。响应头 `claw-session-id` / `x-request-id` 与响应体 `sessionId` / `requestId` 与有效值一致。
    - 若请求头已带 `claw-session-id` 或 `x-request-id`，且请求体 **`sessionId` 与头不一致**，返回 `400`（`sessionId conflicts with claw-session-id or x-request-id header`）。
  - **续聊与路径**：网关将 `(sessionId, dsId)` 与工作区目录的映射写入 SQLite。请求体传入 **`sessionId` 且非空** 表示显式续聊：若库中无该 `(sessionId, dsId)` 行，返回 `400`（`unknown sessionId (no session history for this dsId)`）。未传 `sessionId` 但头里携带的会话在库中已有记录时，会 **复用同一工作目录** 与磁盘上的对话 jsonl，实现多轮连续。
  - 请求体字段：
    - `dsId`：必填，数据源 ID，整数且需 `>= 1`
    - `userPrompt`：必填，用户自然语言输入，非空字符串
    - `sessionId`：可选，非空时表示按该 id 续聊（须已在库中有历史）；与头冲突规则见上
    - `model`：可选，指定使用的模型标识，缺省走网关默认模型
    - `timeoutSeconds`：可选，整体超时时间（秒）
    - `extraSession`：可选，**JSON 对象**，业务会话级上下文（例如用户、租户、workspace 等标识）
      - 若存在但不是 object，将返回 `400`（`extraSession must be a JSON object when present`）
      - 序列化后大小上限约为 `8KB`，超出将返回 `400`（`extraSession is too large (max 8KB)`）
  - 追踪约定：
    - 网关会为本次调用确定 `sessionId`（等于 `claw-session-id`）
    - 响应体主字段使用 `sessionId`，并保留 `requestId` 兼容字段（同值）
    - 在访问上游模型时透传 HTTP 头：
      - `clawcode-session-id: <sessionId>`
      - `claw-session-id: <sessionId>`
    - 在访问下游 MCP 服务（包括 SQLBot）时，会通过 MCP 协议 `tools/call._meta.extra_session` 向工具端暴露 `extraSession`（如存在），用于会话级业务上下文消费。
  - 对话状态：同一会话目录下使用 `.claw/gateway-solve-session.jsonl` 持久化消息；若文件损坏导致无法加载，返回 `500`（不会静默丢弃历史）。

- `POST /v1/solve_async`
  - 用途：异步提交 solve 任务，返回 `taskId`
  - ID 约定：`taskId` 与 `sessionId` 为同一个值（同一逻辑会话 ID），用于统一追踪与轮询。
  - 响应兼容：同时返回 `requestId`（值等于 `sessionId`）；响应头 `claw-session-id` / `x-request-id` 与有效 `sessionId` 一致（与 `/v1/solve` 相同合并规则）。
  - **显式续聊**：请求体带非空 `sessionId` 时，若库中无该 `(sessionId, dsId)`，在入队前返回 `400`（文案同同步接口）。
  - **串行**：同一 `sessionId` 已存在状态为 `queued` 或 `running` 的异步任务时，再次 `POST /v1/solve_async` 返回 **`409 Conflict`**（`session has active async task`），需等待完成或取消后再提交。
  - 追踪约定：异步调用同样透传 `clawcode-session-id` 与 `claw-session-id`（值均为该次任务的网关层会话 ID）

- `GET /v1/tasks/{task_id}`
  - 用途：查询异步任务状态与结果

- `POST /v1/tasks/{task_id}/cancel`
  - 用途：按 `taskId`（与异步会话 `sessionId` 同值）取消仍处于 `queued` 或 `running` 的 solve 异步任务
  - 成功时：任务状态变为 `cancelled`，`finishedAtMs` 写入，`error` 为 `{"detail":"cancelled by client"}`
  - 若任务已是 `succeeded` / `failed` / `cancelled`：返回 `400`
  - 若 `task_id` 未知：返回 `404`
  - 说明：取消通过中止网关侧异步 worker 实现；若当前正阻塞在长时间同步推理 `run_turn` 中，可能要等该段同步逻辑返回后 worker 才会结束，但**不会**再用成功结果覆盖已为 `cancelled` 的状态

- `GET /v1/biz_advice_report?task_id=<taskId>`
  - 用途：基于异步任务原始输出，生成清洗后的最终业务报告（去除中间过程与工具轨迹）
  - 查询参数：
    - `task_id`：必填，`/v1/solve_async` 返回的任务 ID
  - 前置条件：
    - 目标任务状态必须是 `succeeded`
    - 若任务为 `queued/running/failed`，返回 `400`
  - 返回字段：
    - `taskId`：目标任务 ID
    - `sourceRequestId`：原任务 requestId
    - `sourceDsId`：原任务 dsId
    - `sourceStatus`：原任务状态（通常为 `succeeded`）
    - `reportText`：清洗后的报告文本（字符串）
    - `reportJson`：清洗后的结构化 JSON（如模型返回 JSON）

## Skills（按 ds 工作区）

技能文件约定：`<CLAW_WORK_ROOT>/ds_<dsId>/.claw/skills/<skill_name>/SKILL.md`（与 Claw 技能目录布局一致）。`skill_name` 为目录名，不含 `/`、`\` 或 `..`。

- `GET /v1/skills/{ds_id}`
  - 用途：列出该 `dsId` 下所有已存在的技能（仅包含存在 `SKILL.md` 的子目录）
  - 成功响应 JSON：`{ "ds_id": <int>, "skills": [ { "skill_name": "<str>", "skill_content": "<str>" }, ... ] }`（按 `skill_name` 排序）

- `GET /v1/skills/{ds_id}/{skill_name}`
  - 用途：读取单个技能的完整 `SKILL.md` 文本
  - 成功响应 JSON：`{ "ds_id": <int>, "skill_name": "<str>", "skill_content": "<str>" }`
  - 若文件不存在：返回 `404`

## MCP

- `POST /v1/mcp/inject`
  - 用途：为指定 `dsId` 注入 `mcpServers`

- `GET /v1/mcp/injected/{ds_id}`
  - 用途：查看 `dsId` 对应 MCP 注入及加载结果

- `DELETE /v1/mcp/injected/{ds_id}`
  - 用途：删除 `dsId` 对应 MCP 注入（支持按名称删除）
