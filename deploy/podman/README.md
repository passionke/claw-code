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

## 2) Configure env (repo root only)

```bash
cp .env.example .env
```

Edit **repository root** `.env`:

- `GATEWAY_IMAGE`, `GATEWAY_HOST_PORT`
- `OPENAI_API_KEY`, `OPENAI_BASE_URL` (e.g. claude-tap: `http://host.containers.internal:8080/v1`)
- `INTERNAL_CLAUDE_TAP_HOST` (for docs/scripts; gateway does not inject `OPENAI_BASE_URL` automatically)
- `CLAW_HOST_LOG_DIR` (default `./deploy/podman/claw-logs` — create the directory if missing)
- optional `CLAW_LOG_LEVEL`, `CLAW_TRACE_ENABLED`
- for `start-with-tap.sh`: `UPSTREAM_OPENAI_BASE_URL`, `CLAUDE_TAP_PORT`, `CLAUDE_TAP_LIVE_PORT`

Mount **repository root** `.claw.json` at `/app/.claw.json` (see `podman-compose.yml`). The gateway applies `.claw.json` `env` (when unset) and `model` (after `CLAW_DEFAULT_MODEL`) on each solve.

`podman-compose.yml` also loads `deploy/podman/gateway-allowlist.env` after the root `.env` so `CLAW_ALLOWED_TOOLS` can override an empty `CLAW_ALLOWED_TOOLS=` in the root file (Compose `environment:` cannot fix that). Edit that file to test per-request `allowedTools` against a global allowlist.

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
