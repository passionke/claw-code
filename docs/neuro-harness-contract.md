# Neuro Harness Worker 契约 v1

Author: kejiqing

适用范围：`neuro-opencode`、`neuro-appserver` 这类通过 ACP（Agent Client Protocol）驱动外部引擎的 worker CLI。claw 引擎（`claw gateway-solve-once`）不受本文约束，现状不变。

本文是实验分支 `feat/neuro-harness` 的契约。分支会长期存在，合入主干前以本文为准。

## 1. 入口与进程模型

```text
<bin> gateway-solve-once --task-file <session_root>/gateway-solve-task.json
<bin> mcp-proxy --server <name> --session-root <session_root>      # 仅由引擎子进程调用
```

- 每个引擎一个固定的 bin，不接受 `--engine` 参数：`neuro-opencode`、`neuro-appserver`。
- 执行环境与 claw 相同（`deploy/e2b/e2b_exec.py`）：`cwd = HOME = /claw_sessions/{seg}`，`CLAW_PROJECT_CONFIG_ROOT=/claw_ds/project_home_def`。
- 一次 exec 只跑一轮：spawn 引擎子进程，经 stdio 走 ACP JSON-RPC，本轮结束后关闭子进程。
- gateway 的布局和命名本期不动：`.claw/`、`CLAW_*`、`__CLAW_GATEWAY_STDOUT__`、`clawExitCode` 全部沿用。

## 2. 输入

`GatewaySolveTaskFile`（`rust/crates/gateway-solve-turn/src/lib.rs`）原样复用，读取的字段：

| 字段 | 用法 |
| --- | --- |
| `userPrompt`、`attachments` | 组成 ACP `session/prompt` 的内容：每个附件一个 `resource_link`（`file://{session_root}/{path}`，ACP 规定所有 agent 都必须支持），然后是正文 text；transcript 的 user 消息复用 `build_user_turn_message` |
| `sessionId`、`turnId`、`requestId`、`extraSession` | 写入 `.neuro-harness/turn-context.json`，供 mcp-proxy 构造 `_meta` |
| `model` | 为空时取 `CLAW_DEFAULT_MODEL` |
| `maxIterations` | 客户端统计 `tool_call` 次数，超出后发送 `session/cancel` |
| `timeoutSeconds` | 本轮内部截止时间；gateway 侧仍有自己的超时和 `force_kill_slot` |
| `landlockDsl`、`landlockDslSource` | spawn 引擎前对自身执行 `apply_strict_landlock_jail`，子进程继承 |
| `responsesStream` | 为 true 时才输出 `thinking.delta` |
| `llmRoute` | 只作为审计快照回写到 `outputJson.llmRoute` |

不支持的能力：task 中出现 `interactionMode=plan`、`askUserQuestionEnabled=true`、`sealedPlanId` 或 `sealedPlanMarkdown` 时，直接输出 `solve.done` 错误 `unsupported_by_engine: <field>`（`httpStatusHint=400`），不启动引擎。gateway 侧也会拦截，两边都把关。

环境变量（由 gateway exec 注入，与 claw 相同）：`OPENAI_BASE_URL`（tap 代理）、`OPENAI_API_KEY`（占位值 `claw-tap-cluster`）、`CLAW_DEFAULT_MODEL`、`CLAW_SESSION_ID`、`CLAW_TURN_ID`。

## 3. 输出

stdout 中以 `__CLAW_GATEWAY_STDOUT__` 开头的行是结构化事件，其他行一律视为日志。**每次 exec 最后必须恰好有一条 `solve.done`**：成功、失败、引擎崩溃、超时、收到 SIGTERM/SIGINT 都一样。

### 3.1 ACP 事件到契约事件的映射

