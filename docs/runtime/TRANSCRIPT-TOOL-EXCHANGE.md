# Transcript 消息环：tool_use / tool_result 成对落盘

Author: kejiqing

Status: **设计定稿 + 已实施** — 2026-07-06。内核层（`runtime` crate）；**与 OVS / gateway 传输无关**。

取代：[`TRANSCRIPT-TOOL-USE-SEAL.md`](./TRANSCRIPT-TOOL-USE-SEAL.md) 中「下轮 stream 前 seal 自愈」方案（**已废弃**）。

相关：

- 交互式 jsonl SoT：[`docs/ovs-chat/OVS-INTERACTIVE-CONTEXT-PLAN.md`](../ovs-chat/OVS-INTERACTIVE-CONTEXT-PLAN.md)
- 事故登记：[`docs/ovs-chat/FC-OVS-E2E-FAILURES.md`](../ovs-chat/FC-OVS-E2E-FAILURES.md)（F16）
- API 契约：[`rust/crates/api/src/providers/openai_compat.rs`](../../rust/crates/api/src/providers/openai_compat.rs)

---

## 1. 问题与根因

### 1.1 现象

多轮续聊中某轮 tool 中断后，后续轮次 API 400：

```text
An assistant message with 'tool_calls' must be followed by tool messages
responding to each 'tool_call_id'.
```

### 1.2 根因（证据见 F16 / `ovs-chat-2-dc88af29dc1d24b7`）

```text
LLM 返回 tool_use
  → push_message(assistant)     # jsonl 已 append（有头）
  → execute tool …
  → （取消 / 超时 / 杀进程）
  → tool_result 未写入           # 无尾 → 下轮 400
```

### 1.3 职责分界

| 层 | 职责 |
|----|------|
| **Transcript 消息环（内核）** | 磁盘 jsonl 永不出现「只有 tool_use、没有 tool_result」的半截 |
| **Turn 记账（gateway）** | `succeeded` / `failed` / `cancelled`；过程问题查日志 |
| **用户明确 rollback** | 产品能力，**不在本文** |

---

## 2. 设计原则（讨论结论，不得变形）

1. **不 rollback**：不删除已提交的 assistant `tool_use` 假装没发生（rollback 仅用户显式产品操作）。
2. **不在 OVS / gateway 写 seal**：交互入口 `gateway-interactive-once` → `run_turn`，无 transport 特例。
3. **不做「下轮补全上一轮」**：禁止在 `stream` 前对历史 jsonl 做 seal 自愈（`seal_unanswered` **不得**出现在 `run_turn` 热路径）。
4. **不做 in-flight 记账**（仅工具名+时间）：与兜底文案信息量相当，不单独引入 jsonl 协议。
5. **不要求每个 tool 实现 seal**：错误写入在 **dispatch 切面**（内核统一 `execute` 路径）。
6. **过程排障靠日志**；**消息环严格可靠**靠成对落盘。

---

## 3. 目标架构

### 3.1 成对落盘（主路径）

含 `tool_use` 的 assistant 消息 **在 tool 执行完成之前不进入** `session.messages` / jsonl。

```text
LLM stream → assistant（含 tool_use）留在内存
  → execute_pending_tool_uses（dispatch 切面，见 §3.2）
  → 收齐每条 tool_result
  → Session::push_tool_exchange(assistant, tool_results[])
       # 一次性 batch append：assistant + 全部 tool_result
  → 下一轮 stream 可见完整一对
```

**仅含文本**（无 `tool_use`）的 assistant：与现网相同，**立即** `push_message`。

**未成功 `push_tool_exchange` 时**（进程被杀、persist 失败前）：jsonl **无变更** → 下轮 retry 合理，等价于「本段 tool 回合未提交」。

### 3.2 Dispatch 切面 + 守卫（主路径错误写入）

`execute_pending_tool_uses`：

- **不向 session 增量 `push_message`**；只收集 `Vec<ConversationMessage>`（tool 行）。
- 每个 `tool_use_id` 在离开 dispatch 时必须有条 result（`Ok` / `Err` / hook 拒绝 → 正常 `tool_result` 文案）。
- **守卫**：若某 id 仍无 result（panic、背景线程未 join 等），补 `is_error: true` 的 `tool_result`，`detail` 用当时已知的 `execute` / hook 错误；**禁止**静默缺失。

正常 `execute` 返回 `Err("context deadline exceeded…")` → **在 dispatch 内**写入 error `tool_result`，再成对落盘；**不是**下轮 seal。

### 3.3 `max_iterations`

