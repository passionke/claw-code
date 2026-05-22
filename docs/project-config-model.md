# `project_config`：按 `dsId` 的项目级 Agent 配置（PostgreSQL）

Author: kejiqing

## 目标

在 **`CLAW_GATEWAY_DATABASE_URL`（PostgreSQL）** 的 **`project_config`** 表中，按 **`ds_id`** 存储规则、MCP、**内联 Skills**、工具勾选与 **`CLAUDE.md`** 正文；网关按 `content_rev` 物化到 `ds_<id>/home`，solve 只认磁盘结果。

**不再**用 projects-git 同步 CLAUDE / Skills（`CLAW_PROJECTS_GIT_*` 可选且默认不启用）。

## 表设计（一行一个 `ds_id`）

| 列 | 类型 | 说明 |
| --- | --- | --- |
| `ds_id` | `BIGINT PRIMARY KEY` | 与 API `dsId` 一致。 |
| `content_rev` | `TEXT NOT NULL` | **当前生效**配置版本；轮询比对 `.claw/project_config_applied_rev`。 |
| `updated_at_ms` | `BIGINT NOT NULL` | 行更新时间（毫秒）。 |
| `rules_json` | `JSONB NOT NULL` | 规则清单，物化到 `home/.cursor/rules/`。 |
| `mcp_servers_json` | `JSONB NOT NULL` | 与 `mcpServers` 同形；**solve 仅读此列**。 |
| `skills_json` | `JSONB NOT NULL` | 内联 skills，见下。 |
| `skills_sources_json` | `JSONB NOT NULL` | **已废弃**（须为空数组）；保留列兼容旧库。 |
| `claude_md` | `TEXT` | 物化为 `home/CLAUDE.md`（非空才满足 `ds_project_tree_ready`）。 |
| `allowed_tools_json` | `JSONB NOT NULL` | 本项目工具勾选。 |
| `git_sync_json` | `JSONB NOT NULL` | 每项目 **单向** Git 推送配置（见下）；默认 `{}`。 |

### `git_sync_json`（每项目单向 Git）

**方向**：`home/` 下**用户工作文件** → 远程仓库（单向）；**不**从远程拉回 DB。

**不进入 Git**：推送时按当前 `project_config` 行计算排除列表（`project_config_apply::git_excluded_home_relpaths`）——即所有由 DB 物化到 `home/` 的路径：

- 非空 `claude_md` → `home/CLAUDE.md`
- 非空 `skills_json` → `home/skills/` 整树
- `rules_json` 每条 `relativePath`（如 `home/.cursor/rules/*.mdc`）

其余 `home/` 路径（用户 / Agent 工作成果）可进 Git。`sessions/`、`.claw/` 不在 `home/` 下，本就不推送。

**对象**（camelCase，存于 `git_sync_json`）：

| 字段 | 说明 |
| --- | --- |
| `enabled` | 是否启用；`false` 时不推送。 |
| `gitUrl` | GitHub/GitLab 风格 HTTPS 或 `git@` / `ssh://`；HTTPS **禁止** URL 内嵌用户名密码。 |
| `gitRef` | 分支名，默认 `main`。 |
| `gitToken` | HTTPS 用 PAT（**仅存 PG**）；API 读/列表 **不返回** 明文，仅 `gitTokenSet`。 |
| `authorName` / `authorEmail` | 可选；缺省用 `CLAW_PROJECTS_GIT_AUTHOR`。 |
| `lastPushAtMs` / `lastPushCommitId` / `lastPushError` | 网关推送后回写。 |

- 保存：`PUT /v1/project/config/{ds_id}` 的 `gitSyncJson`；PUT **省略** `gitSyncJson` 时保留库内已有配置；PAT 留空则保留已存 token。
- 推送：`POST /v1/projects/{ds_id}/git/push`（先物化再 push）。
- 物化成功后若 `enabled=true`，网关会 **尽力** 自动推送（失败仅 warn，不阻断 solve）。

### `skills_json` 约定

**`array`**，元素：

```json
{
  "skillName": "sql-safety",
  "skillContent": "# Skill\n..."
}
```

- 物化到 **`home/skills/<skillName>/SKILL.md`**（`project_config_apply::write_skills_json`）。
- 管理 API：`POST /v1/project/skills/{ds_id}` 合并写库；`GET /v1/skills/{ds_id}` 优先读库。

### `rules_json` / `mcp_servers_json` / `allowed_tools_json`

与先前设计一致；MCP 见 `build_settings`（**仅** `mcp_servers_json`，无 `.claw.json` / env 回退）。

**`allowed_tools_json`**：`string[]`，Admin Tools 页逐条勾选写入 DB。`[]` 表示 solve **不限制**工具；非空则仅允许所列名称（支持 `mcp__*` 前缀模式）。**不**使用 `CLAW_ALLOWED_TOOLS`；请求体 `allowedTools` 仍可在单次 solve 上进一步收窄，但不得超出项目基线。

### `skills_sources_json`

**禁止**在 `PUT` 中提交非空数组（返回 400）。历史 git 拉取逻辑已移除。

### 状态机（每 `ds_id`）