| ACP `session/update` | 契约事件 | 说明 |
| --- | --- | --- |
| `agent_message_chunk`（text） | `report.delta{text, emitSeq}` | 同时累积为本轮 `message` |
| `agent_thought_chunk`（text） | `thinking.delta{text}` | 仅 `responsesStream=true` 时输出 |
| `tool_call` | `tool.start{toolCallId, name, kind, title, argsSummary}` | `name` 取 ACP `name`，没有时取 `title` |
| `tool_call_update`，status 为 `in_progress` 且带内容 | `shell.chunk{toolCallId, text}` | 仅 kind 为 `shell` 的工具 |
| `tool_call_update`，status 为 `completed` / `failed` | `tool.end{toolCallId, name, kind, status, durationMs, resultSummary, output}` | `output` 取 content 中的文本，没有时取 `rawOutput` 的 JSON |
| `usage_update` | 不输出 | 只是上下文占用，不计入 token 用量 |
| `plan`、`available_commands_update`、`session_info_update`、`current_mode_update`、`config_option_update`、`user_message_chunk` | 不输出 | 首期忽略 |

`kind` 沿用现有的取值集合（`gateway_stdout.rs` 的 `tool_process_kind`），不新增取值：

| ACP `ToolKind` | 契约 `kind` |
| --- | --- |
| `execute` | `shell` |
| `edit`、`delete`、`move` | `edit` |
| `read` | `read` |
| `search` | `search` |
| 工具名或 title 能匹配到本轮注入的 MCP server（`mcp.<server>.`、`mcp__<server>`、`<server>_`） | `mcp` |
| 其余（`think`、`fetch`、`switch_mode`、`other`） | `tool` |

### 3.2 `solve.done`

成功：

```json
{"ev":"solve.done","clawExitCode":0,"outputText":"<outputJson 序列化>","outputJson":{
  "model":"<wire model>","iterations":<本轮 tool_call 次数>,"message":"<本轮正文>",
  "completionReason":"model_end_turn|max_tokens|max_turn_requests|max_iterations",
  "usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0} | null,
  "harnessEngine":"opencode|appserver","llmRoute":{...}}}
```

失败：`{"ev":"solve.done","clawExitCode":1,"error":"...","httpStatusHint":<400|500|504>}`。

`stopReason` 的处理：

| `stopReason` | 结果 |
| --- | --- |
| `end_turn` | 成功，`completionReason=model_end_turn`（与 claw 同值） |
| `max_tokens`、`max_turn_requests` | 成功，`completionReason` 原样带出 |
| `cancelled`，且是客户端因 `maxIterations` 发起的 | 成功，`completionReason=max_iterations` |
| `cancelled`（其他原因）、`refusal` | 失败，500 |
| JSON-RPC 错误、引擎子进程退出 | 失败，500 |
| 本轮超过 `timeoutSeconds` | 失败，504 |
| 收到 SIGTERM / SIGINT | 失败，500 |

`usage` 取自 ACP `PromptResponse.usage`（schema 中的 unstable 字段 `unstable_end_turn_token_usage`）。引擎没给时写 `null`，gateway 按 0 计。口径见第 7 节的 spike 结论：两个引擎报的都可能是最后一次 LLM 调用的用量，不是整轮累计。权威的 token 计量仍以 tap 为准。

`iterations` 是本轮 `tool_call` 的次数，不是 LLM 往返次数。ACP 不暴露后者。

## 4. Transcript

- 文件：`{session_root}/.claw/gateway-solve-session.jsonl`，复用 `runtime::Session` 写入，行格式与 claw 完全一致：`{"type":"message","message":{"role","blocks","usage"}}`。
- **只追加**：本轮开始时追加 user 消息；本轮结束后追加 assistant / tool 消息。gateway 每次读回都会先删后导，所以这个文件天然就是整个 session 的完整历史。
- 角色与块：
  - user：`build_user_turn_message(prompt, attachments)`，与 claw 相同。
  - assistant：`text`、`tool_use{id, name, input}`；本轮的 `usage` 挂在最后一条 assistant 消息上。
  - tool：`tool_result{tool_use_id, tool_name, output, is_error}`。不能用 user 角色，否则 gateway 的 `turn_message_groups_from_jsonl_contents` 会错误切分轮次。
