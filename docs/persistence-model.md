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
| `worker_report_host` | Gateway-reachable host for live report SSE proxy (container IP, name, or published host). |
| `worker_report_port` | TCP port paired with `worker_report_host` (in-container 18765 or published host port). |

Schema is applied at gateway startup via `GatewaySessionDb::migrate` (`ALTER TABLE ... IF NOT EXISTS` for new columns). Per-`ds_id` agent bundle storage lives in **`project_config`** (see `docs/project-config-model.md`).

## Gateway process restart

On **each** gateway binary startup, `reconcile_interrupted_turns_on_startup` sets every `gateway_turns` row still in **`queued`** or **`running`** to **`failed`**, with `output_json` explaining `restartReconciled` (process-local: no in-memory worker or pool lease survives restart). **Succeeded / failed / cancelled** rows are untouched.

This matches the rule: after restart, an “in-flight” DB row is not trustworthy as live work; clients should treat it as **interrupted / failed**, not as still runnable without a new solve.

**Multi-gateway caveat:** this `UPDATE` is global to the database. If several gateway instances share one PostgreSQL and you rely on cross-host `queued`/`running` semantics, do not use this as-is; scope reconciliation by instance id or drop the startup sweep.

## `POST /v1/tasks/{task_id}/cancel`

- **Memory hit** (async worker still tracked): same as before — abort host task, `docker_slots` / `force_kill_slot` when present, then `gateway_turns` → `cancelled` for that `turnId`.
- **Memory miss** (“cold cancel”): read **latest** `gateway_turns` for `session_id = task_id`. If that row is already **`succeeded` / `failed` / `cancelled`**, return **200** with the same **idempotent** `error` payload as the in-memory path (no DB status change). If the row is **`queued` / `running`**, write **`cancelled`** in PG only (no pool kill — there is no local handle). If there is **no** turn row for that session id, **404**.

## Formal report resolution order

`GET /v1/biz_advice_report` (non–live-spill path) uses `resolve_formal_report_text`:

1. In-process **`TaskRecord`** for `(sessionId == taskId)` when `turnId` matches — fast path.
2. **`gateway_turns.report_message`** for that `turnId` — survives restart.
3. **Turn-scoped jsonl** — `final_assistant_report_text_from_jsonl_for_user_turn_index` using the turn’s ordinal among `gateway_turns` rows (avoids concatenating every assistant block in a multi-turn file when DB snapshot is missing).
4. **Legacy** — full-session `final_assistant_report_text_from_jsonl` (last resort).

## Related code

- `rust/crates/http-gateway-rs/src/session_db.rs` — DDL + repositories.
- `rust/crates/http-gateway-rs/src/main.rs` — `finalize_solve_turn_*`, `try_load_task_record`, solve/async/cancel wiring.
- `rust/crates/http-gateway-rs/src/biz_advice_report_live.rs` — `resolve_formal_report_text`.
- `rust/crates/gateway-solve-turn/src/session_report.rs` — jsonl helpers including per–user-turn index.

## Live report contract (locked; Author: kejiqing)

Product/BFF/admin **must** follow this; do not substitute alternate gates (e.g. open SSE on `running` without `hasReport`, client `frameSeq` / `afterSeq`, or `start.snapshotText`).

| # | Rule |
| --- | --- |
| 1 | **`hasReport`** means the gateway has received at least one byte on `POST /v1/internal/turns/{turnId}/report-stream` for this turn (in-memory relay), **or** legacy PG live chunks exist, **or** turn `succeeded`. Frontend opens report SSE **only after** `GET /v1/tasks` returns `hasReport: true`. |
| 2 | **Worker:** model `TextDelta` → in-container HTTP on fixed port **`CLAW_WORKER_REPORT_SSE_PORT` (default 18765)** — `GET /v1/turns/{turnId}/report` (SSE) and `GET …/report/status` (`hasReport`). Text coalesced (≥48 chars or 80ms) before `biz.report.delta`. |
| 3 | **Gateway:** on pool lease, daemon **`podman run -p 0.0.0.0:{publish_port}:18765`** and persists **dial** `worker_report_host` / `worker_report_port` (`CLAW_POOL_WORKER_REPORT_ADVERTISE_HOST` + `CLAW_*_WORKER_REPORT_PUBLISH_BASE + slot`) on `gateway_turns`. Container IP is logged for ops only. Any gateway can proxy live SSE. Cleared on turn terminal / startup reconcile. |
| 4 | **SSE to client:** standard events only — `biz.report.start`, `biz.report.delta` `{"text":"…"}`, `biz.report.done`. Client does **not** use `afterSeq`; reconnect opens a new GET; worker replays its in-memory coalesced-delta buffer from the start, then tails new deltas. |

