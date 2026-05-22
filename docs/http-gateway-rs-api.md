# HTTP Gateway RS API

本文档是 Rust 网关对外 HTTP 接口清单，保持与运行代码一致。

Base URL 示例：`http://127.0.0.1:18088`

## Health

- `GET /healthz`
  - 用途：健康检查与关键运行配置回显
  - 回显字段含 `sessionDatabaseBackend`（`postgresql`）、`gatewayDatabaseUrl`（脱敏连接串）。会话/轮次/反馈表在 **PostgreSQL**，由环境变量 **`CLAW_GATEWAY_DATABASE_URL`** 指定（compose 默认连栈内 `postgres` 服务；生产可指向独立 PG 集群）。
  - **`deployImageRef`** / **`deployImageTag`**：由 **`./deploy/stack/gateway.sh up`** 带入的 **`GATEWAY_IMAGE`**（根 `.env` 或 `up --release release-vX.Y.Z` 写入的 `deploy/stack/.claw-image-release.env`）经 compose 注入 **`CLAW_GATEWAY_IMAGE_REF`**，无需单独配置。Admin 顶栏展示 `deployImageTag`：`…:local` → `local`；`…:release-v1.2.3` → `release-v1.2.3`。
  - **`claudeTap`**（claude-tap 代理 / Live，与 MCP 的 `defaultHttpMcp*` 无关）：
    - `internalProxyBaseUrl`：worker 侧 LLM 走 tap 的地址（通常 `INTERNAL_CLAUDE_TAP_HOST`，如 `http://host.docker.internal:8080`）
    - `publicProxyBaseUrl` / `publicLiveBaseUrl`：浏览器从**当前访问网关的 Host** 推导（同主机、换端口 `CLAUDE_TAP_PORT` / `CLAUDE_TAP_LIVE_PORT`）；若设 **`CLAW_GATEWAY_PUBLIC_BASE_URL`**（如 `http://192.168.9.252:18088`、`http://127.0.0.1:18088`、`http://localhost:18088`）则以其 hostname 为准（`127.0.0.1` 与 `localhost` **原样保留**，不互相替换）
    - 须为带 `http://` 或 `https://` 的**绝对 URL**；不支持无 scheme 的「相对」写法（如仅 `:18088` 或 `/healthz`）
    - `liveSessionQueryParam`：固定 `session`
    - `liveSessionUrlTemplate`：例 `http://192.168.9.252:3000/?session={sessionId}`（将 `{sessionId}` 换成网关 `sessionId`）
    - 端口默认：`tapProxyPort=8080`（`CLAUDE_TAP_HOST_PORT` 或 `CLAUDE_TAP_PORT`）、`tapLivePort=3000`（`CLAUDE_TAP_LIVE_PORT`）

## Solve

