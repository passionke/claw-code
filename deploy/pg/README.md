# PostgreSQL Deployment

PG is deployed independently from the gateway. Use `gateway.sh pg-up` / `gateway.sh pg-down`
for the compose-managed PG, or `gateway.sh infra-pg-up` / `gateway.sh infra-pg-down` for the
shared infrastructure PG.

## Connection

Gateway connects via `CLAW_GATEWAY_DATABASE_URL` env var. No other gateway service should
manage PG lifecycle. The gateway binary performs migration at startup (unless
`CLAW_GATEWAY_SKIP_DB_MIGRATE=1` is set for secondary instances sharing the same PG).

## Migration

Schema migrations are versioned SQL files under
`rust/crates/http-gateway-rs/migrations/` (sequential integers: `1_baseline.sql`,
`2_…`, …), embedded via `sqlx::migrate!` and applied at gateway startup
(`GatewaySessionDb::open` → `db_migrate::run`), unless
`CLAW_GATEWAY_SKIP_DB_MIGRATE=1` is set for secondary instances sharing the same PG.

Applied versions are recorded in PostgreSQL table `_sqlx_migrations`.
Legacy databases that already have business tables but no `_sqlx_migrations`
rows are **stamped** at version `1` (baseline not re-executed).

There is no separate `--migrate-only` / `admin-migrate` CLI: restart the primary
gateway with migrate enabled, or point a one-shot process at the same
`CLAW_GATEWAY_DATABASE_URL` and let startup migrate.
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
| `claw_pool` | gateway (legacy schema) | Historical pool registry; **no live heartbeat** in e2b-only mode |
