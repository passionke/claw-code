# OVS 交互式 `@claw`：会话 ID 与 Tap 对齐

Author: kejiqing

**原则：不新造 session 模型。** 复用 Admin 对话里已有的 `sessionId`（`record_session_id`）、solve 已有的 LLM header 契约、claude-tap 已有的 `claw-session-id` 索引。

**上下文 SoT：** `record_session_id` → `{clusterId}/proj_{N}/sessions/{segment}/.claw/interactive-session.jsonl`（guest `/claw_sessions/{segment}`）。`worker_session_id`（`ovs-{projId}`）仅表示项目级 worker 租约，**不**决定 prior messages。`home/`（`/claw_ds`）对 worker **只读**。

**多轮上下文续聊：** 已落地 B1（`session_agent_api` exec + `interactive-session.jsonl` resume）。详见 **[OVS-INTERACTIVE-CONTEXT-PLAN.md](./OVS-INTERACTIVE-CONTEXT-PLAN.md)**。

---

## 三种 ID（不要混用）

| 名称 | 示例 | 用途 | 是否给 tap / Admin Tap 链接 |
|------|------|------|------------------------------|
| **`record_session_id`** | `ovs-chat-3-afc29a80…` | `gateway_sessions` / `gateway_turns.session_id`；续聊 jsonl 主键；Tap `?session=` | **是** |
| **`worker_session_id`** | `ovs-3` | FC 项目 worker 租约、agent WS path、人工 terminal | 否 |
| **claw 交互续聊 jsonl** | `{clusterId}/proj_N/sessions/{segment}/.claw/interactive-session.jsonl` | harness 多轮 transcript（per `record_session_id`） | 否 |
| **claw 默认 SessionStore** | `session-1739…-0` | **legacy ttyd REPL** 每轮新建；agent 主路径已不用 | 否 |

OVS Chat 扩展通过 `chatSessionId` query 把 **record** 传给网关；worker 始终是 `ovs-{projId}`（见 `extensions/claw-vscode/extension.js` `agentWsParts`）。

---

## 数据流（FC 交互式 — agent/ws 主路径）

```
OVS Chat
  → gateway agent/ws（query: projId, chatSessionId → record_session_id）
  → ensure_terminal_active(worker_session_id = ovs-{projId})   // 项目 worker 存活
  → ensure_ovs_chat_record_session(record_session_id)          // gateway_sessions 注册
  → stage gateway-record-session-id（Tap header）
  → ensure interactive-session.jsonl（不存在则写 session_meta）
  → fc exec: claw gateway-interactive-once --session-jsonl <JSONL> --prompt-b64 …
  → gateway 解析 stdout CDP → WS 推给扩展
  → import_turn_messages_to_db（cc_messages 写回）
  → finalize gateway_turns
```

**不再**用 ttyd WS 喂 `@claw` prompt（`Spawn` 为 legacy no-op）。人工 `/terminal/*` 仍可用 ttyd，与 agent 解耦。

与 **`/v1/solve` / `gateway-solve-once`** 使用同一套 LLM header 名；交互式上下文来自 **per-record jsonl**，不从 PG 注水。

---

## 代码落点

| 环节 | 位置 |
|------|------|
| jsonl 路径 + exec 脚本 | `rust/crates/gateway-solve-turn/src/ovs_interactive.rs` |
| worker 内 one-shot | `rust/crates/rusty-claude-cli` → `claw gateway-interactive-once` |
| Agent WS 桥 | `rust/crates/http-gateway-rs/src/session_agent_api.rs` |
| Tap header 文件 | `rust/crates/gateway-solve-turn/src/worker_env.rs` |
| 扩展 WS | `extensions/claw-vscode/extension.js`（仅 `prompt`，无 `spawn`） |
| E2E | `deploy/stack/lib/verify-ovs-claw-e2e.sh`、`verify-ovs-claw-context-isolation.sh` |

---

## Admin 展示

`gateway_turns` 在 OVS agent 开 turn 后应写入：

- `pool_id` = `fc-interactive`（常量 `FC_INTERACTIVE_POOL_ID`，与 solve 的 `fc-cloud` 区分）
- `worker_name` = 实际项目 worker，如 `fc:sbx_866ed706f88d`

---

## 排障

1. 每 `record_session_id` 只有一个 jsonl：`ls {clusterId}/proj_N/sessions/{segment}/.claw/interactive-session.jsonl`（行数随轮次增长）。
2. Worker 内：`cat /claw_host_root/.claw/gateway-record-session-id` 应等于当轮对话 `sessionId`。
3. 不同 Chat 面板（不同 `record_session_id`）不得共享同一 jsonl 路径（见 `verify-ovs-claw-context-isolation.sh`）。
4. `GET {observe}/api/sessions/traces?session={record_session_id}` 应有记录。

---

## 相关文档

- **实施细节与验收：** [OVS-INTERACTIVE-CONTEXT-PLAN.md](./OVS-INTERACTIVE-CONTEXT-PLAN.md)
- NAS 布局：`docs/fc-nas-workspace.md`
- Tap 写入 vs Live 读取：`docs/ovs-chat/FC-TAP-SINGLETON-DESIGN.md`
