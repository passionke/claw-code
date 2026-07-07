# OVS 交互式多轮上下文续聊 — 实施计划

Author: kejiqing

Status: **已实施（B1 agent/ws）** — 2026-06-29。ttyd 人工终端仍 legacy；solve 路径未改。交互 turn 后写回 `cc_messages`。

相关：

- 会话 ID / Tap：[OVS-INTERACTIVE-SESSION-ID.md](./OVS-INTERACTIVE-SESSION-ID.md)
- Transcript 工具成对落盘（内核）：[../runtime/TRANSCRIPT-TOOL-EXCHANGE.md](../runtime/TRANSCRIPT-TOOL-EXCHANGE.md)
- 路线图：[PLAN.md](./PLAN.md)
- Solve 持久化（**本计划不采用**）：[persistence-model.md](../persistence-model.md)

---

## 1. 问题与证据

### 1.1 现象

同一 OVS Chat `record_session_id`（例 `ovs-chat-3-be39fbcbcc1bb92a`）多轮对话时，AI 频繁「失忆」：第二轮不记得第一轮已对齐的围棋研究计划，偶发话题跑偏（围棋 → 国际象棋）。

### 1.2 证据链（2026-06-22，可复核）

| 证据 | 内容 |
|------|------|
| Admin / API | `GET /v1/sessions/ovs-chat-3-be39fbcbcc1bb92a/turns?proj_id=3` 有 11 轮完整 `userPrompt` + `reportBody` |
| Worker | 各轮 `worker_name` 均为 `fc:sbx_0c4a44a270bf`（同一交互 worker） |
| claw session 文件 | 每轮**不同** jsonl，例如 T1 `session-1782117031036-0.jsonl`、T2 `session-1782117160540-0.jsonl` |
| T2 jsonl | 仅 1 条 user（「计划文档写到本地」），assistant reasoning 写明 *I don't have context* |
| 路径 | `proj_3/home/.claw/sessions/…/`（`/claw_ds` 下 SessionStore 默认落点） |

结论：**网关 `gateway_turns` 有连续记录，claw harness 每轮却是新 session** — UI/Admin 看起来有历史，LLM 请求无历史。

### 1.3 根因（代码路径）

```
OVS 扩展 runAgentPrompt
  → 每轮 new WebSocket + { type: "spawn" }
  → gateway session_agent_api 连 ttyd 也发 spawn
  → ttyd 新连接 → 新 claw 进程 → LiveCli::new() → new_cli_session()
  → 每轮新 session-*.jsonl，harness 从未跨轮积累
```

`record_session_id` 当前只用于 `gateway_turns` 记账 + `gateway-record-session-id`（Tap header），**未绑定续聊 jsonl**。

`PLAN.md` 写「共用 `ovs-{projId}` terminal、共享 REPL 上下文」— **已与实现对齐修正**：worker 共享，**上下文按 record jsonl 隔离**。

### 1.4 跨 session 混入 prior messages（2026-06-29，Admin / fc-cloud 线索）

| 证据 | 内容 |
|------|------|
| 用户可见 prompt | `Determine the language…` 含 **Prior turn 1–3**（`4`、`2`、`今天营业额多少`），Current turn 为 `are you ok?` |
| turn 元数据 | `pool fc-cloud`、`worker fc:sbx_*`（solve 池，非 `fc-interactive`） |
| 代码 | `deploy/e2b/e2b_exec.py` 将 inline `session_jsonl` 写到 **固定** `/claw_host_root/.claw/gateway-solve-session.jsonl`；新 session 无 PG 消息时不覆盖，**残留上一 session 文件** |
| 语言推理 | `gateway-solve-turn/src/turn_language.rs` `collect_prior_user_prompts` 读该固定路径 |

**OVS B1 修复范围：** agent/ws 使用 per-record `interactive-session.jsonl`，不经过上述 solve 固定路径。

**Solve 残留问题（未在本变更修改）：** fc-cloud NAS 下应用 per-session 路径或每轮清空/覆盖 `gateway-solve-session.jsonl` — 需单独变更 `e2b_exec.py` / solve materialize。

---

## 2. 架构原则（分界轴）

### 2.1 按「是否交互」分两套，不按 e2b / Podman 分

