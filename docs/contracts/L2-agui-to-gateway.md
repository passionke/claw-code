# L2 — ag-ui-claw-bridge → http-gateway-rs

Version: **v1**  
Author: kejiqing

## Purpose

Bridge orchestrates gateway solve APIs and translates gateway progress into AG-UI events.

## Gateway APIs used (v1)

| Method | Path | Use |
|--------|------|-----|
| POST | `/v1/solve_async` | Start solve; returns `sessionId` (`taskId` in JSON = same value, see [L0](L0-shared-identifiers.md)) |
| GET | `/v1/tasks/{task_id}` | Poll until terminal status; `?dsId=` returns `status: idle` when session exists but no in-memory task |
| GET | `/v1/sessions/{session_id}/execution?ds_id=` | Progress + optional trace |
| GET | `/v1/events/{task_id}` | **Event tap** (NDJSON stream, v1 extension) |
| POST | `/v1/interrupts/{interrupt_id}/resolve` | Resume after human input (L4) |
| POST | `/v1/dev/agui/seed-task` | **Dev only** (`CLAW_GATEWAY_DEV_AGUI=1`): in-memory task + tap lines |
| POST | `/v1/dev/agui/seed-interrupt/{task_id}` | **Dev only**: register interrupt + tap line |

Full field semantics: [http-gateway-rs-api.md](../http-gateway-rs-api.md).

## Bridge → gateway: start solve

Maps L1 `RunAgentInput` to `SolveRequest`:

```json
{
  "dsId": 1,
  "userPrompt": "<last user message text>",
  "sessionId": "<threadId when continuing>",
  "timeoutSeconds": 600,
  "extraSession": {}
}
```

Headers:

- `claw-session-id: <threadId>` → gateway `sessionId`
- `x-request-id: <runId>` → one turn per user send (not gateway `taskId`)

## Event tap (NDJSON)

`GET /v1/events/{task_id}` returns `application/x-ndjson` lines:

```json
{"type":"solve.queued","taskId":"...","tsMs":0}
{"type":"text.delta","text":"hello"}
{"type":"tool.start","toolName":"bash","toolCallId":"..."}
{"type":"tool.end","toolCallId":"...","ok":true}
{"type":"solve.finished","status":"succeeded"}
```

Bridge maps:

| Tap `type` | AG-UI event |
|------------|-------------|
| `text.delta` | `TEXT_MESSAGE_CONTENT` |
| `tool.start` | `TOOL_CALL_START` |
| `tool.end` | `TOOL_CALL_END` |
| `solve.finished` | `RUN_FINISHED` |
| `solve.failed` | `RUN_ERROR` |

Polling fallback: if tap unavailable, bridge polls `/v1/tasks/{id}` and emits coarse progress only (not preferred).

## Environment

| Variable | Default |
|----------|---------|
| `CLAW_GATEWAY_BASE_URL` | `http://127.0.0.1:8080` |

## Self-check (M2)

`tests/http-gateway-agui-bridge.sh`