- 引擎自己的会话状态（opencode 的 sqlite、Codex 的 rollout 和 sqlite）只用于续接，不作为 gateway 的数据源。

## 5. 读写范围

- 元数据只从两处读取：
  - `$CLAW_PROJECT_CONFIG_ROOT`（`project_home_def`，路径解析复用 `runtime::gateway_project_config_root`）：`CLAUDE.md` 等指令文件、`.cursor/rules`、`.claw/skills`。指令文件的发现规则复用 `runtime::ProjectContext`，与 claw 相同。`instructions.md` 依次包含：指令文件、rules、claw 的 pool 布局说明（`gateway_pool_layout_prompt_section`，`CLAW_GATEWAY_WORK_ROOT` 存在时）、`extraSession` 块。
  - `{session_root}/.claw/settings.json`：MCP 配置。与 claw 的 `initialize_mcp_runtime` 一致，用 `ConfigLoader::default_for(session_root)` 加载，不读 `project_home_def` 里带 `${…}` 模板的那份。
- 私有状态只写在 `{session_root}` 下：
  - `.neuro-harness/`：`state.json`（引擎和 agent sessionId）、`turn-context.json`、`instructions.md`、`acp-trace.ndjson`（本轮的原始 ACP 流）、引擎数据（opencode 的 `opencode.db`）。
  - `.config/opencode/`、`.local/`、`.cache/`：opencode（`XDG_*` 全部指向 session 目录）。
  - `.codex/`：Codex 的 `CODEX_HOME`。
  - `tmp/`：`TMPDIR`。

## 6. 会话续接

- `.neuro-harness/state.json` 不存在：`session/new`，然后把 `{engine, agentSessionId}` 写入 state.json。
- state.json 存在：
  - `engine` 与当前 bin 不一致：报错。引擎在项目创建后不可修改。
  - agent 支持 `sessionCapabilities.resume`：用 `session/resume`。
  - 否则 agent 支持 `loadSession`：用 `session/load`。load 期间引擎会回放历史 update，客户端**丢弃 load 响应之前的所有 update**。
  - 两者都不支持：报错，不做兜底。
- MCP 注入：`session/new`、`session/resume`、`session/load` 都带上同一组 `mcpServers`。session 配置里的每个 MCP server（stdio、http、sse）都替换成一个 stdio 类型的 mcp-proxy：

```json
{"name":"<server>","command":"<当前 bin 的绝对路径>","args":["mcp-proxy","--server","<server>","--session-root","<session_root>"],"env":[<worker 进程的全部环境变量>]}
```

env 显式传入 worker 的全部环境变量：Codex 会清洗 MCP 子进程的环境，而上游 MCP server 应该看到与 claw 下相同的环境。

## 7. mcp-proxy

- 对引擎来说是一个 stdio MCP server；对上游用 `runtime::McpServerManager` 连接真实 server（stdio、http、sse），行为与 claw 相同。
- `tools/list`：返回上游的原始工具名和 schema。
- `tools/call`：每次调用都重新读取 `turn-context.json`，用 `runtime::build_mcp_call_meta` 生成 `{"extra_session": {...}}`，与引擎自带的 `_meta` 合并（同名 key 以 gateway 为准），再转发给上游。
- 不写 stdout 事件。`tool.*` 事件由主进程根据 ACP 事件统一输出。
- 首期只代理 tools，不代理 resources 和 prompts。

## 8. Engine profile

每个 bin 只负责：启动命令、provider 配置、指令和 skills 的投影。配置每轮重新生成，项目在 session 进行中发布新版本时，下一轮就能生效。

引擎安装位置固定在 `/usr/local/lib/neuro-engines/`（landlock 默认 ro 覆盖 `/usr`）。`NEURO_OPENCODE_BIN`、`NEURO_CODEX_ACP_BIN` 只用于本机开发时指向本地安装，镜像中不设置。

