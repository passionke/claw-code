# Podman Deployment (Gateway + Claude-Tap Proxy)

中文快速上手请看：`docs/http-gateway-rs-quickstart.md`

This deployment runs two processes:
- `gateway-rs` in podman container
- `claude-tap` on host as API proxy/trace viewer

`claude-tap` is not an MCP server. It proxies model API traffic and records traces.

## 1) Build gateway image

```bash
./deploy/podman/build.sh
```

Optional: build with a specific tag (for release deployment):

```bash
./deploy/podman/build.sh release-v1.0.8
```

## 2) Configure env

```bash
cp deploy/podman/.env.example deploy/podman/.env
```

Set in `deploy/podman/.env`:
- `GATEWAY_IMAGE`
- `GATEWAY_HOST_PORT`
- `INTERNAL_CLAUDE_TAP_HOST` (usually `http://host.containers.internal:8080`)
- `CLAW_HOST_LOG_DIR` (host bind mount for `/var/log/claw`: JSON stdout from `podman logs` is separate; trace `.ndjson` and `sse-debug.log` persist here across container recreation)
- optional `CLAW_LOG_LEVEL` (default `info`), `CLAW_TRACE_ENABLED` (default `1`)

If `CLAW_HOST_LOG_DIR` points at a path that does not exist yet, create it on the host (default in `.env.example` is `./claw-logs` under `deploy/podman/`).

Set in root `.env`:
- `OPENAI_API_KEY`
- `UPSTREAM_OPENAI_BASE_URL` (or existing `OPENAI_BASE_URL`, e.g. `https://api.deepseek.com`)
- optional `CLAUDE_TAP_PORT` (default 8080)
- optional `CLAUDE_TAP_LIVE_PORT` (default 3000)

## 3) Start (gateway + claude-tap)

```bash
./deploy/podman/start-with-tap.sh
```

## 4) Verify connectivity chain

```bash
./deploy/podman/check-connectivity.sh
```

This script validates:
- gateway `/healthz`
- async solve path
- default MCP wiring (`CLAW_DEFAULT_HTTP_MCP_NAME`) inside gateway

## 5) Stop

```bash
./deploy/podman/stop-with-tap.sh
```
