# PostgreSQL Deployment

PG is deployed independently from the gateway. Use `gateway.sh pg-up` / `gateway.sh pg-down`
for the compose-managed PG, or `gateway.sh infra-pg-up` / `gateway.sh infra-pg-down` for the
shared infrastructure PG.

## Connection

Gateway connects via `CLAW_GATEWAY_DATABASE_URL` env var. No other gateway service should
manage PG lifecycle. The gateway binary performs migration at startup (unless
`CLAW_GATEWAY_SKIP_DB_MIGRATE=1` is set for secondary instances sharing the same PG).

## Migration

Migrations are embedded in the gateway binary (`GatewaySessionDb::open()`). To run migrations
without starting the full gateway:

```bash
# Run embedded migrations via gateway --migrate-only (CLI mode)
CLAW_GATEWAY_SKIP_DB_MIGRATE=0 ./target/release/http-gateway-rs --migrate-only
```

## Tables

| Table | Owner | Description |
|-------|-------|-------------|
| `gateway_sessions` | gateway | Session metadata |
| `gateway_turns` | gateway | Turn metadata, timing, artifacts |
| `gateway_feedback` | gateway | User feedback |
| `gateway_conversation_translate` | gateway | Translation cache |
| `gateway_global_settings` | gateway | JSON KV store (clawTap, LLM, Git PATs, etc.) |
| `project_config` | gateway | Project configuration per proj_id |
| `project_config_revisions` | gateway | Immutable version history |
| `project_entity_revisions` | gateway | Entity revision history |
| `gateway_llm_cluster_model` | gateway | LLM model cluster config |
| `gateway_llm_cluster_revision` | gateway | LLM revision history |
| `gateway_llm_cluster_state` | gateway | LLM cluster state snapshots |
| `claw_pool` | gateway (legacy schema) | Historical pool registry; **no live heartbeat** in FC-only mode |
