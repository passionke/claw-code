# HTTP Gateway RS API

本文档是 Rust 网关对外 HTTP 接口清单，保持与运行代码一致。

Base URL 示例：`http://127.0.0.1:18088`

## Health

- `GET /healthz`
  - 用途：健康检查与关键运行配置回显

## Solve

- `POST /v1/solve`
  - 用途：同步执行一次 solve
  - 请求体字段：`dsId`、`userPrompt`、可选 `model`、可选 `timeoutSeconds`
  - 追踪约定：网关会为本次调用生成 `requestId`，并在访问上游模型时透传 HTTP 头 `clawcode-session-id: <requestId>` 与 `claw-session-id: <requestId>`

- `POST /v1/solve_async`
  - 用途：异步提交 solve 任务，返回 `taskId`
  - 追踪约定：异步调用同样透传 `clawcode-session-id` 与 `claw-session-id`（值均为该次任务的网关层请求 ID）

- `GET /v1/tasks/{task_id}`
  - 用途：查询异步任务状态与结果

## MCP

- `POST /v1/mcp/inject`
  - 用途：为指定 `dsId` 注入 `mcpServers`

- `GET /v1/mcp/injected/{ds_id}`
  - 用途：查看 `dsId` 对应 MCP 注入及加载结果

- `DELETE /v1/mcp/injected/{ds_id}`
  - 用途：删除 `dsId` 对应 MCP 注入（支持按名称删除）
