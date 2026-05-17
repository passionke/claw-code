# L5 — Auth & audit (optional SaaS)

Version: **v1**  
Author: kejiqing

## Purpose

Multi-tenant access when `CLAW_GATEWAY_AUTH=1`.

## Authentication

All gateway routes except `GET /healthz` require:

```
Authorization: Bearer <JWT>
```

JWT: RS256, `sub` = user id, optional `tenant_id` claim.

## Authorization

- `dsId` must appear in `CLAW_DS_REGISTRY` entry allowed for caller's `tenant_id`.
- Cross-tenant access → `403`.

## Audit

`GET /v1/audit?tenant_id=&from=&to=` returns solve lifecycle events (no secrets).

Stored fields: `ts`, `tenant_id`, `user_sub`, `session_id`, `ds_id`, `action`, `detail_json`.

## Self-check (M5)

`cargo test -p http-gateway-rs -- auth_audit`

With auth disabled (default), tests verify audit write is no-op and routes stay open.
