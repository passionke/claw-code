# HTTP Gateway RS API

本文档是 Rust 网关对外 HTTP 接口清单，保持与运行代码一致。

Base URL 示例：`http://127.0.0.1:18088`

## Health

- `GET /healthz`
  - 用途：健康检查与关键运行配置回显
  - 回显字段含 `sessionDatabaseBackend`（`postgresql`）、`gatewayDatabaseUrl`（脱敏连接串）。会话/轮次/反馈表在 **PostgreSQL**，由环境变量 **`CLAW_GATEWAY_DATABASE_URL`** 指定（compose 默认连栈内 `postgres` 服务；生产可指向独立 PG 集群）。
  - **`deployImageRef`** / **`deployImageTag`**：由 **`./deploy/stack/gateway.sh up`** 带入的 **`GATEWAY_IMAGE`**（根 `.env` 或 `up --release release-vX.Y.Z` 写入的 `deploy/stack/.claw-image-release.env`）经 compose 注入 **`CLAW_GATEWAY_IMAGE_REF`**，无需单独配置。Admin 顶栏展示 `deployImageTag`：`…:local` → `local`；`…:release-v1.2.3` → `release-v1.2.3`。
  - **`clawTapCluster`**：内存态 clawTap 集群校验快照（`strict` / `mismatch` 等）；端点主机与 Live URL 在 Admin **`GET /v1/gateway/global-settings`** 的 **`clawTap`**（PG）中维护：`host`、`proxyPort`、`livePort`，以及派生字段 `proxyBaseUrl`、`liveBaseUrl`、`liveSessionUrlTemplate`（例 `http://192.168.9.252:3000/?session={sessionId}`）
  - **`liveReport`**：求解过程 stdout 实时 SSE（经 pool 代理），与 claude-tap Live 查看器无关

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
      - 若 `project_config.extra_session_fields_json` 非空：请求体须为 object，且**每个定义字段**均存在且值为 **string**（可为 `""`）；允许额外系统 key（`tenant_code`、`solution_code`、`biz_type`、`_claw_*`）。否则 `400`（`extraSession 不符合要求：…`）
      - enqueue 时将完整入口参数写入 `gateway_turns.entry_params_json`（含 `extraSession`）；`GET /v1/sessions/{sessionId}/turns` 每项返回 `extraSession` 快照
    - `allowedTools`：可选，字符串数组，指定**本次 solve**允许暴露给模型并执行的工具名（与异步 `/v1/solve_async` 相同）。
      - **未传 `allowedTools`**：沿用网关进程环境变量 `CLAW_ALLOWED_TOOLS`（逗号分隔，与 `GET /healthz` 中 `allowedTools` 字段一致）。若该环境变量也未配置（空），则下游 `gateway-solve-turn` 将空列表视为「不额外收紧」，**内置 MVP 工具（含 `bash` 等）会全部挂上**。
      - **已配置全局 `CLAW_ALLOWED_TOOLS`（非空）**：请求里的 `allowedTools` 若出现，则其中**每一项**都必须被全局策略放行（支持前缀通配：全局项若以 `*` 结尾，则匹配该前缀开头的工具名；请求侧若以 `*` 结尾，则该项须与全局列表中的某一项字面完全一致）。否则返回 `400`，提示 `requested tool pattern is not allowed by gateway policy`。因此若部署里把全局白名单写得很窄（例如仅 `read_file,glob_search`），仅靠请求体无法「偷偷」加上 `bash`，需要先把 **`CLAW_ALLOWED_TOOLS` 扩大到包含 `bash`（或 `bash*`）等**。
      - 常见内置名（与 `rust/crates/tools` 中 `mvp_tool_specs` 一致，按需选用）：`bash`、`read_file`、`write_file`、`edit_file`、`glob_search`、`grep_search`、`WebFetch`、`WebSearch`、`MCP`、`Skill`、`TodoWrite` 等；MCP 动态工具名按运行时注册为准。
      - 典型「交给 resolve/solve 里强 agent 自决」时，在**全局白名单已包含**的前提下，可在一次调用里显式放宽，例如：`"allowedTools": ["read_file","glob_search","grep_search","bash","write_file","edit_file","MCP"]`。
  - 追踪约定：
    - 网关会为本次调用确定 `sessionId`（等于 `claw-session-id`）
    - 响应体主字段使用 `sessionId`，并保留 `requestId` 兼容字段（同值）
    - 响应体含 `sessionHomeRel`：相对 `CLAW_WORK_ROOT` 的会话工作目录（与 PG 表 `gateway_sessions.session_home` 一致），与 `workDir`（绝对路径）成对出现；含 **`turnId`**（当次轮次，`T_<32位小写hex>`）。**新建会话**时目录名为 `ds_{dsId}/sessions/<segment>`：在 `sessionId` 可作为安全单段路径名时 `<segment>` **与 `sessionId` 相同**（网关生成的 32 位十六进制 id 即落在该目录下）；若 `sessionId` 含路径分隔符等不安全字符，则 `<segment>` 为对该 id 做 UUID v5 派生的 32 位十六进制名（与 id 一一对应、可复现）。续聊仍按库中已有 `session_home` 打开原目录。
    - 在访问上游模型时透传 HTTP 头：
      - `clawcode-session-id: <sessionId>`
      - `claw-session-id: <sessionId>`
    - 在访问下游 MCP 服务（包括 SQLBot）时，`tools/call` 的 `_meta` 仅含 `extra_session` 对象（详见 [`gateway-mcp-call-meta.md`](gateway-mcp-call-meta.md)）：业务字段来自请求体 `extraSession`，并注入 `_claw_session_id`、`_claw_turn_id` 供串联。非 MCP HTTP 出站 header。
  - 对话状态：worker 容器内用 `.claw/gateway-solve-session.jsonl` 续聊；读回后 HTTP 消费端只读 PG `cc_messages`（`render_session_jsonl`）。见 [`docs/pool-v1-consumer-matrix.md`](pool-v1-consumer-matrix.md)。
  - **Solve preflight（按项目、可选）**：在 `ds_<id>/home/.claw/solve-preflight.json` 声明，例如 `{"kinds":["sqlbot_mcp_start"]}`（兼容历史 `{"kind":"sqlbot_mcp_start"}`）。仅**该 `sessionId` 第一次**（尚无 `gateway-solve-session.jsonl`）时：先写入用户问题，再按 `kinds` 顺序执行 preflight 并注入 transcript（当前仅 `sqlbot_mcp_start`：一次 `mcp_start`，暴露 `access_token` / `chat_id`）。续聊 turn 不跑 preflight。表结构不在 transcript 注入：由外部 job 维护 `ds_<id>/home/schema.md`（`CREATE TABLE` DDL），worker ro mount 到 `home/schema.md`，系统提示词引导模型读取该文件。

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
  - **Live 报告**：stdout-v1 全链路见 [`docs/live-report-contract.md`](live-report-contract.md)（含顺序保证、已知缺陷与 `pack-deploy` 验收）

