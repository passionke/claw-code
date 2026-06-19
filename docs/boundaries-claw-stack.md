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
| **HTTP gateway** | `rust/crates/http-gateway-rs/` | Axum API, solve via **host `claw-sandbox` + worker 容器池**, `mcpServers` merge, `dsId` registry | Doris query implementation, SQLBot server code |
| **Doris MCP** | `third_party/doris-mcp/` | Read-only SQL + metadata **only** (`mcp__doris__*`) | Gateway, SQLBot, transport bridge |
| **SQLBot (product)** | Your cluster (e.g. :8000 / :8001) | NL 问数、MCP 工具 `mcp_start` / `mcp_question`、业务库 | This repo (except optional PG/API **read** for config) |
| **Transport adapter** | Out-of-repo or custom bridge | Remote MCP (SSE/HTTP) **wire** → one stdio-shaped child for the gateway | **Not** the name “SQLBot MCP” in front of Claw; Claw sees **`mcp__sqlbot__*`** from the **merged** server config |
| **SQLBot Postgres (metadata)** | `SQLBOT_PG_*` | Encrypted datasource rows for **resolve** | Not the MCP port; not “running SQLBot” inside gateway |
| **OVS Web IDE** | `deploy/stack` `openvscode-server` + `extensions/claw-vscode` | Project-scoped VS Code Web UI + `@claw` Chat | Worker pool, solve_async, `/coding` ttyd UI |

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

## Where to change what

| You want to… | Edit |
| --- | --- |
| HTTP routes, timeout, inject MCP, 容器池、`SQLBOT_MCP_*`、`CLAW_DEFAULT_HTTP_MCP_*`、根 `.claw.json` | `rust/crates/http-gateway-rs/`（`main.rs`、`solve_pool.rs`、`pool/` 等） |
| Doris SQL guard / `doris_query` | `doris-mcp/src/` |
| Remote→stdio wire | Your transport bridge（本仓库默认不内置） |
| Claw tool naming / allowlist | `rust/crates/tools/` + env `CLAW_ALLOWED_TOOLS` + `project_config.allowed_tools_json` |

## Environment files (no hand-maintained “component .env”)

- **Single human-maintained deploy env:** repo root `.env` (see `.env.example`).
- **All `deploy/stack/*.env` except `.env.example`:** generated or overridden by **`./deploy/stack/gateway.sh`** / `deploy/stack/lib/*.sh` — do not edit by hand; re-run `gateway.sh up` after changing root `.env`.
- **Never create `deploy/stack/.env`** — Compose loads it implicitly and fights root `.env` / release pins (`docs/env-files.md`).

## Interactive coding terminal (CDP)

| Layer | Path | Role |
| --- | --- | --- |
| **Claw CLI display** | `rust/crates/rusty-claude-cli/src/display.rs` | `DisplaySession`: ANSI (local TTY) vs OSC **Claw Display Protocol** (`CLAW_DISPLAY_MODE=web`) |
| **Worker env** | `session_terminal_api.rs` ttyd spawn | Sets `CLAW_DISPLAY_MODE=web` for browser workers |
| **Web bundle** | `web/claw-display/` → `web/gateway-async-playground/claw-display/` | `DisplayRouter`: strip OSC → document pane (markdown) + status bar; clean bytes → xterm |
| **Playground UI** | `web/gateway-async-playground/coding.html` | Hybrid shell: document view + xterm input pane |

CDP v1 frame (embedded in ttyd stdout): `ESC ] 1337 ; Claw ; <base64url(json)> BEL` — events `content.delta`, `content.flush`, `status`, `thinking`, `turn.begin`. One WS (ttyd); no second SSE for interactive REPL.

Build (local): `CLAW_DISPLAY_LOCAL_BUILD=1 ./deploy/stack/gateway.sh claw-display-build`

## Web IDE (openvscode-server) — primary interactive entry (OVS + plugin)