| 模式 | API / 入口 | **LLM 上下文 SoT** | PG `cc_messages` |
|------|-------------|-------------------|------------------|
| **交互式** | `GET …/agent/ws`（OVS `@claw`）；`/terminal/*` 人工终端（见 §6） | NAS `interactive-session.jsonl` + harness `resume` | **仅写回**（轮次后，统一 Admin/tools）；**不**从 PG 灌入 |
| **Resolve / Solve** | `/v1/solve`、`/v1/solve_async`、`/v1/start` | PG 注水 → worker jsonl | **读 + 写**（`materialize_in` / `readback_out`，不动） |

交互式在 **FC** 与 **Podman** 上走**同一套 agent 业务逻辑**；仅「把 shell 送进 worker」的 **transport** 不同（fc exec vs podman/sandbox exec）。

Solve 仍可能走沙箱（`fc-cloud` 或 podman 槽）— 那是 resolve 线，**不经过** `session_agent_api`。

### 2.2 池与后端（基础设施仍分离）

| 用途 | `pool_id` / 模块 | 本计划 |
|------|------------------|--------|
| OVS 交互 warm worker | `fc-interactive` / `FcProjWarmPool` | **改** agent prompt 路径 |
| Solve（含 e2b 沙箱） | `fc-cloud` / `e2b_orchestrated_pool` | **不动** |
| OVS 壳 | `claw-ovs` singleton | **不动** |
| Observe Live | `claw-observe` singleton | **不动** |

---

## 3. 目标方案：B1（exec one-shot + resume jsonl）

### 3.1 每轮 prompt 流程

```
OVS Chat
  → gateway agent/ws（query: projId, chatSessionId → record_session_id）
  → ensure_terminal_active(worker_session_id = ovs-{projId})   // 交互 worker 存活
  → ensure_ovs_chat_record_session(record_session_id)          // gateway_sessions 注册
  → stage gateway-record-session-id（Tap header，现有）
  → ensure interactive-session.jsonl（不存在则写最小 session_meta）
  → exec 进 worker（FC 或 Podman，同一 shell 脚本）:
        claw --resume <JSONL> -p "<user>"   // CLAW_DISPLAY_MODE=web → stdout CDP
  → gateway 解析 stdout OSC/CDP → WS 推给扩展
  → finalize gateway_turns（轻记账，现有）
  → import_turn_messages_to_db（cc_messages 写回，统一 Admin 视图）
```

**不做（交互 prompt 前）：** `render_session_jsonl` / `materialize_in` / `ensure_jsonl_from_db`（solve 专用）。

**要做（交互 turn 后）：** 从 NAS `interactive-session.jsonl` 解析当轮 messages → [`import_turn_messages_to_db`](../../rust/crates/http-gateway-rs/src/persistence/transcript.rs) → `cc_messages`（与 solve `readback_out` 解析同构，**无** `workspace_tar`）。Admin、`GET …/turns/{id}/tools` 与 solve 共用 PG 视图。

```
OVS Chat → agent/ws
  → ensure worker + ensure jsonl + exec(claw --resume JSONL -p …)
  → harness persist → JSONL（SoT）
  → turn 成功：gateway 读 JSONL → import_turn_messages_to_db → cc_messages（只写）
  → finalize gateway_turns
```

### 3.2 续聊 jsonl 路径（固定 per record，与 `gateway_sessions` 同目录）

```
Guest:  /claw_ds/.claw/interactive/{segment}/interactive-session.jsonl
        (symlink → ../../../sessions/{segment}/interactive-session.jsonl)
NAS:    proj_{N}/sessions/{segment}/interactive-session.jsonl
```

**一个 Admin session = 一个 `proj_N/sessions/{segment}/` 文件夹**；`interactive-session.jsonl` 在该文件夹内增长。不再使用 `proj_N/home/.claw/ovs-chat/`（遗留路径仅在 exec 脚本里做一次迁移拷贝）。

**`segment` 是什么：** `sessions_directory_segment(record_session_id)`（`session_merge.rs`）— 把 `record_session_id` 变成安全单级目录名：

- 安全字符 → 原样（例 `ovs-chat-3-be39fbcbcc1bb92a`）
- 含 `/` 等 → UUID v5 确定性 32 hex

