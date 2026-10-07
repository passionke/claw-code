# Gateway solve preflight（插件化管道）

Author: kejiqing

## 真相源

| 层 | 位置 |
| --- | --- |
| **DB** | `project_config.solve_preflight_json`（PostgreSQL） |
| **插件注册表** | `preflight_plugin` 表；Admin `GET/PUT /v1/preflight/plugins` |
| **物化** | `proj_<id>/home/.claw/solve-preflight.json`（`steps` 数组） |
| **运行时** | `gateway-solve-turn` `PreflightRunner`：按 `scope` 过滤 → builtin / 子进程 SPI → 声明式 `effects` |
| **Pool 挂载** | `home/.claw/solve-preflight.json` → worker ro |

与 `allowed_tools_json` 同模式：**DB 配置，gateway 物化，worker 只读**。

## 配置格式（`solvePreflightJson`）

推荐 **`steps`**（顺序 + **`on` 生命周期事件** + 可选 per-step `config` / `impl`）。事件表与上下文见 [`preflight-lifecycle-events.md`](preflight-lifecycle-events.md)。

```json
{
  "steps": [
    {
      "pluginId": "turn_language",
      "on": "turn.start",
      "impl": { "type": "builtin", "handler": "turn_language" }
    },
    {
      "pluginId": "sqlbot_mcp_start",
      "on": "session.start",
      "impl": { "type": "builtin", "handler": "sqlbot_mcp_start" }
    }
  ]
}
```

旧字段 `scope: every_turn|session_first_turn` 仍可读，映射为 `turn.start` / `session.start`。

兼容历史：

```json
{ "kinds": ["sqlbot_mcp_start"] }
{ "kind": "sqlbot_mcp_start" }
{ "kind": "none" }
```

| 输入 | 物化 / 运行时 |
| --- | --- |
| `kinds` 含 `sqlbot_mcp_start` | 自动前置 `turn_language`（`every_turn`）+ sqlbot（`session_first_turn`） |
| `kind: none` / `steps: []` | 物化空 `steps`；运行时**不执行**任何 preflight（含 `turn_language`） |
| 无 `solve-preflight.json` 文件 | 运行时默认每轮 `turn_language` |
| `language_pipeline_json`（deprecated） | 合并进 `turn_language` step 的 `config` |

## 事件 `on`（取代 scope）

| `on` | 何时执行 |
| --- | --- |
| `turn.start` | 每轮 solve（含续聊）；兼容旧 `every_turn` |
| `session.start` | 仅该 `sessionId` 第一次 solve，且尚未 satisfied；兼容旧 `session_first_turn` |
| `worker.init.start` / `worker.init.end` | 该项目 worker **create** 时（Landlock 前） |
| `worker.reuse.start` | acquire 已有 warm worker（Landlock 前） |
| `turn.end` / `session.end` / `worker.reuse.end` | 见生命周期指南 |

完整表与上下文：[`preflight-lifecycle-events.md`](preflight-lifecycle-events.md)。

## 何时执行（solve 管道：session/turn）

1. 加载 system prompt、初始化 MCP
2. **`push_user_text`**
3. **`PreflightRunner`**（仅 `session.start` + `turn.start`，按 `steps` 顺序；**不含** `worker.*`）
4. 合并 effects → system prompt / session 文件 / transcript
5. `ConversationRuntime::run_turn_after_user_message`

`worker.init.*` 在 gateway `create_and_persist_slot`；`worker.reuse.start` 在 warm acquire。语言推断：`turn_language` builtin（`turn.start`）。

## 内置插件

| `pluginId` | 作用 |
| --- | --- |
| `turn_language` | LLM 推断输出语言 → `lockLanguage` effect（`.claw/turn-language.json` + system prompt） |
| `sqlbot_mcp_start` | MCP `mcp_start` + schema 文件 + transcript 摘要（见下表） |

## 外部插件（子进程 SPI）

契约：[`docs/preflight-spi-v1.md`](preflight-spi-v1.md)、[`schemas/preflight-spi-v1.json`](../schemas/preflight-spi-v1.json)。

- stdin：SPI v1 请求 JSON
- stdout：SPI v1 响应 JSON（`effects[]`）
- subprocess **禁止** `injectToolExchange`（仅 builtin）

在 Admin **插件库**注册 `pluginId` + 默认 `command`，项目管道引用 `pluginId` 并可覆盖 `impl`。

## SQLBot 会话内 Markdown

| 文件 | MCP 来源 |
| --- | --- |
| `home/schema.md` | `mcp_datasource_tables` |
| `home/tables_and_rels.md` | `mcp_datasource_list` |
| `home/terminologies.md` | `mcp_datasource_terminologies` |
| `home/sql_examples.md` | `mcp_datasource_examples` |

## 扩展新 preflight

1. 实现子进程脚本（SPI v1）或后续 builtin 包装
2. `PUT /v1/preflight/plugins/{pluginId}` 注册
3. 项目 `solvePreflightJson.steps` 引用 `pluginId`、选 **`on`**、填 `config`
4. **无需**再改 `gateway-solve-turn` 的 `match kind`
5. 装软件等冷启动逻辑用 `worker.init.start`（见生命周期指南）

## 相关

- 生命周期事件（项目扩展点主入口）：[`preflight-lifecycle-events.md`](preflight-lifecycle-events.md)
- 编排（非 preflight）：`solve_orchestration_json` → [`multi-agent-analysis.md`](multi-agent-analysis.md)
- 项目配置总览：[`project-config-model.md`](project-config-model.md)
