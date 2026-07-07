# Multi-Environment Self-Hosted e2b Deploy Guide

Author: kejiqing

## Goal

For multi-cluster/multi-environment deployment, keep all environment-specific values in repo root `.env` anchors and avoid hardcoded host/domain values in runtime scripts.

## What changed

- Added gateway command: `./deploy/stack/gateway.sh sync-e2b-env`
  - Script path: `deploy/stack/lib/sync-e2b-host-env.sh`
  - Applies `.env` anchors to co-located e2bserver configs:
    - panel config: `config/deploy.toml`
    - worker config: `config/worker.toml`
- `gateway.sh up` now auto-runs `sync-e2b-env` when `CLAW_E2B_SERVER_ROOT` is set.
- Added production template: `deploy/stack/env.selfhosted-prod.example`
  - Uses `anchors + derived` pattern for environment migration.
- `sync-e2b-env` now enforces DNS correctness:
  - It fails fast if `CLAW_E2B_DOMAIN` does not resolve to `CLAW_E2B_HOST`.
  - No `/etc/hosts` fallback is performed.

## Source of truth

Only maintain environment-dependent values in `.env`:

- e2b endpoints and domain:
  - `CLAW_E2B_HOST`
  - `CLAW_E2B_API_PORT`
  - `CLAW_E2B_SANDBOX_PORT`
  - `CLAW_E2B_TRAFFIC_PORT`
  - `CLAW_E2B_DOMAIN`
- e2b host integration:
  - `CLAW_E2B_SERVER_ROOT`
  - `CLAW_E2B_NAS_HOST_MOUNT`
- database anchors:
  - `CLAW_PG_HOST`
  - `CLAW_PG_PORT`
  - `CLAW_PG_USER`
  - `CLAW_PG_PASSWORD`
  - `CLAW_PG_DATABASE`

Derived URLs in `.env` should reference anchors only.

## Recommended workflow (new environment)

1. Copy template:
   - `cp deploy/stack/env.selfhosted-prod.example .env`
2. Edit only anchor values.
3. Ensure DNS:
   - `CLAW_E2B_DOMAIN` apex and wildcard point to `CLAW_E2B_HOST`.
4. Sync e2b config:
   - `./deploy/stack/gateway.sh sync-e2b-env --restart --nginx`
5. Start gateway/admin:
   - `./deploy/stack/gateway.sh quick` (or `up`)
6. Verify:
   - `curl http://127.0.0.1:${GATEWAY_HOST_PORT}/readyz`
   - `curl http://127.0.0.1:${GATEWAY_HOST_PORT}/v1/gateway/global-settings/e2b-singletons`
   - `curl http://${CLAW_E2B_HOST}:${CLAW_E2B_API_PORT}/health`

## Notes

- Do not hand-edit e2bserver toml files for environment migration; use `sync-e2b-env`.
- Do not introduce fallback paths for DNS/hosts in deploy scripts.
- Keep one default deployment path (anchors in `.env` + sync command) to reduce operational branching.