**为何放 `proj_N/sessions/{segment}/`：** 与 `gateway_sessions.session_home` 同一文件夹；Guest 经 `home/.claw/interactive/{segment}` symlink 写入。  
**为何不放 `/claw_host_root`：** 按 workerId 隔离，换槽即丢。

### 3.3 首轮（jsonl 不存在）

1. `mkdir -p` 父目录  
2. 若 jsonl 不存在：写入单行 `session_meta`（`workspace_root=/claw_ds`，0 条 message）  
3. `claw --resume <JSONL> -p "…"` — harness 追加 user/assistant 并 persist 到**同文件**

不在 PG 建 transcript；不靠 `claw -p` 默认 SessionStore 路径（避免 proliferation `session-1782117*-0.jsonl`）。

### 3.4 与三种 ID 的关系

| ID | 本方案职责 |
|----|------------|
| `record_session_id` | jsonl 路径主键 + `gateway_turns` + Tap |
| `worker_session_id` (`ovs-{projId}`) | 租约交互 worker；**不**作为续聊文件键 |
| claw jsonl 内 `session_id` 字段 | 文件内自洽即可（如 `ovs-interactive-{segment}`） |

更新 [OVS-INTERACTIVE-SESSION-ID.md](./OVS-INTERACTIVE-SESSION-ID.md) 数据流图：agent prompt **不再以 ttyd 为主路径**。

---

## 4. 并发、打断、错误

| 场景 | 策略 |
|------|------|
| 同 `record_session_id` 上一轮 exec 未完成又发 prompt | **409**「上一轮进行中」（默认；实现时可改排队，需单独立项） |
| 不同 `record_session_id`、同一 `ovs-{projId}` | 允许并行（不同 jsonl）；若 exec 资源争用再评估限流 |
| 用户点 VS Code Chat **停止** | 扩展 `CancellationToken` → `ws.close()`（已有）；gateway **kill 当轮 exec** + turn `cancelled`/`failed` |
| exec 非零退出 | turn `failed`；WS 推 error CDP / JSON |
| WS 中途断开 | 同停止：cancel exec |

前端不另做 mutex；依赖 Chat 进行中 UI + 停止 + 网关 409。

---

## 5. 明确不动（范围护栏）

- `/v1/solve`、`/v1/solve_async`、`/v1/start` 及 `gateway-solve-turn`、`session_db_sync.materialize_in` / `readback_out`
- PG 表 `cc_messages`：交互路径 **不写 solve 式 materialize**；turn 后 **只写回** transcript
- `fc-cloud` solve 执行逻辑
- OVS / Observe singleton 部署契约（除非验证脚本需改 prompt 路径）
- OVS Chat UI 本地历史存储
- Git 分支 → 独立 REPL（[PLAN.md](./PLAN.md) 后续项）

---

## 6. ttyd / Terminal API 边界

| 能力 | 本计划 |
|------|--------|
| `@claw` **agent/ws** | 改为 B1 exec；**不再**用 ttyd WS 喂 prompt |
| `/v1/sessions/…/terminal/*` 人工终端 | **本期不重构**；仍可为调试保留 ttyd。与 agent 解耦，避免混为「两套交互」 |

若日后人工终端也要续聊同一 jsonl，可复用同一 `interactive-session.jsonl` 路径（单独里程碑）。

---

## 7. 实施阶段

### Phase 0 — 文档与契约（本文件）

- [x] 问题证据、分界原则、B1 流程、路径、范围
- [ ] 评审通过后更新 `OVS-INTERACTIVE-SESSION-ID.md`、`PLAN.md` 轮次语义

### Phase 1 — 脚本契约（gateway-solve-turn）

- [x] `ovs_interactive_jsonl_guest` / `ovs_interactive_jsonl_host`
- [x] `build_ensure_ovs_interactive_session_script`
- [x] `build_ovs_interactive_prompt_script` + `claw gateway-interactive-once`
- [x] 单元测试

### Phase 2 — `session_agent_api.rs` 重构

- [x] e2b exec 流式 CDP（`exec_shell_script_streaming`）
- [x] per-`record_session_id` 409 锁
- [x] turn 后 `import_turn_messages_to_db`
- [x] agent 路径绕过 ttyd WS

### Phase 3 — 扩展与 E2E

