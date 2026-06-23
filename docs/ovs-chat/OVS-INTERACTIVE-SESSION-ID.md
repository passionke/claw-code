# OVS 交互式 `@claw`：会话 ID 与 Tap 对齐

Author: kejiqing

**原则：不新造 session 模型。** 复用 Admin 对话里已有的 `sessionId`（`record_session_id`）、solve 已有的 LLM header 契约、claude-tap 已有的 `claw-session-id` 索引。

**多轮上下文续聊：** 已落地 B1（`session_agent_api` exec + `interactive-session.jsonl` resume）。详见 **[OVS-INTERACTIVE-CONTEXT-PLAN.md](./OVS-INTERACTIVE-CONTEXT-PLAN.md)**。

---

## 三种 ID（不要混用）

| 名称 | 示例 | 用途 | 是否给 tap / Admin Tap 链接 |
|------|------|------|------------------------------|
| **`record_session_id`** | `ovs-chat-3-afc29a80…` | `gateway_sessions` / `gateway_turns.session_id`；Admin 对话与 Tap `?session=` | **是** |
| **`worker_session_id`** | `ovs-3` | FC warm pool 租约、ttyd、agent WS path | 否 |
| **claw 交互续聊 jsonl** | `proj_N/sessions/{segment}/interactive-session.jsonl` | harness 多轮 transcript（per `record_session_id`） | 否 |

OVS Chat 扩展通过 `chatSessionId` query 把 **record** 传给网关；worker 始终是 `ovs-{projId}`（见 `extensions/claw-vscode/extension.js` `agentWsParts`）。

---

## 数据流（FC / Podman 交互式 agent）

```
OVS Chat
  → gateway agent/ws (record_session_id 已知)
  → ensure worker ovs-{projId}
  → fc/podman exec 写 gateway-record-session-id（Tap header）
  → exec: claw --resume /claw_ds/.claw/interactive/{segment}/interactive-session.jsonl -p "…"
  → stdout CDP → gateway WS → 扩展
  → turn 成功：读 NAS jsonl → import_turn_messages_to_db → cc_messages
```

`/terminal/*` 人工终端仍可为调试保留 ttyd；**agent prompt 不再经 ttyd**。

与 **`/v1/solve` / `gateway-solve-once`** 使用同一套 header 名；差别只是 solve 用 task 文件 + `docker exec -e CLAW_SESSION_ID`，交互式用 **文件**（warm worker 上 `claw` 进程已启动，不能靠改 env）。

---

## 代码落点

| 环节 | 位置 |
|------|------|
| jsonl 路径 + exec 脚本 | `rust/crates/gateway-solve-turn/src/ovs_interactive.rs` |
| 文件契约 + header 解析 | `rust/crates/gateway-solve-turn/src/worker_env.rs`（`GATEWAY_RECORD_SESSION_ID_*`、`gateway_llm_session_extra_headers`） |
| agent WS exec + cc_messages 写回 | `rust/crates/http-gateway-rs/src/session_agent_api.rs` |
| REPL 发 LLM 带 header | `rust/crates/rusty-claude-cli/src/main.rs` `AnthropicRuntimeClient::stream` |
| Admin turn `pool` / `worker` | `assign_ovs_turn_pool_worker` → `pool_id=fc-interactive`，`worker_name=fc:sbx_…` |
| tap 只认 header | `claude_tap/claw_session.py`（无 header 则不写 session trace） |

---

## Admin 展示

`gateway_turns` 在 OVS agent 开 turn 后应写入：

- `pool_id` = `fc-interactive`（常量 `FC_INTERACTIVE_POOL_ID`，与 solve 的 `fc-cloud` 区分）
- `worker_name` = 实际 warm worker，如 `fc:sbx_866ed706f88d`

此前只 `insert_turn` 未 `assign_turn_pool_worker`，Admin 会显示 `pool —`。

---

## 排障

1. Worker 内：`cat /claw_host_root/.claw/gateway-record-session-id` 应等于对话 `sessionId`。
2. tap 日志：有 header 时为 `[Turn N]`，无 header 为 `[proxy]`（不写 session 索引）。
3. `GET {observe}/api/sessions/traces?session={record_session_id}` 应有记录。
4. 不要用 claw 托管 `session-…` 或 `turnId` 查 Tap（与 Admin 对话模型不一致）。

---

## 相关文档

- **交互式多轮上下文（实施计划）：** [OVS-INTERACTIVE-CONTEXT-PLAN.md](./OVS-INTERACTIVE-CONTEXT-PLAN.md)
- Tap 写入 vs Live 读取：`docs/ovs-chat/FC-TAP-SINGLETON-DESIGN.md`
- solve header 约定：`docs/gateway-mcp-call-meta.md`、`docs/http-gateway-rs-api.md`