- `POST /v1/solve`
  - 用途：同步执行一次 solve
  - 会话 ID 约定：
    - **有效 `sessionId`**：请求体可选字段 `sessionId`（非空）优先；否则使用请求头 `claw-session-id`；再否则 `x-request-id`；皆无则网关生成 UUID。响应头 `claw-session-id` / `x-request-id` 与响应体 `sessionId` / `requestId` 与有效值一致。
    - 若请求头已带 `claw-session-id` 或 `x-request-id`，且请求体 **`sessionId` 与头不一致**，返回 `400`（`sessionId conflicts with claw-session-id or x-request-id header`）。
  - **续聊与路径**：网关将 `(sessionId, dsId)` 与工作区目录的映射写入 PostgreSQL（`gateway_sessions`）。请求体传入 **`sessionId` 且非空** 表示显式续聊：若库中无该 `(sessionId, dsId)` 行，返回 `400`（`unknown sessionId (no session history for this dsId)`）。未传 `sessionId` 但头里携带的会话在库中已有记录时，会 **复用同一工作目录** 与磁盘上的对话 jsonl，实现多轮连续。
  - 请求体字段：
    - `dsId`：必填，数据源 ID，整数且需 `>= 1`
    - `userPrompt`：必填，用户自然语言输入，非空字符串
    - `sessionId`：可选，非空时表示按该 id 续聊（须已在库中有历史）；与头冲突规则见上
    - `model`：可选，指定使用的模型标识，缺省走网关默认模型
    - `timeoutSeconds`：可选，整体超时时间（秒）
    - `extraSession`：可选，**JSON 对象**，业务会话级上下文（例如用户、租户、workspace 等标识）
      - 若存在但不是 object，将返回 `400`（`extraSession must be a JSON object when present`）
      - 序列化后大小上限约为 `8KB`，超出将返回 `400`（`extraSession is too large (max 8KB)`）
  - 追踪约定：
    - 网关会为本次调用确定 `sessionId`（等于 `claw-session-id`）
    - 响应体主字段使用 `sessionId`，并保留 `requestId` 兼容字段（同值）
    - 响应体含 `sessionHomeRel`：相对 `CLAW_WORK_ROOT` 的会话工作目录（与 PG 表 `gateway_sessions.session_home` 一致），与 `workDir`（绝对路径）成对出现；含 **`turnId`**（当次轮次，`T_<32位小写hex>`）。**新建会话**时目录名为 `ds_{dsId}/sessions/<segment>`：在 `sessionId` 可作为安全单段路径名时 `<segment>` **与 `sessionId` 相同**（网关生成的 32 位十六进制 id 即落在该目录下）；若 `sessionId` 含路径分隔符等不安全字符，则 `<segment>` 为对该 id 做 UUID v5 派生的 32 位十六进制名（与 id 一一对应、可复现）。续聊仍按库中已有 `session_home` 打开原目录。
    - 在访问上游模型时透传 HTTP 头：
      - `clawcode-session-id: <sessionId>`
      - `claw-session-id: <sessionId>`
    - 在访问下游 MCP 服务（包括 SQLBot）时，会通过 MCP 协议 `tools/call._meta.extra_session` 向工具端暴露 `extraSession`（如存在），用于会话级业务上下文消费。
  - 对话状态：同一会话目录下使用 `.claw/gateway-solve-session.jsonl` 持久化消息；若文件损坏导致无法加载，返回 `500`（不会静默丢弃历史）。
  - **SQLBot 预注入（可选）**：环境变量 **`CLAW_GATEWAY_SQLBOT_PREFLIGHT`**（根 `.env`，经 worker 白名单传入 solve 进程）。**未设置时默认开启**：在首轮 LLM 之前自动执行 `mcp_start`、`mcp_datasource_list`、`mcp_datasource_tables`，并把 `tool_use` / `tool_result` 写入会话 jsonl。设为 **`0`** / **`false`** / **`off`** / **`no`** 可关闭，由模型按系统提示自行调用 MCP，避免与用户 prompt / CLAUDE 指令冲突。

- `POST /v1/start`
  - 用途：异步提交 solve（与 `solve_async` 相同入队逻辑），**立即**返回 `sessionId` / `requestId`（二者同值，且等于 `taskId`）；供 BFF「agent/start」等会话引导场景使用，**不要**再同步调用 `/v1/solve` 阻塞等待。
  - 请求体：与 `/v1/solve_async` 相同（`SolveRequest`）。
  - 响应体：仅 `sessionId`、`requestId`；响应头 `claw-session-id` / `x-request-id` 与体字段一致。
  - 错误语义：与 `solve_async` 相同（未知续聊 `sessionId` → `400`；同会话已有 `queued`/`running` 任务 → `409`）。