Legacy `POST …/assistant-stream` (NDJSON → `gateway_turn_live_chunks`) remains for old workers; new pool workers use `report-stream` only.

## `gateway_turn_live_chunks` (live report tail, v1)

| Column | Role |
| --- | --- |
| `turn_id`, `seq` | Primary key; monotonic chunk sequence per turn (strong ordering). |
| `chunk` | Opaque UTF-8 text fragment exactly as worker sent in that NDJSON line (**not** merged into paragraphs in SQL). |
| `created_at_ms` | Insert time (per-line ingest). |

- **Ingest:** pool worker streams NDJSON to the gateway; gateway **per non-empty line** `INSERT` + one `NOTIFY claw_turn_live` per transaction (`maxSeq` in payload; `terminal` on turn end).
- **`hasReport`:** `running` → `EXISTS` live row for `turn_id`; `succeeded` → always `true` (`task_has_report` in `http-gateway-rs`).
- **Live SSE:** subscribe → bootstrap `seq > 0` rows in order → loop `SELECT seq > last_emitted_seq ORDER BY seq ASC` → map each row’s `chunk` to `biz.report.delta` (+ optional 128-scalar frame split); on `succeeded` + formal `report_message` → `biz.report.done` (skip redundant formal flush when live body already matches length; see `should_skip_formal_flush_after_live_pg`).
- **Cleanup:** `succeeded` → `terminal NOTIFY` then optional `DELETE` live rows (`CLAW_GATEWAY_DELETE_LIVE_CHUNKS_ON_SUCCESS`); failed/cancelled/orphans → [`scripts/purge-gateway-turn-live-chunks.sh`](../scripts/purge-gateway-turn-live-chunks.sh).

**Reconstructing full text:** `string_agg(chunk, '' ORDER BY seq)` equals the streamed assistant body for that turn (modulo marker strip in sanitizer on export, not in stored `chunk`).

**Report SSE timing (debug):** set `CLAW_REPORT_SSE_TIMING=1` (or `CLAW_SSE_DEBUG=1`) on **gateway and pool worker** containers. Logs use target `claw_report_sse_timing` with phases `trunk_in` → `hub_push` (`trunk_to_hub_ms`) → `sse_emit` (`hub_to_sse_ms`, `trunk_to_sse_ms`) on the worker, and `gateway_proxy_connect` / `gateway_proxy_first_byte` / `gateway_proxy_chunk` on the gateway proxy. Example:

```bash
rg 'claw_report_sse_timing|T_555814ed' deploy/stack/claw-logs/
```

**Live report audit (debug):** set `CLAW_LIVE_SSE_EMIT_TRACE=1` or `CLAW_SSE_DEBUG=1` on the gateway, then:

```bash
podman logs -f claw-gateway-rs 2>&1 | rg 'live (chunk PG notify|report SSE)'
```

| `phase` | Meaning |
| --- | --- |
| `pg_notify_sent` | After PG `INSERT` commit, `pg_notify` fired (`sent_at_ms`, `max_seq`) |
| `pg_notify_received` | `LISTEN claw_turn_live` got payload (`received_at_ms`, `notify_max_seq`) |
| `sse_loop_wake` | SSE worker iteration start (`wake_reason`: `after_bootstrap` / `pg_notify` / `poll_timer_2s`) |
| `sse_bootstrap_query` / `sse_tail_query` | `SELECT seq > …` (`query_start_ms`, `query_done_ms`, `query_elapsed_ms`) |
| `sse_*_emit` | Per-row SSE send (`seq`, `pg_created_at_ms`, `sse_emitted_at_ms`, `lag_ms`) |

## Future (not in this KISS slice)

- Versioned SQL migrations directory, `cc_messages`, dedicated `gateway_async_tasks` table, transcript HTTP API — see the design plan Phase 1–2 items; implement when multi-node SoT for every message is required.
