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

## 代码

- `rust/crates/gateway-solve-turn/src/mcp_call_context.rs` — `GatewayMcpCallContext::to_mcp_meta()` / `build_mcp_call_meta`
- 常量：`CLAW_EXTRA_SESSION_SESSION_ID`、`CLAW_EXTRA_SESSION_TURN_ID`

## 与 HTTP header

| 链路 | 关联 |
|------|------|
| 上游 LLM | `clawcode-session-id` / `claw-session-id` = `sessionId`（与 `_claw_session_id` 同值） |
| 下游 MCP | 仅 `_meta.extra_session`（上表），非 MCP 出站 HTTP header |

`requestId` / NDJSON trace 仍在网关 worker 内使用（`GatewayMcpCallContext.request_id` / `trace_id`），**不**写入 MCP `_meta`。
