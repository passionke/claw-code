# L2 — ag-ui-claw-bridge → http-gateway-rs

Version: **v1.1** (additive: tool stream)  
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
{"type":"tool.start","toolName":"write_file","toolCallId":"tc-1"}
{"type":"tool.result","toolCallId":"tc-1","toolName":"write_file","ok":true,"summary":"创建 pi_power.py（4 行）","payloadKind":"file_write","payload":{"type":"create","filePath":"/claw_host_root/pi_power.py","structuredPatch":[...]}}
{"type":"tool.end","toolCallId":"tc-1","ok":true}
{"type":"solve.finished","status":"succeeded"}
```

### `tool.result` envelope (v1.1)

Gateway **MUST** emit `tool.start` → `tool.result` → `tool.end` for each tool invocation (when tap is enabled).  
`text.delta` **MUST** contain only user-visible natural language — **never** raw tool JSON.

| Field | Required | Meaning |
|-------|----------|---------|
| `toolCallId` | yes | Stable id for this invocation |
| `toolName` | yes | e.g. `write_file`, `bash`, `mcp__doris__query` |
| `ok` | yes | Tool succeeded |
| `summary` | yes | One-line human text for diagnostics + history |
| `payloadKind` | yes | UI router key (see table below) |
| `payload` | yes | Tool-native JSON (opaque to bridge) |
| `error` | when `ok=false` | Short error string |

**`payloadKind` (v1.1 minimum):**

| `payloadKind` | Tool(s) | `payload` shape |
|---------------|---------|-----------------|
| `file_write` | `write_file` | `WriteFileOutput` (`type`: `create`, `filePath`, `structuredPatch`, …) |
| `file_edit` | `edit_file` | `EditFileOutput` |
| `file_read` | `read_file` | `ReadFileOutput` |
| `bash` | `bash` | `{ "command", "stdout", "stderr", "exitCode" }` |
| `generic` | other | any JSON |

Rust reference: `runtime/src/file_ops.rs` (`WriteFileOutput`, `StructuredPatchHunk`).

Bridge maps:

| Tap `type` | AG-UI / client (L1 Option A) |
|------------|------------------------------|
| `text.delta` | `TEXT_MESSAGE_CONTENT` (natural language only) |
| `tool.start` | `TOOL_CALL_START` (`toolCallId`, **`toolCallName`** — bridge reads tap `toolName` and maps to AG-UI field name) |
| `tool.result` | `TEXT_MESSAGE_CONTENT` append: fenced `` ```claw-tool `` block (full envelope JSON) |
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
