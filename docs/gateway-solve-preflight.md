# Gateway solve preflight（按项目、代码执行）

Author: kejiqing

## 真相源

| 层 | 位置 |
| --- | --- |
| **DB** | `project_config.solve_preflight_json`（PostgreSQL，`CLAW_GATEWAY_DATABASE_URL`） |
| **物化** | `proj_<id>/home/.claw/solve-preflight.json`（`project_config_apply` 在设为生效 / 物化时写入） |
| **运行时** | Worker 只读物化文件；`gateway-solve-turn` 按 `kind` 执行注册逻辑 |
| **Pool 挂载** | `proj_*/home/.claw/solve-preflight.json` → worker 内 `/claw_host_root/home/.claw/solve-preflight.json:ro`（与 `schema.md` 同级，仅 session 目录 rw 时必需） |

与 `allowed_tools_json` → `.claw/project_allowed_tools.json` 同模式：**DB 配置，文件由 gateway/daemon 构造，pool 再 ro 挂进 worker**。

## Admin API

`GET` / `PUT /v1/project/config/{proj_id}` 字段 **`solvePreflightJson`**（camelCase）：

```json
{ "kinds": ["sqlbot_mcp_start"] }
```

兼容历史格式（仍可读）：

```json
{ "kind": "sqlbot_mcp_start" }
```

| `kind` | 行为 |
| --- | --- |
| `sqlbot_mcp_start` | 首轮在**用户问题之后**：`mcp_start`（`arguments` = 任务 `extraSession` 业务字段，见 [`gateway-mcp-call-meta.md`](gateway-mcp-call-meta.md) § SQLBot）→ list/tables/terminologies/examples → 会话内 `home/*.md`（见下表）；transcript 仅摘要 + 路径说明；任一步 MCP/解析失败则 `warn` 跳过 |

`PUT` 省略 `solvePreflightJson` 时保留库内已有值（同 `gitSyncJson`）。
`kinds` 为空数组时表示关闭 preflight；有多个时按顺序依次执行。

## 何时执行

- 仅该 `sessionId` **第一次** solve（无 `gateway-solve-session.jsonl`）
- 顺序：用户消息 → preflight → LLM
- **续聊**不跑

## 会话内 Markdown（Worker preflight 生成）

| 文件 | MCP 来源 |
| --- | --- |
| `home/schema.md` | `mcp_datasource_tables` |
| `home/tables_and_rels.md` | `mcp_datasource_list` 行内 `table_relation` 图 |
| `home/terminologies.md` | `mcp_datasource_terminologies` |
| `home/sql_examples.md` | `mcp_datasource_examples` |

- 写在**当前 session 工作区** `home/` 下；新 session 首轮重做；**不**写入 `proj_*` 宿主机目录持久化。
- **Gateway `proj_id` 与 SQLBot `datasource_id` 无关**；数据源范围由 **MCP token** 决定。约定：一业务一 token，list 只返回 **一条** datasource。
- 任一步 MCP 失败或 JSON→MD 解析失败：`tracing::warn`（target `claw_sqlbot_preflight`）后跳过，不阻断整轮 solve（`mcp_start` + list 取 id 仍失败则整段 preflight 失败）。
- System prompt 在 preflight 之后列出已生成的 `home/*.md` 路径。

## 扩展新 `kind`

1. 实现模块（如 `sqlbot_preflight.rs`）
2. 在 `gateway-solve-turn/src/project_preflight.rs` 的 `match cfg.kind` 注册
3. `validate_solve_preflight_json` 允许新 `kind` 字符串
4. 项目 DB 中 `solvePreflightJson.kind` 选用

## 相关：solve 编排（非 preflight）

首轮 preflight（schema 注入）与 **编排管道** 独立配置：

- DB：`project_config.solve_orchestration_json` → `home/.claw/solve-orchestration.json`
- `kind: multi_agent_analysis` 启用分阶段 Planner / 并行问数 / Writer + 并行 Narrator
- 详见 [`docs/multi-agent-analysis.md`](multi-agent-analysis.md)
