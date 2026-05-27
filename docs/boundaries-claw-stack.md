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
| **HTTP gateway** | `rust/crates/http-gateway-rs/` | Axum API, solve via **host `claw-pool-daemon` + worker 容器池**, `mcpServers` merge, `dsId` registry | Doris query implementation, SQLBot server code |
| **Doris MCP** | `third_party/doris-mcp/` | Read-only SQL + metadata **only** (`mcp__doris__*`) | Gateway, SQLBot, transport bridge |
| **SQLBot (product)** | Your cluster (e.g. :8000 / :8001) | NL 问数、MCP 工具 `mcp_start` / `mcp_question`、业务库 | This repo (except optional PG/API **read** for config) |
| **Transport adapter** | Out-of-repo or custom bridge | Remote MCP (SSE/HTTP) **wire** → one stdio-shaped child for the gateway | **Not** the name “SQLBot MCP” in front of Claw; Claw sees **`mcp__sqlbot__*`** from the **merged** server config |
| **SQLBot Postgres (metadata)** | `SQLBOT_PG_*` | Encrypted datasource rows for **resolve** | Not the MCP port; not “running SQLBot” inside gateway |

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

## See also

- `docs/env-files.md` — 人手 vs 生成物路径表、禁止项
- `deploy/config/datasources.example.yaml` — `CLAW_DS_REGISTRY` 模板
- `rust/crates/http-gateway-rs/datasources.example.yaml` — 数据源 registry 模板（勿提交真实凭据）
- `third_party/doris-mcp/README.md` — Doris-only build
- `docs/http-gateway-container-pool.md` — **`http-gateway-rs`** 与 **宿主机 `claw-pool-daemon` / Docker·Podman 容器池**；网关只租借与编排
- `docs/persistence-model.md` — solve **磁盘 jsonl（运行时）** 与 **`gateway_turns` 终态（交接）** 的分工与 `turn_id` 边界
