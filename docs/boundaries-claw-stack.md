# Claw / gateway / Doris / SQLBot — system boundaries

This file is the **single place** for long-term mental model. If something “works but nobody can explain it,” these lines were ignored.

Author: kejiqing

## Why this exists

- **No boundary** → everything looks like one ball of yarn: “doris-mcp 里怎么又有 SQLBot？” / “桥是不是 SQLBot？”
- **Clear boundary** → each change has an obvious home; reviews stay small; new people read one table.

## What `claw` (Rust runtime) is allowed to assume

- It only knows **MCP clients**: stdio processes registered as `mcpServers` with tool names `mcp__<server>__<tool>`.
- It does **not** know whether a server is “really” remote SSE, local Node, or Podman. That is **parent process** (gateway) concern.

## Components (who owns what)

| Component | Code / deploy | Owns | Does **not** own |
| --- | --- | --- | --- |
| **Claw** | `rust/` | Tool surface, `mcp__*` allowlist, session | HTTP, datasource encryption, SQLBot product |
| **HTTP gateway** | `rust/crates/http-gateway-rs/` | Axum API, solve via **FC / e2b worker**, `mcpServers` merge, `dsId` registry | Doris query implementation, SQLBot server code |
| **Doris MCP** | `third_party/doris-mcp/` | Read-only SQL + metadata **only** (`mcp__doris__*`) | Gateway, SQLBot, transport bridge |
| **SQLBot (product)** | Your cluster (e.g. :8000 / :8001) | NL 问数、MCP 工具 `mcp_start` / `mcp_question`、业务库 | This repo (except optional PG/API **read** for config) |
| **Transport adapter** | Out-of-repo or custom bridge | Remote MCP (SSE/HTTP) **wire** → one stdio-shaped child for the gateway | **Not** the name “SQLBot MCP” in front of Claw; Claw sees **`mcp__sqlbot__*`** from the **merged** server config |
| **SQLBot Postgres (metadata)** | `SQLBOT_PG_*` | Encrypted datasource rows for **resolve** | Not the MCP port; not “running SQLBot” inside gateway |
| **OVS Web IDE** | `deploy/stack` `openvscode-server` + `extensions/claw-vscode` | Project-scoped VS Code Web UI + `@claw` Chat | Worker pool, solve_async |

## Two SQLBot product channels (do not mix)

| Channel | Typical | Auth / use |
| --- | --- | --- |
| **SQLBot REST** | `:8000` | `X-SQLBOT-TOKEN` (user JWT) — docs, admin, **not** what `mcp__sqlbot__*` uses |
| **SQLBot MCP** | `:8001` `/mcp` (or streamable path) | `x-ak` / `x-sk` to **reach** the MCP HTTP endpoint; inside tools, `mcp_start` uses **username/password** |

`http-gateway-rs` **does not** read SQLBot Postgres for `dsId` → Doris wiring. Optional `dsId` checks use **`CLAW_DS_REGISTRY`** (YAML) only; see `rust/crates/http-gateway-rs/datasources.example.yaml`.

## Invariants (if you break these, you get a tangle)

1. **Doris MCP** = one stdio server, one job: guarded SQL. No SQLBot strings in its public README as a “feature” of Doris.
2. **SQLBot MCP** = what Claw names **`mcp__sqlbot__*`**; the adapter is **invisible** at the Claw tool layer.
3. **Gateway** = composes processes and env; it is **not** “Doris” and **not** “SQLBot product” — it orchestrates.
4. **Image** = convenience bundle (gateway + Doris dist + adapter script + `claw`); **repository** boundaries still split for understanding.
5. **`CLAW_MCP_MAX_CONCURRENT`**: max in-flight MCP `tools/call` per worker; values `> 1` also enable same-turn parallel SQLBot fan-out (`[parallel-friendly]` tool hint + `shared_executor`). Set `1` for fully serial MCP (`rust/crates/runtime/src/mcp_client.rs`).
6. **Solve preflight (per `ds_*`)**: `ds_<id>/home/.claw/solve-preflight.json` with ordered `kinds` (e.g. `["sqlbot_mcp_start"]`, compatible with legacy `kind`) → **first** `sessionId` turn only, after user text in jsonl, code-run preflight (`rust/crates/gateway-solve-turn/src/project_preflight.rs`). Table DDL: `ds_<id>/home/schema.md`, ro mount + system prompt (`GATEWAY_SCHEMA_MD_REL`).
7. **claude-tap (LLM proxy)**: sidecar 或远程共享 tap；Gateway 注入 per-solve `OPENAI_BASE_URL`。Traces: `CLAW_TAP_TRACES_DIR` 或 NAS `tap-traces/`。
8. **FC NAS workspace**: one logical NFS export root; Gateway mkdir/symlink on container `CLAW_WORK_ROOT`; e2b bind via `hostMountRoot` + relPath. **Do not** mix host path strings with container bind points — see **`docs/e2b-nas-workspace.md`**.
9. **对外 HTTP API 明确类型（少泛型）**：Gateway 对外请求/响应 DTO 与 OpenAPI（`utoipa` `ToSchema` / `#[utoipa::path]`）必须用**明确结构类型与闭环枚举**表达契约。禁止为了省事把已有 Rust 结构压成 `Object`、`additionalProperties: true`，或用自由 `String` 冒充枚举。真正无固定形状的开放 JSON bag（如 `project_config` 的 `*Json`、工具 `input`、jsonl `trace_tail`）可保留 `Object`，但须在字段注释或 API 文档中写明「为何不能结构化」。OpenAPI **唯一来源**是 handler 注解 + DTO 类型；禁止维护第二份手写 schema。实践清单见 **`docs/http-gateway-rs-docs-practice.md`**。