- `POST /v1/internal/turns/{turnId}/stdout-event`
  - 用途：池 daemon 转发 worker 结构化 stdout（JSON body，含 `ev` / `text` 等）
  - 鉴权：请求头 `x-claw-gateway-internal-token` 或 `Authorization: Bearer <CLAW_GATEWAY_INTERNAL_TOKEN>`

- `GET /v1/tasks/{task_id}`
  - 用途：查询异步任务状态与结果
  - 响应含 **`turnId`**（与本次 async 入队时返回的值一致）
  - **网关重启后**：若进程内已无该 `taskId` 的内存任务，网关会按 `session_id = task_id` 从 PostgreSQL 读取 **`gateway_turns` 最新一行** 重建 `TaskRecord`（含 `status`、`result`/`error`、`turnId` 等），以便 BFF 继续轮询。
  - 响应除 `status` 外含 **`currentTaskDesc`**、`progressHistory`、`todos`：来自 `gateway_turns.solve_timing_jsonb`（`taskProgress` / `progressEvents`）。
  - **`running` 中间进度**：每次 poll 先经 **宿主机 pool daemon** RPC `sync_turn_progress`（`podman exec` worker → upsert PG），再读 PG 返回；gateway 容器内直接 exec worker **无效**（根因与修法见 [`docs/pool-v1-consumer-matrix.md`](pool-v1-consumer-matrix.md) § Running `report_progress`）。`queued` 时返回「排队中（x 个等待，y 个执行中）」；`running` 且尚无 PG 上报时返回「处理中」或「工具调用中」。**不**从 jsonl 最后一条 assistant 推导。
  - **升级**：gateway 与 pool daemon 须同版本，否则 running 中间 progress 仍空。
  - 另含 `dsId`、`progressUpdatedAtMs`、**`hasReport`**、**`reportTime`**：见 [`docs/live-report-contract.md`](live-report-contract.md) §6.4。成功后的报告正文在 **`result.outputJson.message`**（非响应顶层 `outputJson`）。