### neuro-opencode（`opencode acp`，v1.18.34）

- 启动：`/usr/local/lib/neuro-engines/opencode/bin/opencode acp`。
- `OPENCODE_CONFIG={session_root}/.neuro-harness/opencode.json`：
  - `provider.neurogate`：`@ai-sdk/openai-compatible`，`baseURL` 取 `OPENAI_BASE_URL`，`apiKey` 写 `{env:OPENAI_API_KEY}`。
  - `model` 和 `small_model` 都是 `neurogate/<model>`。
  - `instructions` 指向 `.neuro-harness/instructions.md`。
  - `permission` 全部 allow，`autoupdate=false`，`share="disabled"`，`plugin=[]`。
- 环境变量：`OPENCODE_DISABLE_CLAUDE_CODE=1`、`OPENCODE_DISABLE_AUTOUPDATE=1`、`OPENCODE_DISABLE_MODELS_FETCH=1`、`OPENCODE_DISABLE_DEFAULT_PLUGINS=1`、`OPENCODE_DISABLE_LSP_DOWNLOAD=1`、`OPENCODE_DISABLE_SHARE=1`、`OPENCODE_DB={session_root}/.neuro-harness/opencode.db`，以及 `XDG_*` 指向 session 目录。
- skills：每轮重建 `{session_root}/.config/opencode/skills`，`$CLAW_PROJECT_CONFIG_ROOT/.claw/skills` 下每个含 `SKILL.md` 的目录建一个软链接。opencode 只加载名字符合 `^[a-z0-9]+(-[a-z0-9]+)*$` 的 skill，不符合的不链接，并在 stderr 记录跳过的名字。

### neuro-appserver（`codex-acp` 2.1.1 + codex 0.159.3）

- 启动：`/usr/local/lib/neuro-engines/codex-acp/node_modules/.bin/codex-acp`。目录由 `npm ci` 按提交的 lockfile 安装，codex 用 codex-acp 自带解析（不设 `CODEX_PATH`），版本由 lockfile 锁定为 0.159.3。
- `CODEX_HOME={session_root}/.codex`。
- `config.toml`：`model`、`model_provider="neurogate"`、`[model_providers.neurogate]`（`base_url` 取 `OPENAI_BASE_URL`，`env_key="OPENAI_API_KEY"`，`wire_api="responses"`）、`approval_policy="never"`、`sandbox_mode="danger-full-access"`、`check_for_update_on_startup=false`、`[analytics] enabled=false`、`model_catalog_json`。
- `model_catalog_json`：只含当前模型的一条目录，内容与 Codex 内置的 fallback 元数据等价（`model_info_from_slug`，`rust-v0.159.3`），`model_messages.instructions_template` 取同版本的 `prompt.md`（`rust/crates/neuro-harness/assets/codex/rust-v0.159.3/prompt.md`，编译期嵌入）。两处与 fallback 不同：`visibility="list"`（否则 `model/list` 为空，codex-acp 的 `session/new` 报 "Codex did not return any models"）；`context_window` 在 `CLAW_CONTEXT_WINDOW_TOKENS` 存在时取该值。不配这一项时，Codex 会把 "Model metadata for `<model>` not found" 警告作为正文 chunk 输出。
- 环境变量：`INITIAL_AGENT_MODE=agent-full-access`（隔离交给 landlock）。
- 指令：`$CODEX_HOME/AGENTS.md` 写入 `.neuro-harness/instructions.md` 的内容（Codex 的全局指令文件）。
- skills：每轮重建 `{session_root}/.agents/skills`，`$CLAW_PROJECT_CONFIG_ROOT/.claw/skills` 下每个含 `SKILL.md` 的目录建一个软链接。

## 9. Spike 结论（2026-10-02，本机 macOS，mock LLM）

