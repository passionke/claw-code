# Claw Web UI

CopilotKit sidebar → Next.js `/api/copilotkit` → AG-UI bridge (`8090`) → gateway (`8088`).

Author: kejiqing

## Quick start

```bash
# From repo root — stack must be up (tap-up or up + bridge)
./deploy/stack/gateway.sh tap-up
./deploy/stack/gateway.sh web-ui
```

Open http://127.0.0.1:4100 (default `CLAW_WEB_UI_PORT`).

## Env

Copy `.env.local.example` to `.env.local` or set via `gateway.sh web-ui`:

| Variable | Default |
|----------|---------|
| `CLAW_AGUI_BRIDGE_URL` | `http://127.0.0.1:8090` |
| `CLAW_GATEWAY_BASE_URL` | `http://127.0.0.1:8088` |
| `CLAW_WEB_UI_PORT` | `4100` |
| `CLAW_WEB_DATABASE_URL` | `postgresql://claw:claw@127.0.0.1:5433/claw_web` (needs `gateway.sh pg-up`) |
| `CLAW_WEB_DEV_USER_ID` | `dev-local` |
| `NEXT_PUBLIC_CLAW_CODE_SERVER_ENABLED` | `0` (set `1` with code-server on `4101`) |

`npm run dev` loads `.env.development` (committed defaults). Override with `.env.local` from `.env.local.example`.

## Verify

```bash
./deploy/stack/gateway.sh verify-web-ui
```

## E2E

Uses **system Google Chrome** (`channel: chrome`). **Do not run** `npx playwright install` — it will try to download ~169MB Chromium.

```bash
cd web/claw-web-ui
npm install
# tap-up + web-ui on :4100 in other terminals, then:
npm run test:e2e
```

Requires macOS/Google Chrome installed. `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1` is set in `package.json` scripts.
