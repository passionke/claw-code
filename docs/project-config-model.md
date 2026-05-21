# `project_config`：按 `dsId` 的项目级 Agent 配置（PostgreSQL）

Author: kejiqing

## 目标（与用户诉求对齐）

在网关已使用的 **`CLAW_GATEWAY_DATABASE_URL`（PostgreSQL）** 中增加 **`project_config`**，把「标准 Agent 能力」从**仅全局 env + projects git 镜像**扩展为**可按数据源（`ds_id`）版本化存储的配置**，并在运行时可还原为当前 Claw 网关已约定的**磁盘布局**。

本仓库现状（证据）：

- **会话 / 轮次 / 反馈**已在 PG：`GatewaySessionDb::migrate`（`rust/crates/http-gateway-rs/src/session_db.rs`）。
- **`ds_<dsId>` 工作区**上，solve 前要求存在**非空 `CLAUDE.md` 指令树**：`ds_project_tree_ready` / `ensure_ds_project_ready`（`rust/crates/http-gateway-rs/src/main.rs`）。
- **MCP** 由网关合并为 `session_home/.claw/settings.json` 的 `mcpServers`：`build_settings`（同上）。
- **Skills** 约定路径为 `ds_<dsId>/home/skills/<skillName>/SKILL.md`：`docs/http-gateway-rs-api.md`「Skills（按 ds 工作区）」。
- **projects git** 当前负责把远端 `ds_<id>/home` 同步到本地，并由 `POST /v1/init` 与可选轮询触发：`sync_ds_project_from_git_mirror`、`tick_projects_git_ds_home_poll`（`main.rs`）。

## 表设计（KISS：一行一个 `ds_id`）

DDL 由网关启动时 `GatewaySessionDb::migrate` 执行（与现有表一致），字段含义：

| 列 | 类型 | 说明 |
| --- | --- | --- |
| `ds_id` | `BIGINT PRIMARY KEY` | 与网关其余 API 的 `dsId` 一一对应（当前无单独 `project_id` 时，**项目 = 该 ds 工作区**）。 |
| `content_rev` | `TEXT NOT NULL` | 业务侧内容版本号或 git SHA；用于轮询时判断「是否有新配置」而不必逐字节比对。 |
| `updated_at_ms` | `BIGINT NOT NULL` | 行更新时间（毫秒）。 |
| `rules_json` | `JSONB NOT NULL` | 规则清单 + 正文，见下 JSON 约定。 |
| `mcp_servers_json` | `JSONB NOT NULL` | 与 `.claw/settings.json` 内 **`mcpServers` 对象**同形（网关 `build_settings` 合并逻辑不变，仅多一个来源）。 |
| `skills_sources_json` | `JSONB NOT NULL` | Skills 来源列表（含 git 坐标），见下 JSON 约定。 |
| `claude_md` | `TEXT` | 可选；若设置则物化为 `home/CLAUDE.md`（与现有 `ds_project_tree_ready` 判定一致）。 |
| `allowed_tools_json` | `JSONB NOT NULL` | 本项目勾选的 **Claw 工具名** 数组（见下）；空数组表示不额外限制，沿用网关 `CLAW_ALLOWED_TOOLS`。 |

### `rules_json` 约定

推荐 **`array`**，元素示例：

```json
{
  "ruleId": "sql-safety",
  "relativePath": ".cursor/rules/sql-safety.mdc",
  "content": "# Rule\n..."
}
```

- **`ruleId`**：稳定键，供幂等 upsert / 删除未再出现的规则文件。
- **`relativePath`**：相对于 **`ds_<dsId>/home/`** 的路径（UTF-8）；物化时创建父目录并写文件。
- **`content`**：完整规则正文。

说明：当前网关「是否 ready」仍以 **非空 `CLAUDE.md`** 为准（`ds_project_tree_ready`）；规则文件为附加能力，不替代 `CLAUDE.md` 门槛，除非后续产品明确修改该不变量。

### `allowed_tools_json` 约定

- **目录（只读）**：`GET /v1/project/tools/catalog` 返回网关当前注册的工具列表（`mvp_tool_specs` 内置工具 + `mcp__*` 模式说明），以及全局策略 `gatewayAllowedTools`（来自 `CLAW_ALLOWED_TOOLS`）。
- **存储**：`PUT /v1/project/config/{ds_id}` 请求体字段 **`allowedToolsJson`**，例如 `["read_file", "bash", "mcp__sqlbot__*"]`。
- **校验**：每个名称须在 catalog 内，且 ⊆ 全局 `CLAW_ALLOWED_TOOLS`（全局非空时）。
- **物化**：写入 `ds_<id>/.claw/project_allowed_tools.json`（`contentRev` + `allowedTools`），与 `project_config_applied_rev` 同轮刷新。
- **运行时**：`POST /v1/solve` / `solve_async` 在全局策略之上应用本项目勾选；请求体 `allowedTools` 只能再缩小范围，不能超出项目配置。`report_progress` 仍由网关自动加入允许列表。

### `mcp_servers_json` 约定

与运行时 **`mcpServers`** map 一致，例如：

```json
{
  "sqlbot": { "type": "http", "url": "https://example/mcp" }
}
```

