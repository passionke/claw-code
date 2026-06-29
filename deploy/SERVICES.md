# Deploy Service Boundaries

Author: kejiqing

Each service has its own build and deploy lifecycle. Changes to one service should not
require rebuilding or restarting others.

## Services

| Directory | Service | Build | Deploy |
|-----------|---------|-------|--------|
| `deploy/stack/` | **Gateway** | `gateway.sh pack-deploy` / `gateway.sh quick` | `gateway.sh up` |
| `deploy/fc-sandbox/` | **FC Sandboxes** | `build-claw-*.py` scripts | e2b template build |
| `deploy/pg/` | **PostgreSQL** | `gateway.sh infra-pg-up` / `pg-up` | Independent lifecycle |

## Cross-Service Dependencies

```
Gateway ──(connects)──> PostgreSQL
Gateway ──(creates)───> FC Sandboxes (workers / singletons)
```

`claw_pool` 表仍由 gateway 迁移保留（历史 JOIN）；**无** 独立 pool-daemon 进程。

## Build Isolation

- **Gateway changes**: `gateway.sh pack-deploy` rebuilds the gateway image; does NOT rebuild FC templates
- **Worker template changes**: `deploy/fc-sandbox/build-claw-worker-*.py` rebuilds e2b templates; gateway does NOT need restart unless API contract changes
- **PG changes**: Run migration via `gateway.sh admin-migrate` or restart gateway with `CLAW_GATEWAY_SKIP_DB_MIGRATE=0`

## Key Principle

**Do not couple service A's deploy to service B's source code.**

See also: [`docs/architecture-governance.md`](../docs/architecture-governance.md), [`docs/README.md`](../docs/README.md).
