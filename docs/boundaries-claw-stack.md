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

## Three different “SQLBot channels” (do not mix)

| Channel | Typical | Auth / use |
| --- | --- | --- |
| **SQLBot REST** | `:8000` | `X-SQLBOT-TOKEN` (user JWT) — docs, admin, **not** what `mcp__sqlbot__*` uses |
| **SQLBot MCP** | `:8001` `/mcp` | `x-ak` / `x-sk` to **reach** the MCP HTTP endpoint; inside tools, `mcp_start` uses **username/password** |
| **SQLBot PG (metadata)** | `:5432` | Gateway reads config to **build Doris connection** for `dsId` — not “executing” SQLBot chat here |

## Invariants (if you break these, you get a tangle)

1. **Doris MCP** = one stdio server, one job: guarded SQL. No SQLBot strings in its public README as a “feature” of Doris.
2. **SQLBot MCP** = what Claw names **`mcp__sqlbot__*`**; the adapter is **invisible** at the Claw tool layer.
3. **Gateway** = composes processes and env; it is **not** “Doris” and **not** “SQLBot product” — it orchestrates.
4. **Image** = convenience bundle (gateway + Doris dist + adapter script + `claw`); **repository** boundaries still split for understanding.

## Where to change what

| You want to… | Edit |
| --- | --- |
| HTTP routes, timeout, inject MCP, `SQLBOT_MCP_*` | `rust/crates/http-gateway-rs/src/main.rs`（及 `solve_pool.rs` / `pool/`） |
| Doris SQL guard / `doris_query` | `doris-mcp/src/` |
| Remote→stdio wire | Your transport bridge (not vendored in this repo by default) |
| Claw tool naming / allowlist | `rust/crates/tools/` + env `CLAW_ALLOWED_TOOLS` |

## See also

- `deploy/config/datasources.example.yaml` — `CLAW_DS_REGISTRY` 模板
- `third_party/doris-mcp/README.md` — Doris-only build
- `docs/http-gateway-container-pool.md` — **`http-gateway-rs`** 与 **宿主机 `claw-pool-daemon`** 的容器池方案
