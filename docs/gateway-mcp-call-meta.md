# Gateway MCP `tools/call` 串联元数据

Author: kejiqing

网关 solve 在每次 MCP `tools/call` 的 `params._meta` 中只带一个字段：**`extra_session`**。业务字段与网关关联 id 放在同一对象里，避免多层 `claw` / 扁平别名。

## 形状

```json
{
  "extra_session": {
    "store_id": "…",
    "org_id": "",
    "tenant_code": "GPOS",
    "_claw_session_id": "<sessionId>",
    "_claw_turn_id": "<turnId>"
  }
}
```

| 键 | 来源 |
|----|------|
| 其它键 | HTTP 请求体 `extraSession`（经 `normalize_extra_session`，缺省含 `org_id: ""`） |
| `_claw_session_id` | 会话 id（`claw-session-id`；任务文件 `sessionId` 优先于 `requestId`） |
| `_claw_turn_id` | 当次 `turnId`（`T_<32位小写 hex>`） |

下划线前缀表示网关注入，避免与业务 `session_id` / `turn_id` 冲突。

## 下游读取

从 MCP `tools/call` 的 `params._meta.extra_session` 读取即可；**不要**再解析顶层 `session_id` / `claw` 信封（已移除）。

## Resolve 入口（统一解析）

从 HTTP / 任务文件字段解析并规范化上下文，使用：

- `gateway_solve_turn::resolve_gateway_mcp_call_context(session_id, turn_id, request_id, extra_session)`
- `gateway_solve_turn::gateway_mcp_call_context_from_task(&task)`

注入 MCP `_meta` 时统一调用 `inject_mcp_call_meta(&ctx)`（`runtime::McpCallContext`）。

## Subagent（`Agent` 工具）

主 solve turn 通过 `DirectToolExecutor` 调用 `Agent` 时，会将父 turn 的 `McpCallContext` **克隆**进子线程（`AgentJob.mcp_call_context`）。子代理内 `MCP` 工具调用与主 agent 使用相同的 `_claw_session_id` / `_claw_turn_id`（**继承主 turnId**，不派生子 turn）。

`allowed_tools_for_subagent` 仍按 subagent 类型限制可用工具；本机制仅保证「允许调用 MCP 时」`_meta.extra_session` 不为空。

## 代码

- `rust/crates/runtime/src/mcp_call_context.rs` — `McpCallContext`、`build_mcp_call_meta`、`inject_mcp_call_meta`
- `rust/crates/gateway-solve-turn/src/mcp_call_context.rs` — `resolve_gateway_mcp_call_context`、`gateway_mcp_call_context_from_task`
- `rust/crates/tools/src/lib.rs` — `execute_agent_with_mcp_context`、`SubagentToolExecutor`
- 常量：`CLAW_EXTRA_SESSION_SESSION_ID`、`CLAW_EXTRA_SESSION_TURN_ID`

## 与 HTTP header

| 链路 | 关联 |
|------|------|
| 上游 LLM | `clawcode-session-id` / `claw-session-id` = `sessionId`（与 `_claw_session_id` 同值） |
| 下游 MCP | 仅 `_meta.extra_session`（上表），非 MCP 出站 HTTP header |

`requestId` / NDJSON trace 仍在网关 worker 内使用（`McpCallContext.request_id` / `trace_id`），**不**写入 MCP `_meta`。