复现方法：`deploy/neuro-harness/spike/` 下的 `mock_openai.py`（chat completions 和 responses）、`mock_mcp.py`（记录 `tools/call` 的完整 params）、`acp_drive.py`（单轮 ACP 驱动，记录原始流）。原始流已脱敏归档到 `rust/crates/neuro-harness/tests/fixtures/`。

| 假设 | 结论 | 证据 |
| --- | --- | --- |
| 跨进程续接 | 两个引擎都支持 `session/resume`，在新进程里续接后，下一次 LLM 请求带上了完整历史；resume 不回放历史 update | opencode turn2：`mode=resume`，LLM 请求中出现第一轮的 user 和 assistant；codex turn2：第二次 `/responses` 的 input 带上了 `msg_1` |
| 续接耗时 | opencode：initialize 约 460ms，resume 约 270ms；codex-acp：initialize 约 200ms，resume 约 30ms | `acp_drive.py` 输出的 `ms` 字段 |
| MCP 注入 | 两个引擎都能列出并调用 stdio MCP。opencode 发出的 `_meta` 只有 `progressToken`；Codex 会带自己的 `x-codex-turn-metadata` 等字段。两者都不会带 gateway 的 `extra_session`，所以 mcp-proxy 注入是必需的 | `mcp.ndjson`：opencode `{"_meta":{"progressToken":2}}`；codex `{"_meta":{"callId":…,"x-codex-turn-metadata":…}}` |
| token 用量 | 两个引擎都在 `PromptResponse.usage` 里返回；带工具调用的一轮（两次 LLM 调用，每次 11/7）也只报了 11/7，像是最后一次调用的值。**待 e2b 用真实上游确认口径** | turn2 的 `prompt_result.usage` |
| 离线运行 | 两个引擎在外网不可达（代理指向不可达地址，只放行 127.0.0.1）时都能正常完成；opencode 需要上面列出的 `OPENCODE_DISABLE_*` 开关，否则会在运行时 npm 安装插件 | oc2 / cx turn6，均约 2s 完成，无错误日志 |
| codex 的 MCP 工具形态 | Codex 用 Responses API 的 `namespace` 工具（`{"type":"namespace","name":"mcp__<server>","tools":[...]}`）。**上游是否支持 namespace 工具需要在 e2b 上用真实上游验证** | mock 记录的 `/responses` 请求体 |
| codex 的未知模型警告 | 不配 `model_catalog_json` 时，正文开头会多一段警告；配上之后消失，base instructions 不变（20751 字节） | turn5：`grep -c "Model metadata"` 为 0 |
| SDK | 采用官方 `agent-client-protocol` 2.2.0（依赖 `agent-client-protocol-schema` =1.9.1，ACP protocol v1），开启 `unstable_end_turn_token_usage` 才能解析 `PromptResponse.usage` | crate 源码 `Cargo.toml` 和 `src/v1/agent.rs` |

本机端到端（`deploy/neuro-harness/local_e2e.sh opencode|appserver`，两个 bin + mock LLM + mock MCP，两轮：普通对话、续接 + MCP 工具调用）两个引擎都通过：`solve.done` 形态、`tool.start/end kind=mcp`、transcript 角色 `user, assistant, user, assistant, tool, assistant`、上游 MCP 收到的 `_meta.extra_session` 含 `org_id`、`_claw_session_id`、`_claw_turn_id`；LLM 请求中出现 CLAUDE.md、skill、extraSession 块和上一轮历史。`maxIterations=0` 得到 `completionReason=max_iterations`；`interactionMode=plan` 得到 400 `unsupported_by_engine: interactionMode`。

尚未验证，需要在 e2b 上确认的：

1. opencode 的 sqlite（WAL）和 Codex 的多个 sqlite 放在 NAS 上能否正常加锁。
2. landlock 下 bun（opencode）和 node（codex-acp）对 `/dev`、`/proc` 的访问。默认 DSL 的 ro 列表里没有这两个路径。
3. codex 经 tap 访问真实上游 `/responses`（含 SSE 和 namespace 工具）。
4. usage 的口径。

