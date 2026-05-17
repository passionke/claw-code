# Claw Web stack

CopilotKit sidebar (`claw-web-ui`) + `ag-ui-claw-bridge` + `http-gateway-rs` (same topology local and cloud).

Author: kejiqing

## Contracts

See [contracts/README.md](contracts/README.md).

## Build

```bash
cd rust
cargo build -p ag-ui-claw-bridge -p http-gateway-rs
```

## Run (development)

```bash
# Stack (gateway + bridge + tap)
./deploy/stack/gateway.sh tap-up

# Web UI (port 4100)
./deploy/stack/gateway.sh web-ui
```

Open http://127.0.0.1:4100. Chat goes `browser → /api/copilotkit → :8090 → :8088`.

Optional Phase 2 file browser:

```bash
./deploy/stack/gateway.sh code-server-up
# Set CLAW_CODE_SERVER_ENABLED=1 in .env and restart web-ui for iframe embed
```

## Self-check

1. Root `.env` merged from [deploy/stack/claw-web.env.example](../deploy/stack/claw-web.env.example) (`CLAW_GATEWAY_DEV_AGUI=1` for full curl tier).
2. `./deploy/stack/gateway.sh tap-up`
3. `./deploy/stack/gateway.sh web-ui` (separate terminal)
4. `./deploy/stack/gateway.sh verify-web-ui`  
   Or: `./tests/verify-claw-web.sh --tier ui`

Checklist: [contracts/VERIFY-CHECKLIST.md](contracts/VERIFY-CHECKLIST.md)

CI / no browser: `./tests/verify-claw-web.sh --tier smoke` or `--tier full`.

## Env reference

[deploy/stack/claw-web.env.example](../deploy/stack/claw-web.env.example)

Web UI README: [web/claw-web-ui/README.md](../web/claw-web-ui/README.md)