- `GET /v1/sessions/{session_id}/turns/{turn_id}/tools?ds_id=<int>`
  - 用途：查看该 **gateway `turnId`** 对应用户轮次的全部 **tool 调用**（`tool_use` 入参 + `tool_result` 返回）。从 PG `render_session_jsonl` 解析；时间戳来自同 turn 的 `solve_timing_jsonb.progressEvents`。
  - 响应：`tools[]` 含 `toolUseId`、`toolName`、`input`（JSON）、`output`、`isError`；超大字段按 `CLAW_TURN_TOOLS_MAX_FIELD_CHARS`（默认 120000）截断并标 `*Truncated`
  - 未知 session / turn：404

- `GET /v1/sessions/{session_id}/execution?ds_id=<int>`
  - 用途：按 `(sessionId, dsId)` 查看当前进度快照、`progressHistory`、网关队列统计、脱敏 trace 尾（`include_trace=true` 时含更多字段）。`progressHistory` 来自 PG `solve_timing_jsonb.progressEvents`（当前 turn）。
  - `progressHistory` 每条 `message` 默认最多 **80** 个 Unicode 字符，超出截断并追加 `...`；环境变量 **`CLAW_PROGRESS_MESSAGE_MAX_CHARS`**（正整数）可覆盖。事件 `kind`：`report_progress`（模型 `report_progress` 工具上报）、`mcp_tool_started`（NL 查询类 MCP 发起时一条；不追加 `mcp_tool_completed` / `mcp_tool_failed`，避免重复或失败文案刷屏）

- `GET /v1/sessions/{session_id}/turns/{turn_id}/timeline?ds_id=<int>`
  - 用途：单轮 swimlane（progress / tool / LLM 等 lane）。来自 PG `solve_timing_jsonb`。
  - 无该会话行：404

- `POST /v1/sessions/{session_id}/turns/{turn_id}/cancel?ds_id=<int>`
  - 用途：按 **`sessionId` + `turnId` + `dsId`** 取消指定轮次（推荐 Admin / BFF 使用）
  - 若该轮次对应当前内存中的 async worker（`record.turnId` 一致）：`abort` worker、`force_kill_slot`（有租约时）、`gateway_turns` → `cancelled`
  - 若内存中无任务或活跃任务属于**另一** `turnId`：先对 PG 中该 `turn_id` 行做 cold cancel（`queued`/`running` → `cancelled`）；若 cold 成功且内存里仍有同 session 的 `queued`/`running`（更新的 turn），网关会**一并释放**该内存任务（abort worker、`force_kill_slot`、移除 `tasks` 表项），避免产品侧已收到 `cancelApplied: true` 仍 `409`。
  - 终态幂等：返回 `200`，`cancelApplied: false`，`error` 说明未再取消
  - 未知 `(session_id, turn_id, ds_id)`：**404**