- `POST /v1/solve_async`
  - 用途：异步提交 solve 任务，返回 `taskId`
  - ID 约定：`taskId` 与 `sessionId` 为同一个值（同一逻辑会话 ID），用于统一追踪与轮询。
  - 响应兼容：同时返回 `requestId`（值等于 `sessionId`）、**`turnId`**（当次轮次，`T_<32位小写hex>`）；响应头 `claw-session-id` / `x-request-id` 与有效 `sessionId` 一致（与 `/v1/solve` 相同合并规则）。
  - **显式续聊**：请求体带非空 `sessionId` 时，若库中无该 `(sessionId, dsId)`，在入队前返回 `400`（文案同同步接口）。
  - **串行**：同一 `sessionId` 已存在状态为 `queued` 或 `running` 的异步任务时，再次 `POST /v1/solve_async` 返回 **`409 Conflict`**（`session has active async task`），需等待完成或取消后再提交。
  - 追踪约定：异步调用同样透传 `clawcode-session-id` 与 `claw-session-id`（值均为该次任务的网关层会话 ID）
  - **Live 报告**：pool worker 将模型 `TextDelta` 经 `POST /v1/internal/turns/{turnId}/assistant-stream` 写入 PostgreSQL `gateway_turn_live_chunks`（需 `CLAW_GATEWAY_INTERNAL_BASE_URL` + `CLAW_GATEWAY_INTERNAL_TOKEN`；`./deploy/stack/lib/sync-worker-openai-env.sh` 会在根 `.env` 缺省时写入 `http://host.containers.internal:<GATEWAY_HOST_PORT>` 与 dev token）
  - 终态清理：默认 **不** 在 `succeeded` 后删 live chunks（便于排查）；设 `CLAW_GATEWAY_DELETE_LIVE_CHUNKS_ON_SUCCESS=1` 恢复旧行为。failed/cancelled 仍用 `scripts/purge-gateway-turn-live-chunks.sh` 运维清理。

- `POST /v1/internal/turns/{turnId}/assistant-stream`
  - 用途：worker 上行 NDJSON `{"chunk":"…"}`；网关批量 `INSERT` + `NOTIFY claw_turn_live`
  - 鉴权：请求头 `x-claw-gateway-internal-token` 或 `Authorization: Bearer <CLAW_GATEWAY_INTERNAL_TOKEN>`
  - ingest 结束后同 turn 再 POST 返回 **409**

- `GET /v1/tasks/{task_id}`
  - 用途：查询异步任务状态与结果
  - 响应含 **`turnId`**（与本次 async 入队时返回的值一致）
  - **网关重启后**：若进程内已无该 `taskId` 的内存任务，网关会按 `session_id = task_id` 从 PostgreSQL 读取 **`gateway_turns` 最新一行** 重建 `TaskRecord`（含 `status`、`result`/`error`、`turnId` 等），以便 BFF 继续轮询；进度句与 `hasReport` 仍尽量从会话目录的 progress / spill 文件恢复。
  - 响应除 `status` 外含 **`currentTaskDesc`**（用户可见进度一句，camelCase JSON）：主要来自 agent 调用的内部工具 `report_progress` 写入会话目录 `.claw/task-progress.json`；`queued` 时网关可返回「排队中（x 个等待，y 个执行中）」；`running` 且无上报时兜底「处理中」或「工具调用中」（不暴露具体工具名）。**不**从 `gateway-solve-session.jsonl` 最后一条 assistant 推导。
  - 另含 `dsId`、`progressUpdatedAtMs`、**`hasReport`**（bool）、**`reportTime`**（ms，可选）：**`hasReport` = PG 里已有报告正文（至少一行 live chunk）**，用于告诉前端「可以开报告 SSE」；**不是**「任务在 running」。`running` 且无 live 行 → `hasReport: false`；`succeeded` 恒 `true`。`reportTime`：`hasReport` 为真时优先 `gateway_turn_live_chunks` 该 `turnId` 的 `MIN(created_at_ms)`；仅 `succeeded` 且无 live 行时用 `finishedAtMs`。完整四条契约见 `docs/persistence-model.md` § Live report contract (locked)。

