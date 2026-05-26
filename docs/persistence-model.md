# Gateway persistence model (solve / turns / tasks)

Author: kejiqing

This document aligns runtime behavior with the **Claw persistence design** plan (`.cursor/plans/claw_persistence_design_*.plan.md`) while keeping a **KISS split** between disk and PostgreSQL.

## Principles

1. **Runtime source of truth: local files** — Continuation and model/tool loops use `.claw/gateway-solve-session.jsonl` under the session home (same process / same volume as today). Intermediate iterations do not need a separate DB authority.
2. **Handoff / restart source of truth: PostgreSQL** — When a user turn ends, the gateway writes a **terminal snapshot** on `gateway_turns` (`report_message`, `output_json`, `claw_exit_code`, `user_prompt`, status timestamps). After a gateway restart, **`GET /v1/tasks/{task_id}`** and formal report resolution use this row before relying on in-memory `TaskRecord`.
3. **Retry / idempotency boundary: `turn_id` (`T_<32 hex>`)** — A failed or abandoned turn is retried by issuing a **new** `turn_id` on the next solve; there is no requirement to resume half-finished model iterations from DB.

## `gateway_turns` (extended)

| Column | Role |
| --- | --- |
| `turn_id` | Primary key; one row per user solve submission. |
| `session_id`, `ds_id` | Session scope; matches `gateway_sessions`. |
| `status` | `queued` / `running` / `succeeded` / `failed` / `cancelled`. |
| `created_at_ms`, `finished_at_ms` | Ordering within a session (used with `turn_id` for stable **turn index** when slicing jsonl). |
| `user_prompt` | Optional copy of the user prompt for auditing. |
| `report_message` | Formal report body for this turn (same basis as `outputJson.message` / `report_body_from_solve_output`). |
| `output_json` | Optional full solve JSON payload for handoff. |
| `claw_exit_code` | Exit code from the worker when succeeded. |

Schema is applied at gateway startup via `GatewaySessionDb::migrate` (`ALTER TABLE ... IF NOT EXISTS` for new columns). Per-`ds_id` agent bundle storage lives in **`project_config`** (see `docs/project-config-model.md`).

## Gateway process restart

On **each** gateway binary startup, `reconcile_interrupted_turns_on_startup` sets every `gateway_turns` row still in **`queued`** or **`running`** to **`failed`**, with `output_json` explaining `restartReconciled` (process-local: no in-memory worker or pool lease survives restart). **Succeeded / failed / cancelled** rows are untouched.

This matches the rule: after restart, an “in-flight” DB row is not trustworthy as live work; clients should treat it as **interrupted / failed**, not as still runnable without a new solve.

**Multi-gateway caveat:** this `UPDATE` is global to the database. If several gateway instances share one PostgreSQL and you rely on cross-host `queued`/`running` semantics, do not use this as-is; scope reconciliation by instance id or drop the startup sweep.

## `POST /v1/tasks/{task_id}/cancel`

- **Memory hit** (async worker still tracked): same as before — abort host task, `docker_slots` / `force_kill_slot` when present, then `gateway_turns` → `cancelled` for that `turnId`.
- **Memory miss** (“cold cancel”): read **latest** `gateway_turns` for `session_id = task_id`. If that row is already **`succeeded` / `failed` / `cancelled`**, return **200** with the same **idempotent** `error` payload as the in-memory path (no DB status change). If the row is **`queued` / `running`**, write **`cancelled`** in PG only (no pool kill — there is no local handle). If there is **no** turn row for that session id, **404**.

## Formal report resolution

| 场景 | 路径 | 正文来源 |
| --- | --- | --- |
| **运行中 live** | `GET /v1/biz_advice_report?stream=true`（`queued`/`running`） | stdout hub SSE（见 [`docs/live-report-contract.md`](live-report-contract.md)） |
| **终态 JSON（默认）** | `GET /v1/tasks/{task_id}` → `result.outputJson.message` | Worker `solve.done` 落盘 |
| **终态非流式 API** | `GET /v1/biz_advice_report?stream=false`（`succeeded`） | `report_body_from_solve_output` → 内存 `TaskRecord` 或 **`gateway_turns.report_message`** |
| **重启后冷读** | `try_load_task_record` + PG | **`gateway_turns`** 行；缺省时 turn-scoped jsonl（`session_report.rs`） |
| **紧急 LLM 润色（备用）** | `GET /v1/biz_advice_report_bak` | DeepSeek polish；默认不用，代码保留 |

## Related code

- `rust/crates/http-gateway-rs/src/session_db.rs` — DDL + repositories.
- `rust/crates/http-gateway-rs/src/main.rs` — `finalize_solve_turn_*`, `try_load_task_record`, solve/async/cancel wiring.
- `rust/crates/http-gateway-rs/src/turn_stdout_hub.rs` — in-memory live report buffer.
- `rust/crates/http-gateway-rs/src/turn_stdout_live_sse.rs` — `GET /v1/biz_advice_report?stream=true` while `running`.
- `rust/crates/gateway-solve-turn/src/gateway_stdout.rs` — worker stdout `__CLAW_GATEWAY_STDOUT__` lines.
- `rust/crates/gateway-solve-turn/src/session_report.rs` — jsonl helpers including per–user-turn index.

## Live report contract (stdout-v1; Author: kejiqing)

**唯一权威文档：** [`docs/live-report-contract.md`](live-report-contract.md)（端到端流、顺序保证、2026-05-23 四条已修缺陷、部署验收、排障树）。

摘要：

| # | Rule |
| --- | --- |
| 1 | **Worker:** `TextDelta` → stdout `__CLAW_GATEWAY_STDOUT__` `report.delta`（`gateway_stdout.rs`）。 |
| 2 | **Pool daemon:** 按行解析 → FIFO 单消费者 HTTP 转发 → `POST …/stdout-event`（见契约 §7.2–§7.3）。 |
| 3 | **Gateway:** `TurnStdoutHub` → live SSE；结束哨兵 `HubMsg::SolveDone`（§7.4，防尾段截断）。 |
| 4 | **`hasReport`:** `running` \| `succeeded`。正式正文：`GET /v1/tasks` → **`result.outputJson.message`**（非顶层 `outputJson`）。 |
| 5 | **`succeeded` 后** 同 URL `stream=true` 可走 polish（`biz_advice_report_bak`）；live 与落盘对照见契约 §8。 |

## Future (not in this KISS slice)

- Versioned SQL migrations directory, `cc_messages`, dedicated `gateway_async_tasks` table, transcript HTTP API — see the design plan Phase 1–2 items; implement when multi-node SoT for every message is required.