- `POST /v1/tasks/{task_id}/cancel`
  - 用途：按 `taskId`（与异步会话 `sessionId` 同值）取消仍处于 `queued` 或 `running` 的 solve 异步任务（等价于取消该 session **最新一轮**）
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
  - 用途：turn `queued`/`running` 且 `stream=true` 时走 **stdout hub** live SSE（无 LLM 润色）；`succeeded` 后 `stream=true` 走 LLM 润色（同 `biz_advice_report_bak`）；`stream=false` 仅终态 JSON
  - 查询参数：`sessionId`、`turnId`（`T_<32 hex>`）、`dsId`（≥1）必填；`stream` 默认 `true`
  - Live SSE：`biz.report.start` / `biz.report.delta` / `biz.report.done`；尾段完整性依赖 `HubMsg::SolveDone`（见 [`docs/live-report-contract.md`](live-report-contract.md) §7.4）
  - 非流式（`stream=false`）：仅 turn 终态可读，返回 JSON（`reportText` / `reportJson.message`）

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
  - 请求体（camelCase）：`rulesJson`、`mcpServersJson`、`skillsJson`、`allowedToolsJson`、`extraSessionFieldsJson`（`string[]`，省略则保留库内已有）、`claudeMd`、`gitSyncJson`（省略则保留 Git 配置）；`skillsSourcesJson` 须为 `[]`
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
  - `GET /v1/gateway/global-settings` — `{ …, clawTap?, clusterId? }`（`clusterId` 只读，来自 `CLAW_CLUSTER_ID`；无修改接口）
  - `PUT /v1/gateway/global-settings/claw-tap` — `{ host, proxyPort }`（必选；保存前须 probe 通过 clusterId+hash）
  - `POST /v1/gateway/global-settings/claw-tap/probe` — `{ host, proxyPort }` → `{ ok, clusterId?, dbHost?, clusterHash?, localClusterHash?, clusterMatch?, hashMatch?, message }`
  - `GET /readyz` — 503 直至 `clawTapCluster.consistency=strict`；`GET /healthz` 含 `clawTapCluster`（`strict` | `cluster_mismatch` | `unconfigured`）
  - 求解 `output_json.llmRoute` — `{ mode, clusterId, clusterHash, clawTapBaseUrl?, upstreamBaseUrl, model, reason? }`
  - `POST /v1/gateway/global-settings/llm-models` — 新建/更新一条模型：`{ id?, name, baseModelUrl, modelName, apiKey? }`（新建须 `apiKey`）
  - `POST /v1/gateway/global-settings/llm-models/{model_id}/apply` — 设为当前并同步 `.env` + `.claw/claw-tap-upstream.json`
  - `DELETE /v1/gateway/global-settings/llm-models/{model_id}` — 删除一条模型
  - `PUT /v1/gateway/global-settings/active-llm-config` — 兼容旧客户端：更新当前/首条并 apply
  - `GET .../versions` — 无版本历史（`versions: []`）
  - 网关后台默认每 **30s** 轮询 DB 全局 LLM（`CLAW_GATEWAY_LLM_CONFIG_POLL_INTERVAL_SECS`，`0` 关闭）；upstream 变更写 JSON 文件，tap 约 2s 内生效
  - **落盘契约**（`gateway.sh up` / `tap-up` 生成 `deploy/stack/.claw-llm-runtime.env`）：宿主机 `${repo}/.env`（`OPENAI_API_KEY` / `CLAW_DEFAULT_MODEL`）与 `${repo}/.claw/claw-tap-upstream.json`（`{"target":"https://..."}`）；gateway 容器 rw 挂载 `/run/claw/worker.env` + `/run/claw/claw/…`，与 claude-tap `--tap-upstream-config`、pool worker 读同一宿主文件
  - `DELETE /v1/gateway/global-settings/llm-models/{model_id}` — 删除模型及其全部 revision
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

- `GET /v1/projects/{ds_id}/sessions`
  - 用途：Admin 对话记录列表（keyset 分页 + 筛选）
  - Query：`limit`（默认 20）、`beforeUpdatedAtMs` + `beforeSessionId`（翻页）、`updatedFromMs` / `updatedToMs`（按 `gateway_sessions.updated_at_ms`）、`q`（首问 `user_prompt` ILIKE）、`sessionId`（`T_<32hex>` 精确到 turn，否则 `session_id` 片段）、`extraSession`（URL 编码 JSON 对象，**仅允许** `project_config.extra_session_fields_json` 中的 key；对每个 key 在**任一轮** `gateway_turns.entry_params_json.extraSession` 上 ILIKE 子串匹配，多 key 为 AND）
  - 响应：`{ "dsId", "sessions": [ { "sessionId", "createdAtMs", "updatedAtMs", "turnCount", "previewPrompt", "clientOrigin"?, "hasBadFeedback", "hasGoodFeedback" } ], "hasMore" }`（反馈标记：该会话任一轮在 `gateway_feedback` 中出现过 `bad` / `good`）

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