| Layer | Path | Role |
| --- | --- | --- |
| **Product entry** | `/ovs?projId=N` (playground) or `:13000/ovs/` | VS Code Web IDE for `proj_N/home/`; **forward path** for `@claw` Chat (`/coding` shelved) |
| **OVS service** | `deploy/stack/podman-compose.yml` → `openvscode-server` | Central **openvscode-server**; mounts `CLAW_WORK_ROOT` at `/home/workspace` |
| **Extension** | `extensions/claw-vscode/`（`ovs-chat-demo` 仅 plumbing 参考） | `@claw` Chat + stub LM；agent WS via `/ovs/agent/ws` |
| **Agent bridge** | `session_agent_api.rs` | `GET /v1/sessions/{id}/agent/ws` → ttyd CDP；每次 `prompt` 写 `gateway_turns`（`client_origin=ovs-chat`） |
| **Project workspace** | `session_ovs_api.rs` | `GET /v1/projects/{id}/ovs/workspace` — folder contract |
| **Session registry** | `session_terminal_api.rs` | `ovs-*` 首次 `terminal_start` → `gateway_sessions`（`client_origin=ovs-chat`） |

Default OVS agent session id: `ovs-{projId}`（每 project 一个 REPL；后续 git 分支见 `docs/ovs-chat/PLAN.md`）。**OVS 交互** worker：`cwd=/claw_ds`（= `proj_N/home`）；**solve** worker：仍为 `/claw_host_root` session 工作区。

Build (local): `./deploy/stack/gateway.sh build` builds `claw-openvscode-server:local` via `Containerfile.openvscode`.

## FC cloud sandbox (interactive only)

| Layer | Path | Role |
| --- | --- | --- |
| **Backend switch** | `CLAW_INTERACTIVE_BACKEND` (`podman` \| `fc`) | Interactive only; **solve_async** stays `claw-sandbox` pool |
| **Rust FC client** | `rust/crates/claw-fc-sandbox-client/` | E2B-compatible REST (`cn-beijing`); ttyd via `deploy/fc-sandbox/fc_exec.py` |
| **Backend trait** | `pool/interactive_backend/` | `PodmanInteractiveBackend` / `FcInteractiveBackend` |
| **Terminal API** | `session_terminal_api.rs` | `terminal/start\|stop\|reattach` → `InteractiveSandboxBackend` |
| **Agent bridge** | `session_agent_api.rs` | ttyd WS via `TtydConnectTarget` (loopback or `wss://7681-sbx…`) |
| **Workspace truth** | NAS cn-beijing | `CLAW_USE_NAS_VOLUME=auto` + `NAS_BASE_URL` → compose NFS volume（Gateway/OVS 容器内直挂，无需 Mac host mount） |
| **Deploy docs** | `deploy/fc-sandbox/README.md` | Phase 0 quickstart, template, NAS, ~¥876/yr @100GB |
| **Env overlay** | `deploy/stack/env.fc-interactive.example` | FC + NAS vars for repo root `.env` |
| **E2E** | `deploy/stack/lib/verify-fc-ovs-e2e.sh` | NAS probe + `terminal/start` + optional OVS agent WS |

**OVS Chat 源码修复（另开工程）：** `docs/ovs-chat-source-handoff.md` — 调用链、证据、证伪清单、demo 成功标准。

## See also

- `docs/env-files.md` — 人手 vs 生成物路径表、禁止项
- `deploy/config/datasources.example.yaml` — `CLAW_DS_REGISTRY` 模板
- `rust/crates/http-gateway-rs/datasources.example.yaml` — 数据源 registry 模板（勿提交真实凭据）
- `third_party/doris-mcp/README.md` — Doris-only build
- `docs/http-gateway-container-pool.md` — **`http-gateway-rs`** 与 **宿主机 `claw-sandbox` / Docker·Podman 容器池**
- `sandbox/docs/system-design.md` — pool_outside 终态（单 pool、HTTP RPC）
- `docs/persistence-model.md` — solve **磁盘 jsonl（运行时）** 与 **`gateway_turns` 终态（交接）** 的分工与 `turn_id` 边界
- `docs/ovs-chat-source-handoff.md` — OVS `@demo` / `@claw` Chat 阻塞与 fork 修源码交接
- `docs/ovs-chat/EXTENSION-STABLE-DEPLOY.md` — **@claw 稳定部署契约（install/cache/settings）**
- `docs/ovs-chat/INTEGRATION.md` — OVS + claw-vscode 集成手册
- `docs/ovs-chat/PLAN.md` — OVS 路线图（git 分支 → REPL 等）
- `docs/ovs-chat-debug-log.md` — claw-code 内简版时间线
