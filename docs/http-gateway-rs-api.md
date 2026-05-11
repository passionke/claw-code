# HTTP Gateway RS API

本文档是 Rust 网关对外 HTTP 接口清单，保持与运行代码一致。

Base URL 示例：`http://127.0.0.1:18088`

## Health

- `GET /healthz`
  - 用途：健康检查与关键运行配置回显

## Solve

- `POST /v1/solve`
  - 用途：同步执行一次 solve
  - 会话 ID 约定：
    - 优先使用请求头 `claw-session-id`
    - 若未传入，则网关生成一个并回写响应头 `claw-session-id`
    - 兼容读取 `x-request-id`（当 `claw-session-id` 缺失时）
  - 请求体字段：
    - `dsId`：必填，数据源 ID，整数且需 `>= 1`
    - `userPrompt`：必填，用户自然语言输入，非空字符串
    - `model`：可选，指定使用的模型标识，缺省走网关默认模型
    - `timeoutSeconds`：可选，整体超时时间（秒）
    - `extraSession`：可选，**JSON 对象**，业务会话级上下文（例如用户、租户、workspace 等标识）
      - 若存在但不是 object，将返回 `400`（`extraSession must be a JSON object when present`）
      - 序列化后大小上限约为 `8KB`，超出将返回 `400`（`extraSession is too large (max 8KB)`）
    - `allowedTools`：可选，字符串数组，指定**本次 solve**允许暴露给模型并执行的工具名（与异步 `/v1/solve_async` 相同）。
      - **未传 `allowedTools`**：沿用网关进程环境变量 `CLAW_ALLOWED_TOOLS`（逗号分隔，与 `GET /healthz` 中 `allowedTools` 字段一致）。若该环境变量也未配置（空），则下游 `gateway-solve-turn` 将空列表视为「不额外收紧」，**内置 MVP 工具（含 `bash` 等）会全部挂上**。
      - **已配置全局 `CLAW_ALLOWED_TOOLS`（非空）**：请求里的 `allowedTools` 若出现，则其中**每一项**都必须被全局策略放行（支持前缀通配：全局项若以 `*` 结尾，则匹配该前缀开头的工具名；请求侧若以 `*` 结尾，则该项须与全局列表中的某一项字面完全一致）。否则返回 `400`，提示 `requested tool pattern is not allowed by gateway policy`。因此若部署里把全局白名单写得很窄（例如仅 `read_file,glob_search`），仅靠请求体无法「偷偷」加上 `bash`，需要先把 **`CLAW_ALLOWED_TOOLS` 扩大到包含 `bash`（或 `bash*`）等**。
      - 常见内置名（与 `rust/crates/tools` 中 `mvp_tool_specs` 一致，按需选用）：`bash`、`read_file`、`write_file`、`edit_file`、`glob_search`、`grep_search`、`WebFetch`、`WebSearch`、`MCP`、`Skill`、`TodoWrite` 等；MCP 动态工具名按运行时注册为准。
      - 典型「交给 resolve/solve 里强 agent 自决」时，在**全局白名单已包含**的前提下，可在一次调用里显式放宽，例如：`"allowedTools": ["read_file","glob_search","grep_search","bash","write_file","edit_file","MCP"]`。
  - 追踪约定：
    - 网关会为本次调用确定 `sessionId`（等于 `claw-session-id`）
    - 响应体主字段使用 `sessionId`，并保留 `requestId` 兼容字段（同值）
    - 在访问上游模型时透传 HTTP 头：
      - `clawcode-session-id: <sessionId>`
      - `claw-session-id: <sessionId>`
    - 在访问下游 MCP 服务（包括 SQLBot）时，会通过 MCP 协议 `tools/call._meta.extra_session` 向工具端暴露 `extraSession`（如存在），用于会话级业务上下文消费。

- `POST /v1/solve_async`
  - 用途：异步提交 solve 任务，返回 `taskId`
  - ID 约定：`taskId` 与 `sessionId` 为同一个值（同一逻辑会话 ID），用于统一追踪与轮询。
  - 响应兼容：同时返回 `requestId`（值等于 `sessionId`）
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

## MCP

- `POST /v1/mcp/inject`
  - 用途：为指定 `dsId` 注入 `mcpServers`

- `GET /v1/mcp/injected/{ds_id}`
  - 用途：查看 `dsId` 对应 MCP 注入及加载结果

- `DELETE /v1/mcp/injected/{ds_id}`
  - 用途：删除 `dsId` 对应 MCP 注入（支持按名称删除）