```text
STEADY:   draft_open=false，生效 E = 某一正式版（必在 project_config_revision）
EDITING: draft_open=true，仅 1 个临时版 content_rev=__draft__，生效 E 不变

各 Tab 保存     → 进入/更新 EDITING（从 E 的正式快照复制，仅一条临时版）
保存为正式版     → 临时版 → 新正式版 F，回到 STEADY（生效仍为 E）
设为生效(rev)   → E := rev（rev 必须是正式版），关闭临时版，物化
废弃(rev)       → 删除正式版 rev（rev ≠ 当前 E）
```

- **生效**只能从**正式版**选择；**临时版** `__draft__` 出现在版本列表首行（`isDraft: true`），不可设为生效、solve 不读临时版。

### `project_config_revision`（正式版，不可变）

| 列 | 说明 |
| --- | --- |
| `(ds_id, content_rev)` PK | 某一版配置快照（不含 `git_sync_json`，Git 仍只在 `project_config` 行上）。 |
| 其余列 | 与物化相关字段同 `project_config`（`rules_json`、`skills_json`、`claude_md` 等）。 |
| `note` | 可选备注（Admin 填写，便于查找；版本号不手填）。 |

- **不可变**：`INSERT … ON CONFLICT DO NOTHING`；已存在的 `content_rev` 不能覆盖。
- **临时版**：`project_config` 在编辑期使用 `content_rev = __draft__`、`draft_open = true`；`stable_content_rev` 为 solve 当前生效版。
- **写入临时版**：`PUT /v1/project/config/{ds_id}`、Rules/Tools/Git、`POST …/claude`、`POST …/skills`、`POST/DELETE …/mcp/inject*` 均先 `ensure_draft`（无临时版时从生效版复制）。
- **保存正式版**：`POST /v1/project/config/{ds_id}/versions/commit` body `{ note? }` — 服务端自动生成正式版号（本地 `YYYY-MM-DD_HH-mm-ss`，冲突加 `-2`）；临时版 → 正式版（不可变）；**不**改变当前生效版；关闭临时版。
- **设为生效**（单独动作）：`POST .../versions/{content_rev}/activate` — 在正式版间切换 solve 物化目标。
- **废弃**：`DELETE .../versions/{content_rev}` — 删除非生效的正式版（不可删当前生效版、`__draft__`）。
- **比对**：`GET .../versions/compare?from=&to=` 返回 `fromDocument` / `toDocument`（展开 JSON：`claudeMd`、`rulesJson`、`skillsJson`、`mcpServersJson`、`allowedToolsJson` 等）及 `changes` 顶层摘要；Admin 侧用 GitHub 风格 split diff 展示，并可按顶层块选择「保留 from / to」合并进 `__draft__`（`PUT /v1/project/config/{ds_id}`）。

`project_config` 另增列：`stable_content_rev`、`draft_open`。

## 运行时

1. **`POST /v1/projects`**：插入 `project_config`（默认 `claude_md` + 空 skills），物化。
2. **`PUT /v1/project/config/{ds_id}`**：更新临时版（不物化、不新增固化行）。
3. **`POST …/versions/commit`**：临时版 → 正式版（历史表）；生效版不变。
4. **`POST …/versions/{rev}/activate`**：切换生效版并物化。
5. **`DELETE …/versions/{rev}`**：废弃非生效正式版。
6. **`POST /v1/project/claude`** / skills / MCP：仅写临时版；物化仅在「设为生效」后发生。
7. **`POST /v1/init`**：要求已有 `project_config` 行，再物化稳定版（**不**拉 projects-git）。
8. **轮询**：`CLAW_PROJECT_CONFIG_POLL_INTERVAL_SECS` 按 `stable_content_rev` 物化（临时版不参与 solve）。

## L2：条目级历史（`project_entity_revision`）

与 L1（整包 `project_config_revision`）并行：**每次条目保存**追加一行，**无**条目级「设为生效」。

| 列 | 说明 |
| --- | --- |
| `domain` | `rule` \| `skill` \| `mcp` \| `claude` \| `tools` |
| `entity_key` | Rule 的 `ruleId`、Skill 名、MCP server 名；`claude` / `tools` 固定为 `_` |
| `entity_rev` | 不可变版本号（与 L1 同格式 `YYYY-MM-DD_HH-mm-ss`；Admin 列表优先显示 `createdAtMs` 格式化为本地时间） |
| `body` | 单条快照 JSON |

**写入**：`PUT /v1/project/config/{ds_id}` 在 rules/skills/mcp/claude/tools 切片变化时批量追加；`POST /v1/project/claude/{ds_id}`、`POST /v1/project/skills/{ds_id}` 在单条保存时再追加对应 domain。

**API**（`entity_key` 需 URL 编码）：

- `GET /v1/project/config/{ds_id}/entities/{domain}/{entity_key}/versions`
- `GET .../versions/compare?from=&to=`
- `POST .../restore` body `{ "entityRev": "..." }` — 写回 `__draft__` 聚合字段，不物化、不改变 L1 生效版

Admin：Rules / Skills / MCP 编辑页折叠面板「条目版本（L2）」。

## 实现状态

- DDL + `skills_json` 列（`session_db.rs` 迁移）。
- L2：`project_entity_revision` 表 + `project_entity_revision.rs`。
- 物化：`project_config_apply.rs`（rules、claude、**skills_json**）。
- 每项目 Git：`project_git_sync.rs` + `git_sync_json` 列；全局 `CLAW_PROJECTS_GIT_URL` mirror 已废弃（可选、默认不启用）。
