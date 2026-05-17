# Claw Web — layer contracts

Inter-layer API contracts for **Claw Web UI → AG-UI bridge → http-gateway-rs → worker/claw**.

Author: kejiqing

## Documents

| File | Boundary | Version |
|------|----------|---------|
| [L0-shared-identifiers.md](L0-shared-identifiers.md) | IDs across all layers | v1 |
| [L1-opencode-to-agui.md](L1-opencode-to-agui.md) | Web client → `ag-ui-claw-bridge` | v1 |
| [L2-agui-to-gateway.md](L2-agui-to-gateway.md) | bridge → `http-gateway-rs` | v1 |
| [L3-gateway-to-worker.md](L3-gateway-to-worker.md) | gateway → container worker | v1 |
| [L4-interrupts.md](L4-interrupts.md) | Human-in-the-loop | v1 |
| [L5-auth-audit.md](L5-auth-audit.md) | JWT / tenant / audit (optional) | v1 |

Component ownership (Doris, SQLBot, MCP) remains in [boundaries-claw-stack.md](../boundaries-claw-stack.md).

Gateway route details: [http-gateway-rs-api.md](../http-gateway-rs-api.md).

## Version policy

- Contract version is `v1` until a breaking change; bump minor for additive fields, major for breaking.
- PRs **update the contract first**, then implementation. Note `v1.x → v1.y` in the PR description.

## Who edits what

| Change | Edit |
|--------|------|
| AG-UI SSE URL, event subset | L1 |
| bridge ↔ gateway JSON, event tap | L2 |
| solve worker, jsonl paths | L3 (+ gateway API doc if public HTTP) |
| permission / AskUser interrupts | L4 |
| Bearer JWT, audit query | L5 |

## Self-check commands (by milestone)

**部署后一键验证**（推荐）：`./tests/verify-claw-web.sh --tier all`  
或 gateway 起栈后：`./deploy/stack/gateway.sh verify-web`  
功能对照表：[VERIFY-CHECKLIST.md](VERIFY-CHECKLIST.md)

| Milestone | Command |
|-----------|---------|
| 全部 | `tests/verify-claw-web.sh --tier all` |
| M0 | `tests/contracts-m0.sh` |
| M1 | `cargo test -p ag-ui-claw-bridge` |
| M2 | `tests/http-gateway-agui-bridge.sh` |
| M3 | `tests/e2e-claw-web-stack.sh` (legacy smoke) |
| M4 | `tests/interrupts-e2e.sh` (legacy) |
| M5 | `cargo test -p http-gateway-rs auth_audit` |

## Default topology

Local and cloud use the **same** chain:

`Browser → claw-web-ui (CopilotKit) → ag-ui-claw-bridge → http-gateway-rs → worker pool → claw runtime`

Differences are **configuration only** (ports, pool size, TLS, JWT). See L1 deployment table.