- `GET /v1/sessions/{session_id}/turns/{turn_id}/tools?ds_id=<int>`
  - 用途：查看该 **gateway `turnId`** 对应用户轮次在 `.claw/gateway-solve-session.jsonl` 中的全部 **tool 调用**（`tool_use` 入参 + `tool_result` 返回）
  - 响应：`tools[]` 含 `toolUseId`、`toolName`、`input`（JSON）、`output`、`isError`；超大字段按 `CLAW_TURN_TOOLS_MAX_FIELD_CHARS`（默认 120000）截断并标 `*Truncated`
  - 未知 session / turn：404

- `GET /v1/sessions/{session_id}/execution?ds_id=<int>`
  - 用途：按 `(sessionId, dsId)` 查看当前进度快照、`progressHistory`（`.claw/progress-events.ndjson` 尾部）、网关队列统计、脱敏 trace 尾（`include_trace=true` 时含更多字段）
  - `progressHistory` 每条 `message` 默认最多 **80** 个 Unicode 字符，超出截断并追加 `...`；环境变量 **`CLAW_PROGRESS_MESSAGE_MAX_CHARS`**（正整数）可覆盖。事件 `kind`：`report_progress`（模型 `report_progress` 工具上报）、`mcp_tool_started`（NL 查询类 MCP 发起时一条；不追加 `mcp_tool_completed` / `mcp_tool_failed`，避免重复或失败文案刷屏）
  - 无该会话行：404

- `POST /v1/tasks/{task_id}/cancel`
  - 用途：按 `taskId`（与异步会话 `sessionId` 同值）取消仍处于 `queued` 或 `running` 的 solve 异步任务
  - 对 `queued` / `running`：成功时状态变为 `cancelled`，`finishedAtMs` 写入，`error` 示例：`{"detail":"cancelled by client","outcome":"cancelled","cancelApplied":true}`（内存路径下还会 `abort` worker、并在有租约时 `force_kill_slot`）
  - 对已是终态 `succeeded` / `failed` / `cancelled`：幂等返回 **`200`**（不改动 `status` / `result`），`error` 说明未再取消，例如：`{"detail":"task already succeeded; cancel had no effect","outcome":"idempotent","cancelApplied":false,"statusAtCancel":"succeeded","previousError":...}`（可安全重试、连点取消）；**网关重启后无内存任务时**亦按 PostgreSQL **最新一轮** `gateway_turns` 状态做同样判断（终态只幂等，非终态则只写 DB 为 `cancelled`）
  - 若 `task_id` 在库中无任何 `gateway_turns` 行：返回 `404`
  - 说明：网关**每次启动**会把仍为 `queued`/`running` 的轮次统一标为 **`failed`**（视为重启中断，见 `docs/persistence-model.md`）。取消通过中止网关侧异步 worker 实现；若当前正阻塞在长时间同步推理 `run_turn` 中，可能要等该段同步逻辑返回后 worker 才会结束，但**不会**再用成功结果覆盖已为 `cancelled` 的状态。

- `POST /v1/agent/feedback`
  - 用途：对会话内**某一轮** Agent 回复点赞/点踩（须带 `turnId`）
  - 请求体：`dsId`（≥1）、`sessionId`、`turnId`（格式 `T_<32位小写hex>`）、`feedback`（`good` | `bad`）
  - 校验：`(sessionId, dsId)` 须在 `gateway_sessions`；`turnId` 须属于该会话（`gateway_turns`）
  - 成功：`sessionId`、`dsId`、`turnId`、`feedback`、`updatedAtMs`
  - 同一轮再次提交为覆盖更新

- `GET /v1/agent/feedback?sessionId=<id>&dsId=<int>`
  - 用途：查询该会话下**已有反馈**的轮次（未操作的 `turnId` 不出现）
  - Query：`dsId` 或 `ds_id` 二选一
  - 成功：`{ "sessionId", "dsId", "items": { "<turnId>": "good"|"bad", ... } }`
  - 未知会话：**404**

- `turnId` 签发：每次 `POST /v1/solve` / `POST /v1/solve_async` 入队或同步受理时由网关生成；响应体与 `GET /v1/tasks/{task_id}` 含 `turnId`。`POST /v1/start` 不签发。

