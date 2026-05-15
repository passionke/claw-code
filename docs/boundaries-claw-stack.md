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
| **HTTP gateway** | `rust/crates/http-gateway-rs/`（`http-gateway-rs` 二进制） | Axum HTTP、`claw` 编排、solve 会话 SQLite、`mcpServers` 合并、`dsId` 工作区、MCP 注入、按 ds 读技能等 | Doris 查询实现、SQLBot 产品服务本体 |
| **Doris MCP** | `third_party/doris-mcp/` | Read-only SQL + metadata **only** (`mcp__doris__*`) | Gateway, SQLBot, transport bridge |
| **SQLBot (product)** | Your cluster (e.g. :8000 / :8001) | NL 问数、MCP 工具 `mcp_start` / `mcp_question`、业务库 | This repo |

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

## Where to change what

| You want to… | Edit |
| --- | --- |
| HTTP routes, timeout, inject MCP, 容器池、`CLAW_DEFAULT_HTTP_MCP_*`、根 `.claw.json` | `rust/crates/http-gateway-rs/` |
| Doris SQL guard / `doris_query` | `doris-mcp/src/` |
| Claw tool naming / allowlist | `rust/crates/tools/` + env `CLAW_ALLOWED_TOOLS` |

## See also

- `rust/crates/http-gateway-rs/datasources.example.yaml` — 数据源 registry 模板（勿提交真实凭据）
- `third_party/doris-mcp/README.md` — Doris-only build
- `docs/http-gateway-container-pool.md` — **`http-gateway-rs`** 用 **Docker/Podman 容器池**隔离 solve：**PoolManager** 启动读 env 管池大小与预热；网关只租借与编排