计数单位 = **`run_turn` 内层 loop 每轮 LLM `stream`**（+ 该轮一批 tool 执行），**不是**每个 tool 调用 +1。

默认 `usize::MAX`；显式 `with_max_iterations(n)` 时超限报错退出。

### 3.4 明确不做

| 方案 | 状态 |
|------|------|
| 下轮 `stream` 前 `seal_unanswered_tool_uses` | **禁止** |
| OVS gateway 读 jsonl 补写 | **禁止** |
| API 层 strip `tool_calls` | **禁止** |
| in-flight jsonl（仅名+时间） | **不做** |
| per-tool seal trait | **不做** |

### 3.5 `seal_unanswered_tool_uses` 保留范围

- 保留 API 供 **测试**、**手工修复 legacy jsonl**。
- **不得**在 `run_turn_inner` / 每次 `stream` 前 / `Drop` 中调用。
- **不得**在 `ConversationRuntime::drop` 中调用（与「不补上一轮」一致；成对落盘下析构时磁盘无半截）。

### 3.6 Legacy 损坏 jsonl

升级前已落盘的「有头无尾」session：**不会**自动下轮修复。需手工编辑 jsonl 或一次性运维脚本调用 `seal_unanswered_tool_uses`。

---

## 4. 实现清单（与代码一一对应）

| 项 | 模块 | 说明 |
|----|------|------|
| `Session::push_messages_batch` | `session.rs` | 多行 append；失败则回滚 `messages` |
| `Session::push_tool_exchange` | `session.rs` | 校验 tool_use ↔ tool_result 齐全后 batch 落盘 |
| `execute_pending_tool_uses` 无 session push | `conversation.rs` | 返回 `Vec<ConversationMessage>` |
| dispatch 守卫 | `conversation.rs` | 缺 result 补 error |
| `run_turn_inner` 分支 | `conversation.rs` | 无 tool → `push_message`；有 tool → exchange |
| 删除 stream 前 seal | `conversation.rs` | 移除 `seal_transcript` 热路径调用 |
| 删除 `Drop` seal | `conversation.rs` | 移除 `impl Drop for ConversationRuntime` |

---

## 5. 失败语义

| 场景 | jsonl | 下轮行为 |
|------|-------|----------|
| tool 正常结束 | +assistant +tool_results | 继续 loop |
| tool `Err`（含 timeout 原文） | +assistant +error tool_results | 继续 loop |
| `push_tool_exchange` 前进程被杀 | **不变** | 重试同一 user 上下文，模型可再次 tool |
| SIGKILL | **不变** | 同上；timeout 原文**仅**若 dispatch 已返回 `Err` 才会进 jsonl |
| 同一 turn 内多次 tool round | 每轮 iteration 各一对 | 受 `max_iterations` 限制 |

**不会**出现「同一份坏 transcript 无限 API 400」：磁盘无半截则历史合法。

---

## 6. 测试

| 测试 | 断言 |
|------|------|
| `push_tool_exchange_persists_pair_atomically` | batch 后 reload，assistant 与 tool_result 同在或同不在 |
| `run_turn_tool_error_still_pairs_exchange` | tool `Err` 仍成对落盘，无 dangling |
| `run_turn_tool_exchange_no_dangling_on_jsonl` | 含 tool 的 turn 后 `unanswered_tool_uses()` 为空 |
| `run_turn_without_tools_still_pushes_immediately` | 纯文本 assistant 行为不变 |
| 保留 `seal_unanswered_tool_uses_*` | 仅测 legacy API，非热路径 |

**集成测试**（mock API + OpenAI payload 配对断言，确定性覆盖 F16）：

| 测试 | 断言 |
|------|------|
| `f16_repro_legacy_dangling_transcript_fails_provider_invariant` | 旧式半截 jsonl 违反 provider 规则（对照组） |
| `mock_api_multi_turn_after_tool_error_yields_valid_openai_payload` | tool `Err` 后第二轮续聊 payload 合法 |
| `mock_api_multi_turn_after_successful_tool_yields_valid_openai_payload` | tool 成功后多轮 payload 合法 |
| `slow_tool_does_not_persist_half_assistant_before_exchange_completes` | dispatch 中途 jsonl 无半截 assistant |
| `mock_api_second_turn_request_sees_paired_history` | 第二轮 mock 请求历史已配对 |

```bash
cargo test -p runtime --lib
cargo test -p runtime --test transcript_tool_exchange_integration
```

---

## 7. 变更记录

| 日期 | 说明 |
|------|------|
| 2026-07-06 | 讨论定稿：成对落盘 + dispatch 守卫；废弃下轮 seal |
| 2026-07-06 | 按本文实施代码 |