- `GET /v1/biz_advice_report?sessionId=<id>&turnId=<T_…>&dsId=<int>`
  - 用途：有 live chunk 或 turn 仍 `running`/`queued` 时，`stream=true` 走 PostgreSQL tail（`LISTEN/NOTIFY` + `SELECT seq ASC`）+ 终态 `gateway_turns.report_message`（无 LLM 润色）；否则与 `biz_advice_report_bak` 相同——终态 LLM 润色
  - **前端约定（locked）：** 仅当 `GET /v1/tasks` 返回 **`hasReport: true`** 后建立 SSE；按 `seq` 顺序消费 `biz.report.delta` 的 `text` 拼接展示，**禁止**用 `running` 代替 `hasReport` 开门，**禁止**客户端 `frameSeq` / `afterSeq` / `start.snapshotText`。见 `docs/persistence-model.md` § Live report contract (locked)。
  - 查询参数：
    - `sessionId`、`turnId`（`T_<32 hex>`）、`dsId`（≥1）必填
    - `stream`：默认 `true`；为 `true` 时 `text/event-stream`（`biz.report.start` / `biz.report.delta` / `biz.report.done`）
  - 结束条件（SSE）：`gateway_turns.status=succeeded` 且 `report_message` 非空 → `biz.report.done`（正文在 `reportJson.message`）；`failed`/`cancelled` 发 error
  - 非流式（`stream=false`）：仅 turn 终态可读，返回 JSON（`reportText` / `reportJson.message`）
  - **正文来源顺序**（`resolve_formal_report_text`）：`gateway_turns.report_message` / `output_json`；详见 `docs/persistence-model.md`。

- `GET /v1/biz_advice_report_bak?task_id=<taskId>`
  - 用途：**旧版**——基于异步任务 `outputJson.message` 再经 `GPOS_BOSS_REPORT_WRITER` skill **LLM 润色**
  - 查询参数：`task_id` 必填；`stream` 可选（默认 `false`）
  - 前置条件：任务 `succeeded` 且 `clawExitCode=0`

## Skills（按 ds 工作区）