合并顺序建议（实现时保持单一默认路径）：**全局网关 env 默认 MCP** → **`project_config.mcp_servers_json`** → **内存 `POST /v1/mcp/inject` 注入**（与现有 `build_settings` 插入顺序对齐，避免双轨行为）。

### `skills_sources_json` 约定

推荐 **`array`**，元素示例（把原「整仓 projects git」**内化**为可多条目的来源描述）：

```json
{
  "gitUrl": "https://git.example.com/org/skills-bundle.git",
  "gitRef": "main",
  "pathInRepo": "packs/analytics",
  "targetUnderHome": "skills",
  "tokenEnv": "CLAW_PROJECT_SKILLS_GIT_TOKEN"
}
```

- **`gitUrl` / `gitRef`**：拉取坐标（等价于原先全局 `CLAW_PROJECTS_GIT_*` 的**按项目拆分**）。
- **`pathInRepo`**：仓库内子树，同步到本地时的源前缀。
- **`targetUnderHome`**：默认 `skills`，表示落到 `home/skills/...`，与 `docs/http-gateway-rs-api.md` 路径一致。

### Git 凭据（不变量：仅 env）

1. **禁止**在 `project_config`（含 `skills_sources_json`）或 `PUT` 请求体中存放 token / PAT / 密码等明文或密文字段（如 `token`、`gitToken`、`accessToken`）。
2. **禁止**在 `gitUrl` 中嵌入 `user:token@`（HTTPS）；凭据只通过 **`tokenEnv`** 命名环境变量，由网关进程在 clone/pull 时 `std::env::var(tokenEnv)` 读取（与现有全局 `CLAW_PROJECTS_GIT_TOKEN` + `projects_git_effective_clone_url` 一致）。
3. **`tokenEnv` 必填**：当 `gitUrl` 为无 userinfo 的 `http://` / `https://` 时；`git@` / `ssh://` 可不填。
4. **禁止** BFF/请求体/PostgreSQL 作为 git token 的第二通道；MCP 的 `POST /v1/mcp/inject` **不**适用于 git 拉取凭据。

运维在 compose / K8s / 本机为网关容器配置上述 env；BFF 只写 `tokenEnv` 变量名，不写 secret 值。

## 配置的运行时使用（设计挂点）

### 1）某个 `ds` 初次使用：从 DB 物化并按规范初始化

挂点 **`prepare_gateway_session`** 内、在 **`ensure_ds_project_ready`** 之前或之内（`main.rs` 约 4436–4438 行附近）：若 PG 存在 `project_config` 行且 `content_rev` 与本地缓存标记不一致，则：

1. 在 **`ds_<dsId>` 锁**下物化：`rules_json` → `home/` 下文件；`claude_md` → `home/CLAUDE.md`；`skills_sources_json` → 克隆/稀疏检出到 `home/skills/`。
2. 写本地标记文件，例如 **`.claw/project_config_applied_rev`**（内容为 `content_rev`），避免每请求重复 git。
3. 再执行现有 **`ensure_workspace_initialized`**、**`build_settings`** 写 **`session_home/.claw/settings.json`**（已存在流程）。

这样仍满足 **`ds_project_tree_ready`** 对 **非空 CLAUDE** 的约束（需在配置或默认模板中保证 `claude_md` 或规则不替代 CLAUDE 门槛）。

### 2）定时加载最新内容（git 模态内化）

挂点可与现有 **`CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS`** 轮询合并为**单一调度器**（KISS）：

- **若行存在 `project_config`**：按行的 git 源与 `content_rev` 拉取更新 → 物化 → 更新 `.claw/project_config_applied_rev`；必要时 bump 会话侧 **`mcp_discovery_cache` 清理**（与 `purge_mcp_discovery` 同类逻辑）。
- **若行不存在**：保留现有 **全局 projects git 镜像**行为（向后兼容），避免第二套并行「仅 env」与「仅 DB」长期分叉。

## 与文档边界表的关系

更新 **`docs/boundaries-claw-stack.md`**：HTTP gateway 拥有 **`project_config` 表**及物化到 `ds_*` 工作区的职责；Claw 运行时仍只读本地 `.claw/settings.json` 与 `ConfigLoader`（`gateway-solve-turn` 不改变该边界）。

## 实现状态

- **已完成**：DDL + `get` / `upsert` / `list_project_config_ds_ids`（`session_db.rs`）。
- **已完成**：物化模块 `project_config_apply.rs`（rules、`claude_md`、skills git → `home/`，`.claw/project_config_applied_rev`）。
- **已完成**：`POST /v1/init`、`ensure_ds_project_ready`（solve 前）、`PUT /v1/project/config`（写库后立即物化）、轮询 `tick_projects_git_ds_home_poll`（有 `project_config` 行时按 `content_rev` 刷新；无行则沿用全局 projects git 镜像）。
- **已完成**：`build_settings` 合并 `mcp_servers_json`（顺序：全局 env → **project_config** → `POST /v1/mcp/inject`）。
- **已完成**：`allowed_tools_json` + `GET /v1/project/tools/catalog` + solve 时按项目勾选合并工具策略（`project_tools.rs`）。
- Git 凭据：**仅** `tokenEnv` → `std::env::var`（`project_config_apply::git_effective_clone_url`）。
