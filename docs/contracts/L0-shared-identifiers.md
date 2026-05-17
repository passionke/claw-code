# L0 — Shared identifiers

Version: **v1**  
Author: kejiqing

## Purpose

Map IDs and business context fields across AG-UI, gateway, and on-disk session layout.

## Domain model (v1)

| Concept | Canonical name | Meaning |
|---------|----------------|---------|
| **Session** | `sessionId` (AG-UI: `threadId`) | One agent window — one coherent task; multiple user turns share this id. |
| **Run** | `runId` | One user submit inside that session (each Send → new `runId`). |

Historical note: gateway async JSON still exposes `taskId` and `requestId`; both equal `sessionId` for a given solve, **not** `runId`. New code and UI copy use **`sessionId`** + **`runId`** only.

## Identifier mapping

| Concept | AG-UI (L1) | Gateway (L2/L3) | On disk | Notes |
|---------|------------|-----------------|---------|-------|
| Session | `threadId` | `sessionId` (preferred); header `claw-session-id` | `gateway_sessions.session_id` | Stable conversation / workspace key |
| Run (one turn) | `runId` | header `x-request-id` | — | Per user message; tracing and log correlation |
| Legacy session alias | — | `taskId`, `requestId` in JSON | — | Same value as `sessionId` for `/v1/solve_async`, `/v1/tasks/{id}`, `/v1/events/{id}` |
| Data source | — | `dsId` | `ds_{dsId}/sessions/...` | Required integer ≥ 1 |
| Workspace | — | `sessionHomeRel` | under `CLAW_WORK_ROOT` | Relative path for session files |

**Rules (v1):**

1. **First turn:** client sets `threadId` (new UUID or supplied). Bridge sends `claw-session-id: <threadId>`. Gateway registers `(sessionId, dsId)` in SQLite (`sessionId` = that value).
2. **Follow-up turns:** same `threadId` + same `dsId`; **new `runId`** on every user message.
3. **Do not** treat gateway `taskId` as a second session id or as `runId`.

## Business context

| Field | Where set | Limit |
|-------|-----------|-------|
| `dsId` | L2 `SolveRequest.dsId` | int ≥ 1 |
| `extraSession` | L2 JSON object | max ~8KB serialized; must be object |

Bridge copies `extraSession` from AG-UI `RunAgentInput` forwarded metadata (see L2) unchanged.

## Headers (bridge → gateway)

| Header | Value |
|--------|-------|
| `claw-session-id` | `sessionId` (= L1 `threadId`) |
| `x-request-id` | `runId` (= L1 `runId`) |

Downstream worker / LLM calls may also propagate `clawcode-session-id: <sessionId>`.

## Self-check

- Unit tests in `ag-ui-claw-bridge` assert `thread_id_to_session_id` round-trip.
- `tests/contracts-m0.sh` verifies this file exists.
