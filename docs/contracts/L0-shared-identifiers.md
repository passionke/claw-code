# L0 — Shared identifiers

Version: **v1**  
Author: kejiqing

## Purpose

Map IDs and business context fields across AG-UI, gateway, and on-disk session layout.

## Identifier mapping

| AG-UI (L1) | Gateway (L2/L3) | On disk | Notes |
|------------|-----------------|---------|-------|
| `threadId` | `sessionId` (preferred) | `gateway_sessions.session_id` | Stable conversation key |
| `runId` | `taskId` | Same value as `sessionId` for async solve | One run per async submit |
| — | `requestId` | — | Legacy alias of `sessionId` in gateway JSON |
| — | `dsId` | `ds_{dsId}/sessions/...` | Required integer ≥ 1 |
| — | `sessionHomeRel` | under `CLAW_WORK_ROOT` | Relative workspace path |

**Rule (v1):** On first turn, bridge sends `threadId` = new UUID (or client-supplied). Gateway stores `(sessionId, dsId)` in SQLite. Continuation: same `threadId` + same `dsId`.

## Business context

| Field | Where set | Limit |
|-------|-----------|-------|
| `dsId` | L2 `SolveRequest.dsId` | int ≥ 1 |
| `extraSession` | L2 JSON object | max ~8KB serialized; must be object |

Bridge copies `extraSession` from AG-UI `RunAgentInput` forwarded metadata (see L2) unchanged.

## Headers (gateway upstream)

When gateway calls LLM/MCP, it may set:

- `claw-session-id: <sessionId>`
- `clawcode-session-id: <sessionId>`

## Self-check

- Unit tests in `ag-ui-claw-bridge` assert `thread_id_to_session_id` round-trip.
- `tests/contracts-m0.sh` verifies this file exists.