## 10. gateway 接入（`http-gateway-rs/src/pool/harness_engine.rs`）

引擎相关的知识只放在这一个模块里，热点文件只加一行调用：

| 位置 | 行为 |
|---|---|
| `POST /v1/projects` | 新增 `harnessEngine` 字段，缺省为 `claw`，非法值返回 400。appserver 项目要求生效中的 LLM `baseModelUrl` 以 `/responses` 结尾，否则返回 400。插入项目之后用一次 UPDATE 写入引擎列；之后所有 upsert 和发布路径都不会碰这一列。 |
| `PUT /v1/projects/{id}/role` | 非 claw 项目只能设为 `normal`，其他角色返回 400 `unsupported_by_engine`。 |
| `PUT /v1/projects/{id}/config` | 非 claw 项目写入 `workerProfileJson.mode=relaxed` 时返回 400。 |
| solve（`run_solve_request_docker`） | 非 claw 项目只能走 e2b 后端。`interactionMode=plan` 或带 sealedPlan 时返回 400。appserver 每次 solve 都重新校验 `/responses`。task 固定为 `interactionMode=agent`、`askUserQuestionEnabled=false`、`forceSingleTurn=true`。exec 的 bin 换成 `/usr/local/bin/neuro-{opencode,appserver}`。 |
| `desired_worker_spec` | 模板取自 PG 的 `e2bWorkerOpencode` / `e2bWorkerAppserver`（`templateId` 为空时用 alias `claw-worker-{opencode,appserver}`，`buildId` 为可选 pin），固定使用 strict。contract key 的 profile 段写成 `strict+<engine>`，claw 项目的 key 不变。 |
| 列表接口、config 接口 | 返回字段 `harnessEngine`。读取失败时返回 `null`，不回落成 claw。 |

`/responses` 的判定新增在 `gateway_tap_client::base_model_url_is_responses`，路径归一化规则与 tap client 相同。注意 `tap_client_from_base_model_url` 不能直接用来判定：它对 chat/completions 和无后缀的 URL 也返回缺省值 `codex`。

## 11. 构建与镜像

- `rust/crates/neuro-harness` 是独立 cargo workspace：自带 `[workspace]` 和 `Cargo.lock`，并在 `rust/Cargo.toml` 中 `exclude`。原因：ACP SDK 会打开 serde_json 的 `preserve_order` / `raw_value` 特性。如果和 claw、gateway 在同一次 cargo 调用里构建，resolver-2 会把这些特性合并进去，改变它们的行为。实测证据：`cargo test -p rusty-claude-cli --test mock_parity_harness` 单独运行时通过，加上 `-p neuro-harness` 后失败；1.88 clippy 在 `runtime/src/mcp_stdio.rs` 报出 4 个新的 `result_large_err`。因此：
  - `linux-compile.sh` 对它单独调用一次 `cargo build --manifest-path crates/neuro-harness/Cargo.toml`，使用同一个 `CARGO_TARGET_DIR`。
  - `.githooks/pre-push` 发现自带 `Cargo.lock` 的 crate 时，用 `--manifest-path` 单独执行 fmt、clippy（`--all-targets -D warnings`）和 test，不会因此触发 workspace 全量检查。
  - 本地产物位于 `rust/crates/neuro-harness/target/`。
  - 已知缺口：`rust-ci.yml` 的 `cargo test --workspace` 不覆盖这个 crate，目前只靠 pre-push 和 `neuro-harness-worker.yaml` 的编译覆盖。
