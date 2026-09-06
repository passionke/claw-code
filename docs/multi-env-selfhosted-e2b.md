# Multi-Environment Self-Hosted e2b Deploy Guide

Author: kejiqing

## Goal

Gateway `.env` holds **only Gateway → e2b API client** settings. e2b host config (NAS bind root, traffic, `sandbox_domain`) lives in **e2bserver** `config/deploy.toml` on the e2b machine.

## claw-code `.env` (Gateway)

Maintain:

- `CLAW_E2B_API_URL` / `CLAW_E2B_API_KEY`
- `CLAW_E2B_DOMAIN` / `CLAW_E2B_SANDBOX_URL` (self-hosted exec/traffic)
- `CLAW_CLUSTER_ID`, PG URLs, LLM bootstrap, gateway ports

Optional anchor pattern (see `deploy/stack/env.selfhosted-prod.example`):

- `CLAW_E2B_HOST` + ports → derive `CLAW_E2B_API_URL` / `CLAW_E2B_SANDBOX_URL` in `.env` only (compose convenience, not pushed to e2b)

## e2bserver (e2b host)

Edit on the e2b machine:

- `config/deploy.toml` — `sandbox_domain`, `[nas].host_mount_root`, `traffic_addr`
- `scripts/install-nginx-traffic.sh` when traffic/nginx is needed

Verify: `curl http://<e2b-api>/health` → `nas.hostMountRoot`, `sandboxInject: bind`.

## Recommended workflow (new environment)

1. Copy template: `cp deploy/stack/env.selfhosted-prod.example .env`
2. Edit Gateway anchors (API URL, domain, PG, cluster id).
3. On e2b host: configure `deploy.toml` + NAS directory + nginx traffic.
4. `./deploy/stack/gateway.sh up` (or `quick`)
5. Verify:
   - `curl http://127.0.0.1:${GATEWAY_HOST_PORT}/v1/gateway/bootstrap/status`
   - `curl http://127.0.0.1:${GATEWAY_HOST_PORT}/v1/gateway/global-settings` → `e2bNas` from e2b `/health`

## Notes

- Do not duplicate e2b `[nas]` / `sandbox_domain` in claw-code `.env`.
- One path: Gateway calls e2b API; e2b owns bind/traffic config.

## Workbox example (2026-08-28)

| Surface | URL |
|---------|-----|
| Admin (NeuroGate playground) | `https://neurogate.workbox.spone.xyz/admin` |
| Gateway API | `https://gateway.workbox.spone.xyz` |
| e2b Panel | `https://e2b.workbox.spone.xyz` |
| Sandbox traffic | `{port}-sbx_{id}.workbox.spone.xyz` |

Admin domain spelling is **`neurogate`** (matches product name NeuroGate). See [`deploy/docs/workbox-neurogate-https.md`](../deploy/docs/workbox-neurogate-https.md).