技能文件与 `POST /v1/project/skills` 一致：`<CLAW_WORK_ROOT>/ds_<dsId>/home/skills/<skill_name>/SKILL.md`。`skill_name` 为目录名，不含 `/`、`\` 或 `..`。

- `GET /v1/skills/{ds_id}`
  - 用途：列出该 `dsId` 下所有已存在的技能（仅包含存在 `SKILL.md` 的子目录）
  - 成功响应 JSON：`{ "ds_id": <int>, "skills": [ { "skill_name": "<str>", "skill_content": "<str>" }, ... ] }`（按 `skill_name` 排序）

- `GET /v1/skills/{ds_id}/{skill_name}`
  - 用途：读取单个技能的完整 `SKILL.md` 文本
  - 成功响应 JSON：`{ "ds_id": <int>, "skill_name": "<str>", "skill_content": "<str>" }`
  - 若文件不存在：返回 `404`

## MCP

Solve 使用的 `mcpServers` **只来自** PostgreSQL `project_config.mcp_servers_json`；无行则为空（不回退 `.claw.json`）。

- `POST /v1/mcp/inject`
  - 用途：写入/合并 `project_config.mcp_servers_json`（`replace: true` 全量替换该字段；否则按名合并）
  - 自动生成 `contentRev`（`mcp-<ms>`）

- `GET /v1/mcp/injected/{ds_id}`
  - 用途：按 DB 配置写 `ds_<id>/.claw/settings.json` 并探针；`injectedServerNames` 为当前 DB 中的 server 名（不返回 Bearer 等敏感字段）

- `DELETE /v1/mcp/injected/{ds_id}`
  - 用途：清空或按 `server_names` 删除 `project_config` 中的 MCP 条目

## Project config (PostgreSQL)

按 `dsId` 在 **`project_config`** 表存储规则、MCP、**内联 `skillsJson`**、工具勾选与 **`claudeMd`**；约定见 **`docs/project-config-model.md`**。写库后物化到 `ds_<dsId>/home`；`POST /v1/init` / solve 前 / 轮询在 `contentRev` 变化时刷新。**须先有 `project_config` 行**（`POST /v1/projects` 或 `PUT`）。

- `GET /v1/project/tools/catalog`
  - 用途：列出网关当前注册的可选工具（内置 + `mcp__*` 模式），供 BFF 勾选 UI
  - 响应：`{ "tools": [ { "name", "description", "source" }, ... ] }`（勾选结果仅存 `allowedToolsJson`，不读 `CLAW_ALLOWED_TOOLS`）

- `GET /v1/project/config/{ds_id}`
  - 用途：读取该 `dsId` 的配置行
  - 无行：**404**

- `PUT /v1/project/config/{ds_id}`
  - 用途：写入**临时版**（`__draft__`）；不新增固化行、不切换生效、不物化（须已有 `project_config` 行）
  - 请求体（camelCase）：`rulesJson`、`mcpServersJson`、`skillsJson`、`allowedToolsJson`、`claudeMd`、`gitSyncJson`（省略则保留 Git 配置）；`skillsSourcesJson` 须为 `[]`
  - 响应：`{ "draftOpen": true, "stableContentRev", "activeConfig": { ... } }`（`activeConfig` 为临时版内容）

- `POST /v1/project/config/{ds_id}/versions/commit`
  - 用途：将临时版**保存为正式版**（不可变）；**不**切换生效版、不物化
  - 请求体：`{ "note": "可选备注" }`（版本号由服务端按本地时间生成 `YYYY-MM-DD_HH-mm-ss`，冲突时 `-2`、`-3`…；Admin 下拉以 `createdAtMs` 显示为可读时间）
  - 响应：`{ "savedContentRev", "activated": false, "stableContentRev", "materialized": false, "activeConfig" }`

- `DELETE /v1/project/config/{ds_id}/versions/{content_rev}`
  - 用途：**废弃**某正式版（非当前生效版）
  - 当前生效版：**409**；`__draft__`：**400**

- `PATCH /v1/project/config/{ds_id}/versions/{content_rev}`
  - 用途：更新该正式版的**备注**（`{ "note": "…" }`，空字符串表示清空）；配置快照仍不可变
  - `__draft__`：**400**

- `GET /v1/project/config/{ds_id}/versions`
  - 用途：列出正式版历史 + 若有编辑中临时版则首行 `__draft__`（`isDraft: true`）；含 `activeContentRev`、`appliedContentRev`、`draftOpen`、每项 `note` / `isActive`

- `GET /v1/project/config/{ds_id}/versions/compare?from={rev}&to={rev}`
  - 用途：两版展开 JSON 比对（`from`/`to` 可为 `__draft__`）；响应含 `fromDocument`、`toDocument`（`claudeMd`、`rulesJson`、`skillsJson`、`mcpServersJson`、`allowedToolsJson` 等）、`changes` 顶层摘要、`same`；不含 `gitSyncJson`（Git 仅在 `project_config` 行）

- `POST /v1/project/config/{ds_id}/versions/{content_rev}/activate`
  - 用途：将**生效版本**切换为指定历史 `content_rev` 并物化到 `home/`

- **L2 条目历史**（`domain`: `rule` | `skill` | `mcp` | `claude` | `tools`；`entity_key` 需 URL 编码，`claude`/`tools` 为 `_`）
  - `GET /v1/project/config/{ds_id}/entities/{domain}/{entity_key}/versions` — 该条目追加历史列表（`entityRev`、`createdAtMs`、`note`）
  - `GET .../versions/compare?from={entityRev}&to={entityRev}` — 两版 `fromBody` / `toBody` JSON 快照
  - `POST .../restore` body `{ "entityRev": "…" }` — 写回 `__draft__` 聚合字段，不切换 L1 生效版、不物化

- **全局配置（与 ds_id 无关）**
  - `GET /v1/gateway/global-settings` — `{ updatedAtMs, gitPats: [{ id, name, note?, createdAtMs, updatedAtMs, tokenSet }] }`（不返回 token 明文）
  - `POST /v1/gateway/global-settings/git-pats` — 创建/更新 PAT；body `{ id?, name, note?, token? }`（新建须 `token`；更新可省略 `token` 保留原值）
  - `DELETE /v1/gateway/global-settings/git-pats/{pat_id}` — 删除 PAT
  - 项目 `gitSyncJson` 使用 `gitPatId` 引用全局 PAT；推送时由网关解析 token，**不在** `project_config` 存 PAT 明文（兼容旧 `gitToken` 内联）

- `POST /v1/projects/{ds_id}/git/push`
  - 用途：将 `home/` 下**非 DB 物化**文件单向推送到远程（排除路径由当前 `project_config` 行计算，与物化规则一致）
  - 前置：`gitSyncJson.enabled=true` 且 URL/分支合法；会先按 DB 物化磁盘
  - 成功：`{ "dsId", "outcome": { "pushed", "commitId", "branch", "gitUrl" }, "gitSyncJson": { ... } }`（含 `lastPush*`）
  - 失败：**502**，`gitSyncJson.lastPushError` 会写入 PG

## Projects (ds workspace lifecycle)

- `GET /v1/projects`
  - 用途：Admin 项目列表；**以 PostgreSQL `project_config` 为准**（`skillsCountDb`、`claudeInDb`、`contentRev` 等），并附带磁盘就绪（`environmentPrepared`、`skillsCountDisk`、`dbSyncedToDisk`）
  - 响应：`{ "projects": [ ... ], "listedAtMs": <ms> }`；每项含 `gitSync` 摘要（`enabled`、`configured`、`gitTokenSet`、`lastPushOk`、`lastPushError` 等，无 PAT）

- `POST /v1/projects`
  - 用途：新建 `ds_<id>`（`work_root` + 空 `project_config` 行 + 占位 `CLAUDE.md`）；`dsId` 可选，省略则自动 `max(已有)+1`
  - 冲突：该 id 已存在于工作区或 `project_config` 时 **409**
  - 成功：同 `InitResponse`（`dsId`, `workDir`, `initialized`）

- `DELETE /v1/projects/{ds_id}`
  - 用途：删除 `work_root/ds_<id>`、`project_config` 行
  - Query：`purgeSessions`（默认 `true`）是否删除该 ds 的 `gateway_sessions` / `gateway_turns`
  - 无此 ds：**404**

## Project workspace

- `POST /v1/init`
  - 用途：要求已有 `project_config`，按 `content_rev` 物化到 `ds_<dsId>/home`（无 PG 行 **404**）
  - 轮询：可选 `CLAW_PROJECT_CONFIG_POLL_INTERVAL_SECS`（或旧名 `CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS`）——仅刷新有 PG 行的 ds

- `GET /v1/project/claude/{ds_id}`
  - 用途：读取 CLAUDE；优先 `project_config.claude_md`，否则磁盘 `home/CLAUDE.md`

- `POST /v1/project/claude/{ds_id}`
  - 用途：写入 `project_config.claude_md` 并物化（**不写** projects-git）
  - 请求体字段：
    - `content`：必填，写入 CLAUDE 文本
  - 落盘路径：
    - `ds_<dsId>/home/CLAUDE.md`
  - 返回字段：
    - `dsId`、`workDir`、`path`、`exists`、`content`

- `POST /v1/project/skills/{ds_id}`
  - 用途：合并写入 `project_config.skills_json` 并物化
  - 请求体：`skillName`（`[a-zA-Z0-9._-]`）、`skillContent`
  - 落盘：`ds_<dsId>/home/skills/<skillName>/SKILL.md`
  - 返回：`dsId`、`skillName`、`skillPath`、`created`、`updated`、`bytesWritten`、`workDir`
