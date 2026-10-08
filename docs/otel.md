# OTEL / OTLP export (SkyWalking or any collector)

ClawCode reports solve traces via **OTLP HTTP**. This path is independent of file/NDJSON observability (`CLAW_TRACE_*`, solve-timing Admin swimlane, etc.).

## Environment variables (repo root `.env`)

```bash
CLAW_OTEL_ENABLED=1
OTEL_EXPORTER_OTLP_ENDPOINT=http://10.22.28.239:12800
# optional:
# OTEL_EXPORTER_OTLP_HEADERS=key=value
# OTEL_SERVICE_NAME=claw-gateway-rs
CLAW_OTEL_LOG_PROMPTS=1   # default on; set 0 to omit prompt/completion on spans
```

`telemetry::otel` requires:

- `CLAW_OTEL_ENABLED` truthy
- `OTEL_EXPORTER_OTLP_ENDPOINT` non-empty (exporter posts to `{endpoint}/v1/traces` when the path is missing)

## 550w / SkyWalking OAP

柜内 NeuroGate（如 `10.22.28.240`）接入 SkyWalking：设 `OTEL_EXPORTER_OTLP_ENDPOINT` 指向 OAP REST（与 KEY 同址，当前 `http://10.22.28.239:12800`）。

```bash
CLAW_OTEL_ENABLED=1
OTEL_EXPORTER_OTLP_ENDPOINT=http://10.22.28.239:12800
```

`gateway.solve` 优先用入站 W3C `traceparent`（KEY 出站），否则用请求 `trace_id`（`X-Trace-Id` / `extra_session.trace_id`）播种同一条 OTEL 树。改 `.env` 后 `gateway.sh up`（镜像含播种逻辑后才生效）。

Worker 侧 timing 里程碑（bootstrap / turn_* / assistant_iteration_*）与 loop-outside tool 会双写 `timing.*` 子 span；in-loop `tool.execution` / `llm.chat` 走专用 span（timing NDJSON 仍写，不再重复 OTEL）。

## Process roles

| Process | `OTEL_SERVICE_NAME` | Spans |
|---------|---------------------|-------|
| `http-gateway-rs` | `claw-gateway-rs` | `gateway.solve`, `timing.bootstrap_*` |
| `claw gateway-solve-once` (worker) | `claw-worker` | `gateway_solve_turn`, `llm.chat`, `tool.execution`, `timing.*` |

Distributed trace: gateway writes W3C `traceparent` into the solve task file and `TRACEPARENT` exec env; worker continues the same trace.

## Parent / `Context::current` contract

- **Worker**：`SolveTurnOtelGuard::enter()` 后，子 span（`llm.chat` / `tool.execution` / `emit_child_span`）用 ambient `Context::current()` 合理。
- **Gateway async solve**：`OtelContextGuard` 非 `Send`，**禁止**跨 `.await` 持有 `enter()`。不要假设整个 solve 期间 current 都是 `gateway.solve`。
- 起子 span：优先 **显式** `OtelSpanGuard::start(..., Some(parent))`；仅在同步短窗口里 `enter()` → `emit_child_span` → drop（见 `solve_pool::append_bootstrap_timing_and_otel`）。
- 未 enter、又不传 parent 就靠 `Context::current()`：**不合理**（易成孤儿 span）；不因此 fail solve，靠代码审查 / 注释约束。

## Worker env forwarding

[`WORKER_ENV_KEYS`](../rust/crates/gateway-solve-turn/src/worker_env.rs) includes `CLAW_OTEL_*`、`OTEL_EXPORTER_OTLP_*` / `OTEL_SERVICE_NAME`。e2b worker exec merges [`otel_forward_env()`](../rust/crates/gateway-solve-turn/src/worker_env.rs) into guest environment. After changing `.env`, run `gateway.sh up`。

## Export failure (does not fail solve)

OTLP batch export errors are logged to **stderr** as a single-line JSON event (rate-limited ≈1/10s):

```text
{"event":"telemetry.otel.export_failed","ts_ms":…,"error":"…","dropped_spans":N,"endpoint_host":"10.22.28.239:12800"}
```

Troubleshoot:

```bash
docker logs <gateway-or-worker> 2>&1 | grep telemetry.otel.export_failed
# or emit failures:
grep telemetry.otel.emit_failed
```

## Smoke checklist (550 / SW)

1. `.env`: `CLAW_OTEL_ENABLED=1` + `OTEL_EXPORTER_OTLP_ENDPOINT=http://<oap>:12800`
2. `gateway.sh up`（或升级镜像后重启）
3. 触发一次 Admin solve
4. SkyWalking UI：同一 `trace_id` 下可见 `gateway.solve` → `gateway_solve_turn` → `llm.chat` / `tool.execution`
5. 若 SW 不可达：日志可 `grep telemetry.otel.export_failed`，业务 solve 仍应成功

本地示例（可选）:

```bash
./scripts/otel/otel-smoke.sh
# or: cargo test -p telemetry --test otel_smoke -- --ignored --nocapture
```

## Disable

Set `CLAW_OTEL_ENABLED=0` (or unset `OTEL_EXPORTER_OTLP_ENDPOINT`). No OTLP traffic; JSONL/file observability unchanged.

Author: kejiqing