## Where to change what

| You want to… | Edit |
| --- | --- |
| e2b NAS layout, env, Mac podman / e2b bind | **`docs/e2b-nas-workspace.md`** + repo root `.env` (`CLAW_NAS_*`, `CLAW_E2B_*`) |
| HTTP routes, timeout, inject MCP, OpenAPI DTO、`SQLBOT_MCP_*`、`CLAW_DEFAULT_HTTP_MCP_*`、根 `.claw.json` | `rust/crates/http-gateway-rs/`（`routes/`、`openapi.rs`、`solve_pool.rs`、`pool/` 等） |
| Doris SQL guard / `doris_query` | `doris-mcp/src/` |
| Remote→stdio wire | Your transport bridge（本仓库默认不内置） |
| Claw tool naming / allowlist | `rust/crates/tools/` + env `CLAW_ALLOWED_TOOLS` + `project_config.allowed_tools_json` |

## Environment files (no hand-maintained “component .env”)

- **Single human-maintained deploy env:** repo root `.env` (see `.env.example`).
- **All `deploy/stack/*.env` except `.env.example`:** generated or overridden by **`./deploy/stack/gateway.sh`** / `deploy/stack/lib/*.sh` — do not edit by hand; re-run `gateway.sh up` after changing root `.env`.
- **Never create `deploy/stack/.env`** — Compose loads it implicitly and fights root `.env` / release pins (`docs/env-files.md`).

## Interactive / OVS / web CLI（已退出）

OVS `@claw`、web CLI（CDP / `agent/ws` / `gateway-interactive-once`）已全面退出，**不是**现行边界。历史取证：`docs/ovs-chat/`、`docs/ovs-chat-source-handoff.md`。

## e2b cloud sandbox（solve）

| Layer | Path | Role |
| --- | --- | --- |
| **Rust e2b client** | `rust/crates/claw-e2b-sandbox-client/` | E2B-compatible REST；guest exec via `deploy/e2b/e2b_exec.py` |
| **Worker pool** | `pool/e2b_proj_worker_registry.rs` + `e2b_orchestrated_pool.rs` | per-project warm workers；`acquire_for_solve` |
| **NAS / Observe helpers** | `pool/interactive_backend/`（历史命名） | `E2bNasApiSingleton`、worker tap、`e2b_observe_is_enabled` |
| **Workspace materialize** | `session_terminal_api.rs`（仅留 materialize） | `proj_N/home` + `claw.projId` settings |
| **Deploy docs** | `deploy/e2b/README.md` | template、NAS、worker 构建 |

## See also

- `docs/env-files.md` — 人手 vs 生成物路径表、禁止项
- `deploy/config/datasources.example.yaml` — `CLAW_DS_REGISTRY` 模板
- `rust/crates/http-gateway-rs/datasources.example.yaml` — 数据源 registry 模板（勿提交真实凭据）
- `third_party/doris-mcp/README.md` — Doris-only build
- `docs/http-gateway-container-pool.md` — **e2b worker 编排**（e2b solve）
- `docs/architecture-governance.md` — 目标拓扑与迁移
- `docs/persistence-model.md` — solve **磁盘 jsonl（运行时）** 与 **`gateway_turns` 终态（交接）** 的分工与 `turn_id` 边界
- `docs/ovs-chat-source-handoff.md` — OVS `@demo` / `@claw` Chat 阻塞与 fork 修源码交接
- `docs/ovs-chat/EXTENSION-STABLE-DEPLOY.md` — **@claw 稳定部署契约（install/cache/settings）**
- `docs/ovs-chat/INTEGRATION.md` — OVS + claw-vscode 集成手册
- `docs/ovs-chat/PLAN.md` — OVS 路线图（git 分支 → REPL 等）
- `docs/ovs-chat-debug-log.md` — claw-code 内简版时间线
