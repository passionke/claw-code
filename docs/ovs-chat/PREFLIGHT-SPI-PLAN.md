# Preflight 插件化与语言判定归位

Author: kejiqing  
Status: **draft** — 实现分支 `feat/preflight-spi`  
Related: [`../gateway-solve-preflight.md`](../gateway-solve-preflight.md)、[`../project-config-model.md`](../project-config-model.md)

---

## 现状

语言判定**不在** preflight 模块里，而是 `gateway-solve-turn` bootstrap 的硬编码 Step 0（`turn_language::infer_and_persist_turn_language_blocking` + `inject_language_into_system_prompt`）。

现有 preflight（`sqlbot_mcp_start`）是另一条路径：用户消息写入 transcript **之后**、LLM **之前**，且仅首轮（`project_preflight::run_first_turn_preflight`）。

| 能力 | 配置源 | Admin UI | 实现位置 | 作用域 |
|------|--------|----------|----------|--------|
| 语言推断 | `language_pipeline_json` → `language-pipeline.json` | 无独立页 | `turn_language.rs` 内联 LLM | **每轮** |
| SQLBot preflight | `solve_preflight_json` → `solve-preflight.json` | `PreflightPage.tsx` | `project_preflight.rs` `match kind` | **首轮** |

扩展新 preflight 仍要求改 Rust 并注册 match（见 `gateway-solve-preflight.md` §扩展新 kind）——与「外部 SPI、Admin 关联、无需改框架」目标冲突。

---

## 目标分层（自上而下）

- **外部实现层**（gateway 工程外）：Python/Shell 等，只依赖 SPI JSON 契约。
- **Admin 层**（`http-gateway-rs` + `gateway-admin`）：全局 `preflight_plugin` 注册表；项目 `solvePreflightJson.steps` 引用 `pluginId` + `scope` + `config` + 顺序。
- **框架层**（`gateway-solve-turn`）：读物化管道 → 按 `scope` 过滤 → spawn 子进程 / builtin → 校验 → 应用 **effects**。**禁止**再新增 `match kind` 业务分支。

---

## SPI 契约（首期：子进程 JSON）

**契约文件**：`schemas/preflight-spi-v1.json`、`docs/preflight-spi-v1.md`（仓库根，不在 `http-gateway-rs` 内）。

**stdin 请求**（框架 → 插件）：

- `spiVersion`: `"1"`
- `step`: `{ pluginId, scope, config }`
- `context`: `{ sessionId, turnId, workDir, isContinuation, userPrompt, priorUserPrompts[], extraSession, model }`
- `artifacts`: 只读路径列表

**stdout 响应**（插件 → 框架）：

- `status`: `ok | skip | error`
- `effects[]`：
  - `lockLanguage` → `.claw/turn-language.json` + system prompt
  - `writeSessionFile` → session `home/`
  - `appendSystemPromptSection`
  - `appendTranscriptSummary` → 单条 assistant `Text`
  - `injectToolExchange` → 成对 ToolUse+ToolResult（**首期仅 builtin sqlbot**）
- `metrics?`（可选）

**首期 builtin**：`turn_language`、`sqlbot_mcp_start` 包装现有 Rust，走同一 runner/effects 路径。

---

## 配置模型

扩展 `solve_preflight_json`（兼容旧 `kinds` 自动迁移为 `steps`）：

```json
{
  "steps": [
    {
      "pluginId": "turn_language",
      "scope": "every_turn",
      "impl": { "type": "builtin", "handler": "turn_language" },
      "config": {
        "languageInferencePriorTurns": 5,
        "languageInferencePriorMaxChars": 3000
      }
    },
    {
      "pluginId": "sqlbot_mcp_start",
      "scope": "session_first_turn",
      "impl": { "type": "builtin", "handler": "sqlbot_mcp_start" }
    }
  ]
}
```

`language_pipeline_json`：deprecated，迁移期 merge 进 `turn_language` step.config。

---

## 运行时顺序

1. `load_system_prompt`
2. 初始化 MCP / tool executor
3. `session.push_user_text(prompt)`
4. `PreflightRunner::run(steps)`（按 scope 过滤）
5. 合并 effects
6. `ConversationRuntime::run_turn_after_user_message`

---

## 对 Message / loop 的影响

**不改动** `runtime` 的 `ConversationMessage`、`ConversationRuntime` loop。Preflight 在 `ConversationRuntime::new` **之前**运行。

- 语言判定：仅 system_prompt + sidecar，不写 `Session.messages`
- sqlbot：loop 前注入 transcript（现网行为延续）
- subprocess **禁止**直接写 `ConversationMessage[]`；仅声明式 effect 白名单

---

## 单元测试策略（合并门槛）

| 层 | 包 | 内容 |
|----|-----|------|
| L1 | `preflight-spi` | JSON 契约、effect 枚举、subprocess 白名单 |
| L2 | `gateway-solve-turn` | scope 矩阵、effects applier、config 迁移、step 顺序 |
| L3 | `gateway-solve-turn` | tempdir 内嵌 sh/python echo JSON |
| L4 | `http-gateway-rs` | `validate_*` / `materialize_*` 纯函数 |

**不进门槛**：e2b 全链路、真实 LLM/MCP、Admin e2e。

```bash
cargo test -p preflight-spi -p gateway-solve-turn
cargo test -p http-gateway-rs solve_preflight
```

---

## 分期

1. **Phase 1**：SPI 文档 + runner + builtin 包装 + 配置合并 + Admin 项目管道 UI
2. **Phase 2**：全局插件库 UI + 示例外置 Python
3. **Phase 3**：删除 `language_pipeline_json` 列（Python 稳定后）