- [x] `extension.js`：去掉 `spawn`
- [x] `verify-ovs-claw-e2e.sh` 更新
- [x] `verify-ovs-claw-context-isolation.sh` 路径隔离
- [x] 文档更新

### Phase 4 — 验收与回归

- [ ] e2b 环境：两轮同 `chatSessionId` 续聊（需在线 gateway + worker）
- [ ] 两 panel 不同 `record_session_id` 不串 prior messages
- [ ] Solve E2E 无 diff（未改 solve 路径）

---

## 8. 代码落点（预计）

| 组件 | 文件 |
|------|------|
| 脚本契约 | `rust/crates/gateway-solve-turn/src/ovs_interactive.rs`（OVS prompt；LLM 凭证由 exec env 注入，不 source `terminal-llm.env`） |
| Worker LLM 准备 | `rust/crates/http-gateway-rs/src/pool/e2b_worker_llm_material.rs`（`prepare_e2b_worker_llm_material` — solve / OVS / terminal 共用） |
| Agent WS | `rust/crates/http-gateway-rs/src/session_agent_api.rs` |
| e2b 流式 exec | `rust/crates/claw-e2b-sandbox-client/src/client.rs`（`exec_shell_script_streaming(..., env)`） |
| e2b exec helper | `deploy/e2b/e2b_exec.py`（`run_sh` + `env` inline export，与 `exec_solve` 共用 `_env_exports_sh`） |
| 扩展 | `extensions/claw-vscode/extension.js` |
| E2E | `deploy/stack/lib/verify-ovs-claw-e2e.sh` |
| 文档 | 本文件、`OVS-INTERACTIVE-SESSION-ID.md`、`PLAN.md` |

---

## 9. 验收标准（Definition of Done）

1. **上下文**：同一 `record_session_id` 连续 2+ 轮，第二轮 LLM 请求携带第一轮 transcript（jsonl message 条数增加；模型不再声称「第一条消息」）。
2. **文件**：每 `record_session_id` 只有一个 `interactive-session.jsonl`，不再每轮新增 `session-{ts}-0.jsonl` 于默认 SessionStore。
3. **分界**：交互 prompt 前无 `materialize_in`；turn 后 `cc_messages` 有当轮行；solve 续聊 E2E 无 diff。
4. **统一视图**：`GET …/turns/{turnId}/tools` 对 OVS 交互轮次可返回 tool 行（来自写回的 `cc_messages`）。
4. **双后端**：FC 与 Podman 各跑通 `verify-ovs-claw-e2e` 多轮变体。
5. **打断**：停止后 turn 终态合理，worker 无僵尸 claw 占满 CPU（手动或脚本抽查）。

---

## 10. 风险与缓解

| 风险 | 缓解 |
|------|------|
| exec 流式 CDP 与 ttyd 行为差异 | 复用同一 `extract_cdp_frames`；E2E 对比 UI 展示 |
| Podman 无流式 exec | 先用 `runtime_exec_with_live_streams`（solve 已有先例） |
| 并发双 prompt 竞态 jsonl | per-record 409 + 单文件 append 由 harness 串行化于单进程 exec |
| jsonl 过大 | 后续可接 harness compaction；本期不阻塞 |
| 多 Chat 面板同 proj 不同 record | segment 隔离；已设计 |

---

## 11. 待决（评审时确认）

| 项 | 默认 | 备注 |
|----|------|------|
| 同 session 连发第二条 | 409 | 若产品要排队，Phase 2 改 |
| Terminal ttyd 是否跟进同一 jsonl | 本期否 | Phase 6+ 可选 |
| jsonl bootstrap 由 shell 还是 Rust `Session::new` 写 | shell | KISS，少引 runtime 进 gateway |

---

## 12. 时间线（粗估）

| Phase | 工作量 |
|-------|--------|
| 0 文档 | 0.5d（本文件 + 评审修订） |
| 1 脚本契约 | 0.5d |
| 2 session_agent_api + 双 transport | 2–3d |
| 3 扩展 + E2E | 1d |
| 4 验收回归 | 1d |

**合计约 5–6d**（含 e2b + Podman 双环境验证）。

---

## 修订记录

| 日期 | 说明 |
|------|------|
| 2026-06-22 | 初稿：B1、交互 vs resolve 分界、证据链、阶段划分 |
