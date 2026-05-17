# L1 — Web client → ag-ui-claw-bridge

Version: **v1**  
Author: kejiqing

## Purpose

Define how the browser (Claw Web UI / CopilotKit, or any AG-UI client) talks to the Claw AG-UI bridge — **not** to raw gateway JSON (`8088/v1/solve`).

Primary UI: `web/claw-web-ui` via Next.js `/api/copilotkit` → `HttpAgent` → bridge.

**Optional / historical:** [OpenCode](https://github.com/sst/opencode) Web — see [deploy/stack/opencode-claw-agent.example.json](../../deploy/stack/opencode-claw-agent.example.json) (aspirational; not required for Claw Web).

## Base URL

| Env | Default |
|-----|---------|
| `CLAW_AGUI_BRIDGE_ADDR` | `0.0.0.0:8090` |
| `CLAW_AGUI_BRIDGE_URL` (Web UI server) | `http://127.0.0.1:8090` |
| Public (behind proxy) | `https://<host>/agui` |

## Endpoints (v1)

### `POST /v1/agent/run`

Starts an agent run. Response: **SSE** (`text/event-stream`).

**Request body** (AG-UI `RunAgentInput` subset):

```json
{
  "threadId": "uuid",
  "runId": "uuid",
  "messages": [{ "role": "user", "content": "..." }],
  "tools": [],
  "forwardedProps": {
    "dsId": 1,
    "extraSession": {}
  }
}
```

**Session rules (v1):**

- `threadId` → gateway `claw-session-id` header on first and follow-up turns.
- Body `forwardedProps.sessionId` only for **explicit continuation** of an existing gateway session (otherwise 400 `unknown sessionId`).

### `GET /healthz`

`200` + `{"status":"ok"}`.

## AG-UI events (client MUST handle v1)

| Event type | Meaning |
|------------|---------|
| `RUN_STARTED` | Run accepted |
| `TEXT_MESSAGE_START` | Assistant message begins |
| `TEXT_MESSAGE_CONTENT` | Token delta (`delta` field) |
| `TEXT_MESSAGE_END` | Message complete |
| `TOOL_CALL_START` | Tool invocation (optional v1) |
| `TOOL_CALL_END` | Tool result (optional v1) |
| `RUN_FINISHED` | Success |
| `RUN_ERROR` | Failure (`message`) |

Interrupt events: see [L4-interrupts.md](L4-interrupts.md).

## Errors

| HTTP | When |
|------|------|
| 400 | Invalid JSON, missing `dsId` in forwardedProps |
| 502 | Gateway unreachable |
| 504 | Solve timeout |

Client SHOULD reconnect only after `RUN_ERROR` or connection drop; new user message = new `runId`.

## Deployment parameters

| Parameter | Local | Cloud |
|-----------|-------|-------|
| Bridge port | 8090 | 8090 (internal) |
| Claw Web UI | 4100 (`CLAW_WEB_UI_PORT`) | 443 via proxy |
| Gateway | host `8088` → container `8080` | internal only |

## Self-check (M1)

```bash
cargo test -p ag-ui-claw-bridge
curl -sS -N -X POST http://127.0.0.1:8090/v1/agent/run \
  -H 'Content-Type: application/json' \
  -d '{"threadId":"t1","runId":"r1","messages":[{"role":"user","content":"hi"}],"tools":[],"forwardedProps":{"dsId":1}}'
```

With `CLAW_AGUI_MOCK=1`, expect SSE containing `RUN_STARTED` and `RUN_FINISHED`.
