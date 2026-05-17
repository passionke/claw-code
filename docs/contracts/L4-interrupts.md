# L4 — Interrupts (permissions & AskUser)

Version: **v1**  
Author: kejiqing

## Purpose

Human-in-the-loop: tool permission and `AskUserQuestion` across UI, bridge, and gateway.

## AG-UI events (bridge → client)

| Event | Fields |
|-------|--------|
| `INTERRUPT` | `interruptId`, `reason` (`permission` \| `ask_user`), `payload` |

Example payload (permission):

```json
{
  "toolName": "bash",
  "command": "rm -rf /tmp/x",
  "options": ["allow_once", "allow_session", "deny"]
}
```

## Resolve API (client → bridge → gateway)

`POST /v1/interrupts/{interrupt_id}/resolve`

```json
{
  "decision": "allow_once",
  "answer": null
}
```

For `ask_user`, `answer` is a string.

Gateway forwards decision to worker wait channel; default timeout **120s** → `deny`.

## Bridge behavior

1. On gateway tap line `interrupt.required`, emit AG-UI `INTERRUPT` and pause tap forwarding.
2. After resolve POST succeeds, emit `INTERRUPT_RESOLVED` and resume.

## Self-check (M4)

`tests/interrupts-e2e.sh`