- `deploy/stack/lib/linux-compile.sh`：产物统一由 `CLAW_LINUX_RELEASE_BINS` 列出，包括 `claw`、`http-gateway-rs`、`neuro-opencode`、`neuro-appserver`。
- npm 源统一用 `claw_npm_registry`（`deploy/stack/lib/claw-region.sh`）：region 为 `china` 时用 `registry.npmmirror.com`，其他情况用 `registry.npmjs.org`。已实测：同一份 lockfile 用 `npm ci --registry=https://registry.npmmirror.com` 安装时，全部从 npmmirror 下载，integrity 校验通过（npm 默认 `replace-registry-host=npmjs`）。
- `Containerfile.gateway-worker-opencode`：`FROM claw-gateway-worker:<tag>`。opencode 用 glibc 平台包 `opencode-linux-{x64,arm64}@1.18.34`。与计划的偏差：计划写的是 musl 包，但 worker 基础镜像是 Debian bookworm（glibc），没有 musl 的动态加载器。tgz 的 sha512 integrity 写死在 `ARG` 里，构建时用 node 计算后比对；npmjs 与 npmmirror 的值一致。
- `Containerfile.gateway-worker-appserver`：`FROM claw-gateway-worker:<tag>`，`node` 取自 `node:22-bookworm-slim`。不用 bookworm 自带的 Node 18：codex-acp 依赖的 `open@11` 要求 Node ≥ 20。codex-acp 目录由 `npm ci --omit=dev --ignore-scripts` 按 `deploy/neuro-harness/codex-acp/package-lock.json` 安装（lockfile 中所有包都有 integrity；所有依赖都没有 install 脚本）。
- `deploy/neuro-harness/build-worker-images.sh <strict-worker-image> <tag>`：构建两个镜像并做冒烟检查。检查项：`neuro-*` 不带参数时退出码为 2（证明二进制能加载），`opencode --version`，`node --version`，`codex --version`。CI 和本地都走这个脚本。
- `.github/workflows/neuro-harness-worker.yaml`：push 到 `feat/neuro-harness` 时触发（只看 neuro-harness 相关路径），tag 固定为 `branch-feat-neuro-harness`。原因：`workflow_dispatch` 要求 workflow 文件已在默认分支上，否则 dispatch 返回 404，而实验分支不改 main。push 前应先对同一 commit 跑 `claw-code-branch-worker`，保证 strict 基础镜像一致。文件进入 main 后也可以手动触发并指定 `tag`。在 home-ubt 上依次编译、构建、冒烟检查，然后推送 `claw-gateway-worker-{opencode,appserver}:<tag>` 和 `:dev-<sha12>` 到 ACR。
- e2b：`build-claw-worker-{opencode,appserver}-selfhosted.py` 的实现都在 `e2b_engine_worker.py`。模板 = strict worker 模板（Dockerfile、start/ready 命令相同）+ 从引擎镜像经 registry HTTP 提取的 `/usr/local/bin` 和 `/usr/local/lib/neuro-engines`。构建完成后写入 PG 的 `e2bWorkerOpencode` / `e2bWorkerAppserver`；写入失败直接报错，不吞掉。`bootstrap-templates-from-ci-tag.sh` 在最后执行这两步：如果该 tag 没有引擎镜像，bootstrap 会在 claw 模板全部完成之后失败。

## 12. 实验分支须知

- migration `6_project_harness_engine.sql` 只部署到独立的测试 PG，不进入 pre 和 prod。合入主干时按当时的最新编号重新编号。
  - 与计划的偏差：`db_migrate.rs` 的单测 `published_migration_checksums_are_pinned` 要求每个 migration 都必须锁定 checksum，否则 `cargo test` 失败。因此分支上已经把 version 6 的 SHA-384 加入了 `PINNED_MIGRATION_CHECKSUMS`。重新编号时要同步改这一条。
- 在分支合入之前，独立测试 PG 不能直接切回主干版本：要么重建，要么手工删除这条 migration 记录（`_sqlx_migrations` 中 version=6 的行）并删除 `harness_engine` 列。
- 每次 rebase 主干后，执行 claw 引擎的现有 e2e 回归，确认 claw 的行为没有变化。
