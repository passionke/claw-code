# Langfuse OTEL export (self-hosted)

ClawCode reports solve traces to a self-hosted Langfuse instance via **OTLP HTTP**. This path is independent of file/NDJSON observability (`CLAW_TRACE_*`, solve-timing, etc.).

## Environment variables (repo root `.env`)

```bash
CLAW_OTEL_ENABLED=1
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
LANGFUSE_BASE_URL=http://your-langfuse-host:8090
CLAW_OTEL_LOG_PROMPTS=1   # default on; set 0 to omit prompt/completion on spans
```

`telemetry::otel` derives:

- Base URL: `{LANGFUSE_BASE_URL}/api/public/otel` (exporter posts to `…/v1/traces`)
- Auth: `Authorization: Basic base64("{LANGFUSE_PUBLIC_KEY}:{LANGFUSE_SECRET_KEY}")`

Optional overrides (Collector / advanced):

- `OTEL_EXPORTER_OTLP_ENDPOINT`
- `OTEL_EXPORTER_OTLP_HEADERS`

## Process roles

| Process | `OTEL_SERVICE_NAME` | Spans |
|---------|---------------------|-------|
| `http-gateway-rs` | `claw-gateway-rs` | `gateway.solve` |
| `claw-pool-daemon` | `claw-pool-daemon` | `pool.exec_solve` |
| `claw gateway-solve-once` (worker) | `claw-worker` | `gateway_solve_turn`, `llm.chat`, `tool.execution` |

Distributed trace: gateway writes W3C `traceparent` into the solve task file and `TRACEPARENT` exec env; worker continues the same trace.

## Worker env forwarding

[`WORKER_ENV_KEYS`](rust/crates/gateway-solve-turn/src/worker_env.rs) includes `CLAW_OTEL_*` and `LANGFUSE_*`. Pool `docker exec` also merges [`otel_forward_env()`](rust/crates/gateway-solve-turn/src/worker_env.rs) into worker environment.

Host `claw-sandbox` reads the same keys from `deploy/stack/.claw-pool-rpc/pool-daemon.env` (written by `gateway.sh pool-up` from repo `.env`). After changing `.env`, run `gateway.sh pool-up --restart`.

## Langfuse requirements

- Langfuse **>= v3.22.0** with OTLP endpoint enabled
- OTLP over **HTTP** (protobuf or JSON); gRPC is not supported by Langfuse

## Disable

Set `CLAW_OTEL_ENABLED=0` (or unset `LANGFUSE_*` keys). No OTLP traffic; JSONL/file observability unchanged.

Author: kejiqing
