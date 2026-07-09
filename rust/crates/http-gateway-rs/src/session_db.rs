//! `PostgreSQL` persistence for gateway sessions, turns, and feedback. Author: kejiqing
//!
//! **Persistence split (see `docs/persistence-model.md`):** conversation jsonl remains the
//! runtime source of truth on disk; `gateway_turns` stores per-`turn_id` terminal snapshots
//! (`report_message`, `output_json`, …) so gateway restarts and `GET /v1/tasks` handoff stay
//! consistent at **turn** granularity.
//!
//! **Per-`proj_id` agent bundle:** `project_config` stores rules / MCP / skills sources for
//! materializing `proj_<id>/home` (see `docs/project-config-model.md`). Author: kejiqing

use std::collections::BTreeMap;

use crate::biz_advice_report::{report_body_from_persisted, solve_failure_detail_from_output_json};
use crate::cluster_scope::resolve_gateway_cluster_id_for_connect;
use crate::pool::system_landlock_default_json;
use crate::turn_id::{self, TURN_ID_PREFIX};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{Error as SqlxError, PgPool, QueryBuilder, Row};

/// One row for [`GatewaySessionDb::list_sessions_for_proj`]. Author: kejiqing
#[derive(Debug, Clone)]
pub struct GatewaySessionSummary {
    pub session_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub turn_count: i64,
    pub preview_prompt: Option<String>,
    /// Who created the session (`gateway-admin`, external app, …). Author: kejiqing
    pub client_origin: Option<String>,
    /// Any turn in session has `gateway_feedback.feedback = 'bad'`. Author: kejiqing
    pub has_bad_feedback: bool,
    /// Any turn in session has `gateway_feedback.feedback = 'good'`. Author: kejiqing
    pub has_good_feedback: bool,
}

/// One row for [`GatewaySessionDb::list_turns_for_session`]. Author: kejiqing
#[derive(Debug, Clone)]
pub struct GatewayTurnSummary {
    pub turn_id: String,
    pub user_prompt: Option<String>,
    pub status: String,
    pub created_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub has_report: bool,
    /// Extracted `message` for admin replay (not raw solve JSON). Author: kejiqing
    pub report_body: Option<String>,
    /// `output_json.detail` when status is `failed` (admin error display). Author: kejiqing
    pub failure_detail: Option<String>,
    /// Request origin at turn enqueue (`gateway-admin`, …). Author: kejiqing
    pub client_origin: Option<String>,
    /// `good` / `bad` from `gateway_feedback` when present. Author: kejiqing
    pub feedback: Option<String>,
    /// Snapshot `extraSession` from enqueue `entry_params_json`. Author: kejiqing
    pub extra_session: Option<Value>,
    /// Pool assigned at enqueue or exec (`gateway_turns.pool_id`). Author: kejiqing
    pub pool_id: Option<String>,
    /// Worker container name after pool exec starts. Author: kejiqing
    pub worker_name: Option<String>,
    /// `podman exec --user` for this turn (`claw`, etc.). Author: kejiqing
    pub worker_exec_user: Option<String>,
}

/// Row for tools API: session path + turn times + 1-based user turn index (single query). Author: kejiqing
#[derive(Debug, Clone)]
pub struct TurnToolsContext {
    pub session_home_rel: String,
    pub created_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub user_turn_index: i64,
}

/// Latest `gateway_turns` row for a session (see [`GatewaySessionDb::fetch_latest_turn_for_session`]).
#[derive(Debug, Clone)]
pub struct LatestTurnRow {
    pub turn_id: String,
    pub session_id: String,
    pub proj_id: i64,
    pub status: String,
    pub created_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub report_message: Option<String>,
    pub output_json: Option<Value>,
    pub claw_exit_code: Option<i32>,
    pub user_prompt: Option<String>,
    pub pool_id: Option<String>,
    pub worker_name: Option<String>,
    pub worker_exec_user: Option<String>,
}

/// One row per `proj_id`: rules, MCP map, inline skills, optional `CLAUDE.md` body. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectConfigRow {
    pub proj_id: i64,
    pub content_rev: String,
    /// Solve/materialize target; unchanged while `draft_open`. Author: kejiqing
    pub stable_content_rev: Option<String>,
    /// In-progress edits use `content_rev = __draft__`. Author: kejiqing
    pub draft_open: bool,
    pub updated_at_ms: i64,
    pub rules_json: Value,
    pub mcp_servers_json: Value,
    /// Deprecated: git skill sources; kept for schema compat, not applied. Author: kejiqing
    pub skills_sources_json: Value,
    /// `[{ "skillName", "skillContent" }, ...]` — sole skills source for materialize. Author: kejiqing
    pub skills_json: Value,
    pub allowed_tools_json: Value,
    pub claude_md: Option<String>,
    /// Per-project one-way git pull: `{ gitUrl, gitRef, gitToken, enabled, lastPull* }`. Author: kejiqing
    pub git_sync_json: Value,
    /// First-turn solve preflight: `{ "kind": "none" | "sqlbot_mcp_start" }`. Materialized to disk. Author: kejiqing
    pub solve_preflight_json: Value,
    /// Solve orchestration pipeline: `{ "kind": "single_turn" | "multi_agent_analysis", ... }`. Author: kejiqing
    pub solve_orchestration_json: Value,
    /// Per-turn language inference prompts (`languageInferencePrompt`, …). Author: kejiqing
    pub language_pipeline_json: Value,
    /// Allowed `extraSession` business keys for this ds (`string[]`). Author: kejiqing
    pub extra_session_fields_json: Value,
    /// Per-ds instruction budgets → `.claw/settings.json`. Author: kejiqing
    pub prompt_limits_json: Value,
    /// Pool worker profile: `{"mode":"strict"|"relaxed"}` (sidecar; not in revision snapshots). Author: kejiqing
    pub worker_profile_json: Value,
}

/// Gateway-managed e2b worker sandbox bound to one project (`project_e2b_worker`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectFcWorkerRow {
    pub proj_id: i64,
    pub slot_index: i32,
    pub sandbox_id: String,
    pub worker_id: String,
    pub template_id: String,
    pub handle_json: Value,
    pub updated_at_ms: i64,
}

/// PG `slot_index` (non-negative) → registry `u32`.
#[inline]
pub fn e2b_worker_slot_u32(slot: i32) -> u32 {
    u32::try_from(slot).unwrap_or(0)
}

/// Registry `slot_index` → PG `i32` (pool size ≤ 16).
#[inline]
pub fn e2b_worker_slot_i32(slot: u32) -> i32 {
    i32::try_from(slot).expect("e2b worker slot_index fits i32")
}

/// One append-only worker rotation audit event (`worker_rotation_log`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct WorkerRotationEvent {
    pub proj_id: i64,
    pub event: String,
    pub sandbox_id: Option<String>,
    pub worker_id: Option<String>,
    pub template_id: Option<String>,
    pub reason: Option<String>,
    pub at_ms: i64,
}

/// Row summary for [`GatewaySessionDb::list_project_config_summaries`]. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectConfigSummary {
    pub proj_id: i64,
    pub content_rev: String,
    pub stable_content_rev: Option<String>,
    pub draft_open: bool,
    pub updated_at_ms: i64,
    pub claude_in_db: bool,
    pub skills_count_db: i64,
    pub rules_count_db: i64,
    pub mcp_servers_count_db: i64,
    pub git_sync_json: Value,
}

/// Immutable snapshot for one `content_rev` (history); `git_sync_json` stays on active `project_config` only.
#[derive(Debug, Clone)]
pub struct ProjectConfigRevisionRow {
    pub proj_id: i64,
    pub content_rev: String,
    pub created_at_ms: i64,
    /// Optional label for admins (search / display). Author: kejiqing
    pub note: Option<String>,
    pub rules_json: Value,
    pub mcp_servers_json: Value,
    pub skills_sources_json: Value,
    pub skills_json: Value,
    pub allowed_tools_json: Value,
    pub claude_md: Option<String>,
}

/// Immutable per-entity snapshot (L2 history). Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectEntityRevisionRow {
    pub proj_id: i64,
    pub domain: String,
    pub entity_key: String,
    pub entity_rev: String,
    pub created_at_ms: i64,
    pub note: Option<String>,
    pub body: Value,
}

#[derive(Debug, Clone)]
pub struct ProjectEntityRevisionSummary {
    pub entity_rev: String,
    pub created_at_ms: i64,
    pub note: Option<String>,
}

/// One immutable global LLM model revision (`gateway_llm_cluster_revision`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct GatewayLlmModelRevisionRow {
    pub cluster_id: String,
    pub model_id: String,
    pub model_rev: String,
    pub created_at_ms: i64,
    pub name: String,
    pub base_model_url: String,
    pub model_name: String,
    pub note: Option<String>,
}

impl GatewayLlmModelRevisionRow {
    #[must_use]
    pub fn with_cluster_id(mut self, cluster_id: &str) -> Self {
        self.cluster_id = cluster_id.to_string();
        self
    }
}

/// Cached zh translation snapshot for one session (`gateway_conversation_translate`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct ConversationTranslateSnapshotRow {
    pub session_id: String,
    pub proj_id: i64,
    pub source_fingerprint: String,
    pub turns_json: Value,
    pub markdown: String,
    pub target_language: String,
    pub model_id: Option<String>,
    /// `translating` | `ready` | `error`. Author: kejiqing
    pub status: String,
    pub error_text: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Per-cluster LLM model row (`gateway_llm_cluster_model`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct GatewayLlmClusterModelRow {
    pub cluster_id: String,
    pub model_id: String,
    pub name: String,
    pub base_model_url: String,
    pub model_name: String,
    pub current_rev: String,
    pub api_key_ciphertext: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Active LLM pointer for one cluster (`gateway_llm_cluster_state`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct GatewayLlmClusterStateRow {
    pub cluster_id: String,
    pub active_model_id: String,
    pub active_model_rev: String,
    pub active_applied_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

/// Summary row for version list API. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectConfigRevisionSummary {
    pub content_rev: String,
    pub created_at_ms: i64,
    pub note: Option<String>,
    pub claude_in_db: bool,
    pub skills_count_db: i64,
    pub rules_count_db: i64,
    pub mcp_servers_count_db: i64,
}

/// Row for [`GatewaySessionDb::upsert_claw_pool`].
#[derive(Debug, Clone)]
pub struct ClawPoolUpsert<'a> {
    pub pool_id: &'a str,
    pub registration_time_ms: i64,
    pub slots_max: i32,
    pub slots_min: i32,
    pub advertise_ip: &'a str,
    pub sse_port: i32,
    /// Browser-reachable gateway base (`http://host:port`) for Admin pool picker. Author: kejiqing
    pub gateway_base: &'a str,
    pub last_heartbeat_ms: i64,
}

/// One row from [`GatewaySessionDb::list_claw_pools`]. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ClawPoolRow {
    pub pool_id: String,
    pub registration_time_ms: i64,
    pub slots_max: i32,
    pub slots_min: i32,
    pub advertise_ip: String,
    pub sse_port: i32,
    pub gateway_base: String,
    pub last_heartbeat_ms: i64,
}

/// Pool heartbeat fresh if within 120s (matches `claw-stack-verify.sh`). Author: kejiqing
#[must_use]
pub fn is_claw_pool_online(last_heartbeat_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_heartbeat_ms) < 120_000
}

/// Millisecond timestamp for pool registry (shared with daemon). Author: kejiqing
#[must_use]
pub fn now_ms_for_registry() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Payload for [`GatewaySessionDb::upsert_project_config`].
#[derive(Debug, Clone)]
pub struct ProjectConfigUpsert<'a> {
    pub proj_id: i64,
    pub content_rev: &'a str,
    pub stable_content_rev: Option<&'a str>,
    pub draft_open: bool,
    pub updated_at_ms: i64,
    pub rules_json: &'a Value,
    pub mcp_servers_json: &'a Value,
    pub skills_sources_json: &'a Value,
    pub skills_json: &'a Value,
    pub allowed_tools_json: &'a Value,
    pub claude_md: Option<&'a str>,
    pub git_sync_json: &'a Value,
    pub solve_preflight_json: &'a Value,
    pub solve_orchestration_json: &'a Value,
    pub language_pipeline_json: &'a Value,
    pub extra_session_fields_json: &'a Value,
    pub prompt_limits_json: &'a Value,
    pub worker_profile_json: &'a Value,
}

/// Gateway session index: one row per `(cluster_id, session_id, proj_id)`.
pub struct GatewaySessionDb {
    pool: PgPool,
    database_url_redacted: String,
    cluster_id: String,
}

// Second gateway on shared PG: node A already ran migrate (pg_advisory_lock). Author: kejiqing
fn gateway_skip_db_migrate_from_env() -> bool {
    matches!(
        std::env::var("CLAW_GATEWAY_SKIP_DB_MIGRATE")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1" | "true" | "yes" | "TRUE" | "YES")
    )
}

fn row_to_project_fc_worker(row: &sqlx::postgres::PgRow) -> Result<ProjectFcWorkerRow, SqlxError> {
    Ok(ProjectFcWorkerRow {
        proj_id: row.try_get("proj_id")?,
        slot_index: row.try_get("slot_index").unwrap_or(0),
        sandbox_id: row.try_get("sandbox_id")?,
        worker_id: row.try_get("worker_id")?,
        template_id: row.try_get("template_id")?,
        handle_json: row.try_get::<Json<Value>, _>("handle_json")?.0,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

impl GatewaySessionDb {
    /// Connects using `CLAW_GATEWAY_DATABASE_URL` (required).
    pub async fn open() -> Result<Self, SqlxError> {
        let url = std::env::var("CLAW_GATEWAY_DATABASE_URL")
            .map_err(|_| SqlxError::Configuration("CLAW_GATEWAY_DATABASE_URL is not set".into()))?;
        let url = url.trim();
        if url.is_empty() {
            return Err(SqlxError::Configuration(
                "CLAW_GATEWAY_DATABASE_URL is empty".into(),
            ));
        }
        if gateway_skip_db_migrate_from_env() {
            return Self::connect_without_migrate(url).await;
        }
        Self::connect(url).await
    }

    /// Connect without DDL (pool daemon; gateway-rs owns `migrate`). Author: kejiqing
    pub async fn open_without_migrate() -> Result<Self, SqlxError> {
        let url = std::env::var("CLAW_GATEWAY_DATABASE_URL")
            .map_err(|_| SqlxError::Configuration("CLAW_GATEWAY_DATABASE_URL is not set".into()))?;
        let url = url.trim();
        if url.is_empty() {
            return Err(SqlxError::Configuration(
                "CLAW_GATEWAY_DATABASE_URL is empty".into(),
            ));
        }
        Self::connect_without_migrate(url).await
    }

    /// Connect and run schema migration (tests and explicit URLs).
    pub async fn connect(database_url: &str) -> Result<Self, SqlxError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        if let Err(e) = Self::migrate(&pool).await {
            eprintln!(
                "http-gateway-rs: GatewaySessionDb migrate failed (check PG / partial proj_id DDL): {e}"
            );
            return Err(e);
        }
        let db = Self {
            pool,
            database_url_redacted: redact_database_url(database_url),
            cluster_id: resolve_gateway_cluster_id_for_connect()?,
        };
        db.ensure_gateway_global_settings_row().await?;
        Ok(db)
    }

    /// Connect without schema migration (existing PG schema required). Author: kejiqing
    pub async fn connect_without_migrate(database_url: &str) -> Result<Self, SqlxError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self {
            pool,
            database_url_redacted: redact_database_url(database_url),
            cluster_id: resolve_gateway_cluster_id_for_connect()?,
        })
    }

    /// This gateway's cluster root (`CLAW_CLUSTER_ID`). Author: kejiqing
    #[must_use]
    pub fn cluster_id(&self) -> &str {
        &self.cluster_id
    }

    #[must_use]
    pub fn database_url_redacted(&self) -> &str {
        &self.database_url_redacted
    }

    #[must_use]
    pub fn pg_pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn turn_exists(&self, turn_id: &str) -> Result<bool, SqlxError> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM gateway_turns WHERE turn_id = $1)")
                .bind(turn_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(exists)
    }

    /// `session_id` + `proj_id` for a `turn_id` (report relay terminal `done`). Author: kejiqing
    pub async fn turn_session_scope(
        &self,
        turn_id: &str,
    ) -> Result<Option<(String, i64)>, SqlxError> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT session_id, proj_id FROM gateway_turns WHERE turn_id = $1 LIMIT 1",
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn migrate(pool: &PgPool) -> Result<(), SqlxError> {
        // Serialize DDL when gateway-rs and pool-daemon start together (CI release up). Author: kejiqing
        const MIGRATE_ADVISORY_LOCK: i64 = 0x434C_4157_4D49;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(MIGRATE_ADVISORY_LOCK)
            .execute(pool)
            .await?;
        let result = Self::run_migrate(pool).await;
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(MIGRATE_ADVISORY_LOCK)
            .execute(pool)
            .await;
        result
    }

    #[allow(clippy::too_many_lines)]
    async fn run_migrate(pool: &PgPool) -> Result<(), SqlxError> {
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_sessions (
                session_id TEXT NOT NULL,
                ds_id BIGINT NOT NULL,
                session_home TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (session_id, ds_id)
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_turns (
                turn_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                ds_id BIGINT NOT NULL,
                status TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                finished_at_ms BIGINT
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_gateway_turns_session ON gateway_turns(session_id, ds_id)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_feedback (
                session_id TEXT NOT NULL,
                ds_id BIGINT NOT NULL,
                turn_id TEXT NOT NULL,
                feedback TEXT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (session_id, ds_id, turn_id)
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_conversation_translate (
                session_id TEXT NOT NULL,
                ds_id BIGINT NOT NULL,
                source_fingerprint TEXT NOT NULL,
                turns_json JSONB NOT NULL,
                markdown TEXT NOT NULL,
                target_language TEXT NOT NULL DEFAULT 'zh-CN',
                model_id TEXT,
                status TEXT NOT NULL DEFAULT 'ready',
                error_text TEXT,
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (session_id, ds_id)
            )",
        )
        .execute(pool)
        .await?;

        for ddl in [
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS user_prompt TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS report_message TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS output_json JSONB",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS claw_exit_code INT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS pool_id TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS worker_name TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS worker_exec_user TEXT",
            "ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS client_origin TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS client_origin TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS entry_params_json JSONB",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS artifacts_ready BOOLEAN NOT NULL DEFAULT FALSE",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS solve_task_json JSONB",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS solve_timing_jsonb JSONB",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS spill_json JSONB",
            "ALTER TABLE gateway_conversation_translate ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'ready'",
            "ALTER TABLE gateway_conversation_translate ADD COLUMN IF NOT EXISTS error_text TEXT",
        ] {
            sqlx::query(ddl).execute(pool).await?;
        }

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS cc_messages (
                message_id BIGSERIAL PRIMARY KEY,
                session_id TEXT NOT NULL,
                ds_id BIGINT NOT NULL,
                turn_id TEXT NOT NULL REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
                iteration_id UUID,
                seq INT NOT NULL,
                role TEXT NOT NULL,
                blocks JSONB NOT NULL,
                usage JSONB,
                created_at_ms BIGINT NOT NULL,
                UNIQUE (turn_id, seq)
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cc_messages_session ON cc_messages(session_id, ds_id, created_at_ms)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_runtime_iterations (
                iteration_id UUID PRIMARY KEY,
                turn_id TEXT NOT NULL REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
                iteration_index INT NOT NULL,
                started_at_ms BIGINT NOT NULL,
                finished_at_ms BIGINT,
                UNIQUE (turn_id, iteration_index)
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_session_artifacts (
                artifact_id UUID PRIMARY KEY,
                session_id TEXT NOT NULL,
                ds_id BIGINT NOT NULL,
                turn_id TEXT,
                kind TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                storage_uri TEXT,
                sha256 TEXT,
                size_bytes BIGINT,
                content TEXT,
                content_json JSONB,
                created_at_ms BIGINT NOT NULL,
                UNIQUE (session_id, ds_id, turn_id, relative_path)
            )",
        )
        .execute(pool)
        .await?;

        // After CREATE TABLE: upgrade legacy rows missing pool-v1 artifact columns (004).
        for ddl in [
            "ALTER TABLE gateway_session_artifacts ADD COLUMN IF NOT EXISTS content TEXT",
            "ALTER TABLE gateway_session_artifacts ADD COLUMN IF NOT EXISTS content_json JSONB",
        ] {
            sqlx::query(ddl).execute(pool).await?;
        }
        // Legacy 002 tables lack UNIQUE for upsert_workspace_tar_b64 ON CONFLICT. Author: kejiqing
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS gateway_session_artifacts_session_ds_turn_path_key \
             ON gateway_session_artifacts (session_id, ds_id, turn_id, relative_path)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS claw_pool (
                pool_id TEXT PRIMARY KEY,
                registration_time_ms BIGINT NOT NULL,
                slots_max INT NOT NULL,
                slots_min INT NOT NULL,
                advertise_ip TEXT NOT NULL,
                sse_port INT NOT NULL,
                gateway_base TEXT NOT NULL DEFAULT '',
                last_heartbeat_ms BIGINT NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "ALTER TABLE claw_pool ADD COLUMN IF NOT EXISTS gateway_base TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_gateway_turns_pool_id ON gateway_turns(pool_id)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS project_config (
                ds_id BIGINT PRIMARY KEY,
                content_rev TEXT NOT NULL DEFAULT '',
                updated_at_ms BIGINT NOT NULL,
                rules_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                mcp_servers_json JSONB NOT NULL DEFAULT '{}'::jsonb,
                skills_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                allowed_tools_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                claude_md TEXT
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS allowed_tools_json JSONB NOT NULL DEFAULT '[]'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS skills_json JSONB NOT NULL DEFAULT '[]'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS git_sync_json JSONB NOT NULL DEFAULT '{}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query("ALTER TABLE project_config ADD COLUMN IF NOT EXISTS stable_content_rev TEXT")
            .execute(pool)
            .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS draft_open BOOLEAN NOT NULL DEFAULT false",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS solve_preflight_json JSONB NOT NULL DEFAULT '{\"kind\":\"none\"}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS solve_orchestration_json JSONB NOT NULL DEFAULT '{\"kind\":\"single_turn\"}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS language_pipeline_json JSONB NOT NULL DEFAULT '{}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS extra_session_fields_json JSONB NOT NULL DEFAULT '[]'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS prompt_limits_json JSONB NOT NULL DEFAULT '{}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE project_config ADD COLUMN IF NOT EXISTS worker_profile_json JSONB NOT NULL DEFAULT '{\"mode\":\"strict\"}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE project_config SET stable_content_rev = content_rev WHERE stable_content_rev IS NULL OR stable_content_rev = ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS project_config_revision (
                ds_id BIGINT NOT NULL,
                content_rev TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                rules_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                mcp_servers_json JSONB NOT NULL DEFAULT '{}'::jsonb,
                skills_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                skills_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                allowed_tools_json JSONB NOT NULL DEFAULT '[]'::jsonb,
                claude_md TEXT,
                PRIMARY KEY (ds_id, content_rev)
            )",
        )
        .execute(pool)
        .await?;
        Self::run_sql_migration_file(
            pool,
            include_str!("../migrations/005_proj_id_pre_revision.sql"),
        )
        .await?;
        sqlx::query(
            r"INSERT INTO project_config_revision (
                ds_id, proj_id, content_rev, created_at_ms, rules_json, mcp_servers_json,
                skills_sources_json, skills_json, allowed_tools_json, claude_md
            )
            SELECT ds_id, COALESCE(proj_id, ds_id), content_rev, updated_at_ms, rules_json, mcp_servers_json,
                   skills_sources_json, skills_json, allowed_tools_json, claude_md
            FROM project_config
            ON CONFLICT (ds_id, content_rev) DO NOTHING",
        )
        .execute(pool)
        .await?;
        sqlx::query("ALTER TABLE project_config_revision ADD COLUMN IF NOT EXISTS note TEXT")
            .execute(pool)
            .await?;
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS gateway_global_settings (
                singleton_id SMALLINT PRIMARY KEY DEFAULT 1 CHECK (singleton_id = 1),
                settings_json JSONB NOT NULL DEFAULT '{"gitPats":[]}'::jsonb,
                git_pat_tokens_json JSONB NOT NULL DEFAULT '{}'::jsonb,
                updated_at_ms BIGINT NOT NULL DEFAULT 0
            )"#,
        )
        .execute(pool)
        .await?;
        if Self::gateway_global_settings_has_singleton_id(pool).await? {
            sqlx::query(
                r"INSERT INTO gateway_global_settings (singleton_id)
                 VALUES (1) ON CONFLICT (singleton_id) DO NOTHING",
            )
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS system_prompt_default TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS system_prompt_version TEXT NOT NULL DEFAULT 'v1'",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_base_model_url TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_model_name TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_model_api_key TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_model_updated_at_ms BIGINT NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_model_applied_at_ms BIGINT",
        )
        .execute(pool)
        .await?;
        if Self::gateway_global_settings_has_singleton_id(pool).await? {
            sqlx::query(
                r"UPDATE gateway_global_settings SET
                     llm_base_model_url = COALESCE(settings_json #>> '{llmModel,baseModelUrl}', ''),
                     llm_model_name = COALESCE(settings_json #>> '{llmModel,modelName}', ''),
                     llm_model_updated_at_ms = COALESCE(
                       NULLIF(settings_json #>> '{llmModel,updatedAtMs}', '')::bigint, 0),
                     llm_model_applied_at_ms = NULLIF(
                       NULLIF(settings_json #>> '{llmModel,appliedAtMs}', '')::bigint, 0)
                 WHERE singleton_id = 1
                   AND llm_model_updated_at_ms = 0
                   AND settings_json ? 'llmModel'",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                r"UPDATE gateway_global_settings SET
                     llm_model_api_key = COALESCE(git_pat_tokens_json ->> '__gateway_llm_api_key__', '')
                 WHERE singleton_id = 1
                   AND llm_model_api_key = ''
                   AND git_pat_tokens_json ? '__gateway_llm_api_key__'",
            )
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_models_json JSONB NOT NULL DEFAULT '[]'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS llm_model_api_keys_json JSONB NOT NULL DEFAULT '{}'::jsonb",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS active_llm_model_id TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS active_llm_applied_at_ms BIGINT",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS active_llm_model_rev TEXT NOT NULL DEFAULT ''",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_llm_model_revision (
                model_id TEXT NOT NULL,
                model_rev TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                name TEXT NOT NULL,
                base_model_url TEXT NOT NULL,
                model_name TEXT NOT NULL,
                note TEXT,
                PRIMARY KEY (model_id, model_rev)
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE INDEX IF NOT EXISTS idx_gateway_llm_model_revision_list
             ON gateway_llm_model_revision (model_id, created_at_ms DESC)",
        )
        .execute(pool)
        .await?;
        if Self::gateway_global_settings_has_singleton_id(pool).await? {
            sqlx::query(
                r"UPDATE gateway_global_settings SET
                     llm_models_json = jsonb_build_array(jsonb_build_object(
                       'id', 'llm-migrated',
                       'name', 'Migrated',
                       'baseModelUrl', llm_base_model_url,
                       'modelName', llm_model_name,
                       'createdAtMs', llm_model_updated_at_ms,
                       'updatedAtMs', llm_model_updated_at_ms
                     )),
                     llm_model_api_keys_json = jsonb_build_object(
                       'llm-migrated', llm_model_api_key),
                     active_llm_model_id = 'llm-migrated',
                     active_llm_applied_at_ms = llm_model_applied_at_ms
                 WHERE singleton_id = 1
                   AND jsonb_array_length(llm_models_json) = 0
                   AND length(trim(llm_base_model_url)) > 0
                   AND length(trim(llm_model_name)) > 0",
            )
            .execute(pool)
            .await?;
            let default_scaffold = runtime::builtin_system_prompt_scaffold_default();
            sqlx::query(
                r"UPDATE gateway_global_settings
                 SET system_prompt_default = $1, system_prompt_version = 'v1'
                 WHERE singleton_id = 1
                   AND (system_prompt_default = '' OR length(trim(system_prompt_default)) = 0)",
            )
            .bind(default_scaffold)
            .execute(pool)
            .await?;
        }
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS project_entity_revision (
                ds_id BIGINT NOT NULL,
                domain TEXT NOT NULL,
                entity_key TEXT NOT NULL,
                entity_rev TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                note TEXT,
                body JSONB NOT NULL,
                PRIMARY KEY (ds_id, domain, entity_key, entity_rev)
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE INDEX IF NOT EXISTS idx_project_entity_revision_list
             ON project_entity_revision (ds_id, domain, entity_key, created_at_ms DESC)",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_llm_cluster_model (
                cluster_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                name TEXT NOT NULL,
                base_model_url TEXT NOT NULL,
                model_name TEXT NOT NULL,
                current_rev TEXT NOT NULL DEFAULT '',
                api_key_ciphertext TEXT NOT NULL DEFAULT '',
                created_at_ms BIGINT NOT NULL,
                updated_at_ms BIGINT NOT NULL,
                PRIMARY KEY (cluster_id, model_id)
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_llm_cluster_state (
                cluster_id TEXT PRIMARY KEY,
                active_model_id TEXT NOT NULL DEFAULT '',
                active_model_rev TEXT NOT NULL DEFAULT '',
                active_applied_at_ms BIGINT,
                updated_at_ms BIGINT NOT NULL DEFAULT 0
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_llm_cluster_revision (
                cluster_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                model_rev TEXT NOT NULL,
                created_at_ms BIGINT NOT NULL,
                name TEXT NOT NULL,
                base_model_url TEXT NOT NULL,
                model_name TEXT NOT NULL,
                note TEXT,
                PRIMARY KEY (cluster_id, model_id, model_rev)
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE INDEX IF NOT EXISTS idx_gateway_llm_cluster_revision_list
             ON gateway_llm_cluster_revision (cluster_id, model_id, created_at_ms DESC)",
        )
        .execute(pool)
        .await?;

        // Legacy live-spill table (stdout-v1); safe no-op if already dropped. Author: kejiqing
        sqlx::query("DROP TABLE IF EXISTS gateway_turn_live_chunks")
            .execute(pool)
            .await?;

        Self::migrate_proj_id_columns(pool).await?;
        Self::migrate_project_e2b_worker_table(pool).await?;
        Self::run_sql_migration_file(
            pool,
            include_str!("../migrations/007_project_e2b_worker.sql"),
        )
        .await?;
        Self::run_sql_migration_file(
            pool,
            include_str!("../migrations/008_worker_rotation_log.sql"),
        )
        .await?;
        Self::migrate_worker_profile_json_column(pool).await?;
        Self::migrate_settings_json_e2b_keys(pool).await?;
        Self::migrate_strict_landlock_default(pool).await?;
        Self::migrate_gateway_turns_e2b_ids(pool).await?;
        Self::run_sql_migration_file(
            pool,
            include_str!("../migrations/006_cluster_id_scoping.sql"),
        )
        .await?;
        Self::migrate_cluster_id_backfill(pool).await?;
        Self::run_sql_migration_file(
            pool,
            include_str!("../migrations/011_cluster_id_models.sql"),
        )
        .await?;
        Self::migrate_cluster_id_phase2(pool).await?;
        Self::run_sql_migration_file(pool, include_str!("../migrations/010_preflight_plugin.sql"))
            .await?;
        Self::run_sql_migration_file(
            pool,
            include_str!("../migrations/012_project_e2b_worker_pool.sql"),
        )
        .await?;
        Self::migrate_project_e2b_worker_pool_slot(pool).await?;

        Ok(())
    }

    const LEGACY_CLUSTER_ID: &'static str = "__legacy__";

    async fn gateway_global_settings_has_singleton_id(pool: &PgPool) -> Result<bool, SqlxError> {
        sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'gateway_global_settings'
                  AND column_name = 'singleton_id'
            )",
        )
        .fetch_one(pool)
        .await
    }

    async fn migrate_cluster_id_backfill(pool: &PgPool) -> Result<(), SqlxError> {
        let has_col: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'gateway_sessions'
                  AND column_name = 'cluster_id'
            )",
        )
        .fetch_one(pool)
        .await?;
        if !has_col {
            return Ok(());
        }
        sqlx::query(
            r"UPDATE gateway_sessions SET cluster_id = split_part(session_home, '/', 1)
              WHERE cluster_id IS NULL
                AND session_home ~ '^[^/]+/proj_[0-9]+/'",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"UPDATE gateway_turns t SET cluster_id = s.cluster_id
              FROM gateway_sessions s
              WHERE t.cluster_id IS NULL
                AND t.session_id = s.session_id AND t.proj_id = s.proj_id
                AND s.cluster_id IS NOT NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"UPDATE gateway_feedback f SET cluster_id = s.cluster_id
              FROM gateway_sessions s
              WHERE f.cluster_id IS NULL
                AND f.session_id = s.session_id AND f.proj_id = s.proj_id
                AND s.cluster_id IS NOT NULL",
        )
        .execute(pool)
        .await?;
        for child in [
            "cc_messages",
            "gateway_session_artifacts",
            "gateway_conversation_translate",
        ] {
            let sql = format!(
                "UPDATE {child} c SET cluster_id = s.cluster_id \
                 FROM gateway_sessions s \
                 WHERE c.cluster_id IS NULL \
                   AND c.session_id = s.session_id AND c.proj_id = s.proj_id \
                   AND s.cluster_id IS NOT NULL"
            );
            sqlx::query(&sql).execute(pool).await?;
        }
        sqlx::query(
            r"UPDATE project_config pc SET cluster_id = sub.cid
              FROM (
                SELECT proj_id, MIN(cluster_id) AS cid FROM gateway_sessions
                WHERE cluster_id IS NOT NULL GROUP BY proj_id
                HAVING COUNT(DISTINCT cluster_id) = 1
              ) sub
              WHERE pc.proj_id = sub.proj_id AND pc.cluster_id IS NULL",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// PK retarget + per-cluster `gateway_global_settings` (idempotent). Author: kejiqing
    async fn migrate_cluster_id_phase2(pool: &PgPool) -> Result<(), SqlxError> {
        let legacy = Self::LEGACY_CLUSTER_ID;
        sqlx::query("UPDATE gateway_sessions SET cluster_id = $1 WHERE cluster_id IS NULL")
            .bind(legacy)
            .execute(pool)
            .await?;
        sqlx::query("UPDATE project_config SET cluster_id = $1 WHERE cluster_id IS NULL")
            .bind(legacy)
            .execute(pool)
            .await?;
        sqlx::query("UPDATE claw_pool SET cluster_id = $1 WHERE cluster_id IS NULL")
            .bind(legacy)
            .execute(pool)
            .await?;
        // Revision rows predate cluster_id column; inherit from project_config (same proj_id).
        sqlx::query(
            r"UPDATE project_config_revision pcr
              SET cluster_id = pc.cluster_id
              FROM project_config pc
              WHERE pcr.proj_id = pc.proj_id
                AND pcr.cluster_id IS NULL
                AND pc.cluster_id IS NOT NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"UPDATE project_entity_revision per
              SET cluster_id = pc.cluster_id
              FROM project_config pc
              WHERE per.proj_id = pc.proj_id
                AND per.cluster_id IS NULL
                AND pc.cluster_id IS NOT NULL",
        )
        .execute(pool)
        .await?;

        let pk_ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_constraint
                WHERE conrelid = 'public.project_config'::regclass
                  AND contype = 'p' AND pg_get_constraintdef(oid) LIKE '%cluster_id%'
            )",
        )
        .fetch_one(pool)
        .await?;
        if !pk_ok {
            sqlx::query("ALTER TABLE project_config DROP CONSTRAINT IF EXISTS project_config_pkey")
                .execute(pool)
                .await?;
            sqlx::query(
                r"INSERT INTO project_config (
                    ds_id, proj_id, cluster_id, content_rev, stable_content_rev, draft_open,
                    updated_at_ms, rules_json, mcp_servers_json, skills_sources_json, skills_json,
                    allowed_tools_json, claude_md, git_sync_json, solve_preflight_json,
                    solve_orchestration_json, language_pipeline_json, extra_session_fields_json,
                    prompt_limits_json, worker_profile_json
                  )
                  SELECT pc.ds_id, pc.proj_id, s.cluster_id, pc.content_rev, pc.stable_content_rev,
                         pc.draft_open, pc.updated_at_ms, pc.rules_json, pc.mcp_servers_json,
                         pc.skills_sources_json, pc.skills_json, pc.allowed_tools_json, pc.claude_md,
                         pc.git_sync_json, pc.solve_preflight_json, pc.solve_orchestration_json,
                         pc.language_pipeline_json, pc.extra_session_fields_json, pc.prompt_limits_json,
                         pc.worker_profile_json
                  FROM project_config pc
                  JOIN (
                    SELECT DISTINCT proj_id, cluster_id FROM gateway_sessions
                    WHERE cluster_id IS NOT NULL AND cluster_id <> $1
                  ) s ON s.proj_id = pc.proj_id
                  WHERE NOT EXISTS (
                    SELECT 1 FROM project_config x
                    WHERE x.cluster_id = s.cluster_id AND x.proj_id = s.proj_id
                  )",
            )
            .bind(legacy)
            .execute(pool)
            .await?;
            sqlx::query("ALTER TABLE project_config ADD PRIMARY KEY (cluster_id, proj_id)")
                .execute(pool)
                .await?;
        }

        sqlx::query(
            r"UPDATE project_e2b_worker w SET cluster_id = pc.cluster_id
              FROM project_config pc
              WHERE w.proj_id = pc.proj_id AND w.cluster_id IS NULL AND pc.cluster_id IS NOT NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(r"UPDATE project_e2b_worker SET cluster_id = $1 WHERE cluster_id IS NULL")
            .bind(legacy)
            .execute(pool)
            .await?;
        let e2b_pk_ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_constraint
                WHERE conrelid = 'public.project_e2b_worker'::regclass
                  AND contype = 'p' AND pg_get_constraintdef(oid) LIKE '%cluster_id%'
            )",
        )
        .fetch_one(pool)
        .await?;
        if !e2b_pk_ok {
            // Renamed from project_fc_worker; legacy PK name may still be project_fc_worker_pkey.
            for drop_name in ["project_e2b_worker_pkey", "project_fc_worker_pkey"] {
                sqlx::query(&format!(
                    "ALTER TABLE project_e2b_worker DROP CONSTRAINT IF EXISTS {drop_name}"
                ))
                .execute(pool)
                .await?;
            }
            sqlx::query("ALTER TABLE project_e2b_worker ALTER COLUMN cluster_id SET NOT NULL")
                .execute(pool)
                .await?;
            sqlx::query("ALTER TABLE project_e2b_worker ADD PRIMARY KEY (cluster_id, proj_id)")
                .execute(pool)
                .await?;
        }

        let has_singleton: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'gateway_global_settings'
                  AND column_name = 'singleton_id'
            )",
        )
        .fetch_one(pool)
        .await?;
        if has_singleton {
            sqlx::query(
                "ALTER TABLE gateway_global_settings DROP CONSTRAINT IF EXISTS gateway_global_settings_singleton_id_check",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                "ALTER TABLE gateway_global_settings DROP CONSTRAINT IF EXISTS gateway_global_settings_pkey",
            )
            .execute(pool)
            .await?;
            sqlx::query(
                r"INSERT INTO gateway_global_settings (
                    cluster_id, singleton_id, settings_json, git_pat_tokens_json, updated_at_ms,
                    system_prompt_default, system_prompt_version,
                    llm_models_json, llm_model_api_keys_json, active_llm_model_id,
                    active_llm_model_rev, active_llm_applied_at_ms,
                    llm_base_model_url, llm_model_name, llm_model_api_key,
                    llm_model_updated_at_ms, llm_model_applied_at_ms
                  )
                  SELECT c.cluster_id, 1, g.settings_json, g.git_pat_tokens_json, g.updated_at_ms,
                         g.system_prompt_default, g.system_prompt_version,
                         g.llm_models_json, g.llm_model_api_keys_json, g.active_llm_model_id,
                         g.active_llm_model_rev, g.active_llm_applied_at_ms,
                         g.llm_base_model_url, g.llm_model_name, g.llm_model_api_key,
                         g.llm_model_updated_at_ms, g.llm_model_applied_at_ms
                  FROM gateway_global_settings g
                  CROSS JOIN (
                    SELECT DISTINCT cluster_id FROM (
                      SELECT cluster_id FROM gateway_sessions
                      WHERE cluster_id IS NOT NULL AND cluster_id <> $1
                      UNION SELECT cluster_id FROM gateway_llm_cluster_state
                      UNION SELECT cluster_id FROM project_config
                      WHERE cluster_id IS NOT NULL AND cluster_id <> $1
                    ) u
                  ) c
                  WHERE g.singleton_id = 1 AND g.cluster_id IS NULL
                    AND NOT EXISTS (
                      SELECT 1 FROM gateway_global_settings x WHERE x.cluster_id = c.cluster_id
                    )",
            )
            .bind(legacy)
            .execute(pool)
            .await?;
            sqlx::query(
                "DELETE FROM gateway_global_settings WHERE singleton_id = 1 AND cluster_id IS NULL",
            )
            .execute(pool)
            .await?;
            sqlx::query("ALTER TABLE gateway_global_settings DROP COLUMN IF EXISTS singleton_id")
                .execute(pool)
                .await?;
            sqlx::query(
                "DELETE FROM gateway_global_settings WHERE cluster_id IS NULL OR cluster_id = $1",
            )
            .bind(legacy)
            .execute(pool)
            .await?;
            let settings_pk_ok: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1 FROM pg_constraint
                    WHERE conrelid = 'public.gateway_global_settings'::regclass
                      AND contype = 'p' AND pg_get_constraintdef(oid) LIKE '%cluster_id%'
                )",
            )
            .fetch_one(pool)
            .await?;
            if !settings_pk_ok {
                sqlx::query("ALTER TABLE gateway_global_settings ADD PRIMARY KEY (cluster_id)")
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn ensure_gateway_global_settings_row(&self) -> Result<(), SqlxError> {
        let empty = json!({"gitPats": []});
        sqlx::query(
            r"INSERT INTO gateway_global_settings (cluster_id, settings_json, git_pat_tokens_json, updated_at_ms)
               VALUES ($1, $2, '{}'::jsonb, 0)
               ON CONFLICT (cluster_id) DO NOTHING",
        )
        .bind(self.cluster_id())
        .bind(Json(empty))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `project_fc_worker` → `project_e2b_worker` (idempotent). Author: kejiqing
    async fn migrate_project_e2b_worker_table(pool: &PgPool) -> Result<(), SqlxError> {
        let fc_exists: bool =
            sqlx::query_scalar("SELECT to_regclass('public.project_fc_worker') IS NOT NULL")
                .fetch_one(pool)
                .await?;
        if !fc_exists {
            return Ok(());
        }
        let e2b_exists: bool =
            sqlx::query_scalar("SELECT to_regclass('public.project_e2b_worker') IS NOT NULL")
                .fetch_one(pool)
                .await?;
        if e2b_exists {
            sqlx::query("DROP TABLE project_fc_worker")
                .execute(pool)
                .await?;
            return Ok(());
        }
        sqlx::query("ALTER TABLE project_fc_worker RENAME TO project_e2b_worker")
            .execute(pool)
            .await?;
        sqlx::query(
            "ALTER INDEX IF EXISTS idx_project_fc_worker_sandbox_id \
             RENAME TO idx_project_e2b_worker_sandbox_id",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// `project_e2b_worker` PK → `(cluster_id, proj_id, slot_index)` for strict worker pool. Author: kejiqing
    async fn migrate_project_e2b_worker_pool_slot(pool: &PgPool) -> Result<(), SqlxError> {
        let table_exists: bool =
            sqlx::query_scalar("SELECT to_regclass('public.project_e2b_worker') IS NOT NULL")
                .fetch_one(pool)
                .await?;
        if !table_exists {
            return Ok(());
        }
        let has_slot: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'project_e2b_worker'
                  AND column_name = 'slot_index'
            )",
        )
        .fetch_one(pool)
        .await?;
        if !has_slot {
            sqlx::query(
                "ALTER TABLE project_e2b_worker ADD COLUMN slot_index INT NOT NULL DEFAULT 0",
            )
            .execute(pool)
            .await?;
        }
        let pk_has_slot: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_constraint
                WHERE conrelid = 'public.project_e2b_worker'::regclass
                  AND contype = 'p'
                  AND pg_get_constraintdef(oid) LIKE '%slot_index%'
            )",
        )
        .fetch_one(pool)
        .await?;
        if pk_has_slot {
            return Ok(());
        }
        for drop_name in ["project_e2b_worker_pkey", "project_fc_worker_pkey"] {
            sqlx::query(&format!(
                "ALTER TABLE project_e2b_worker DROP CONSTRAINT IF EXISTS {drop_name}"
            ))
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "ALTER TABLE project_e2b_worker ADD PRIMARY KEY (cluster_id, proj_id, slot_index)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// `worker_isolation_json` → `worker_profile_json` (idempotent). Author: kejiqing
    async fn migrate_worker_profile_json_column(pool: &PgPool) -> Result<(), SqlxError> {
        let has_isolation: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'project_config'
                  AND column_name = 'worker_isolation_json'
            )",
        )
        .fetch_one(pool)
        .await?;

        if !has_isolation {
            return Ok(());
        }

        let has_profile: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'public' AND table_name = 'project_config'
                  AND column_name = 'worker_profile_json'
            )",
        )
        .fetch_one(pool)
        .await?;

        if has_profile {
            sqlx::query(
                "UPDATE project_config SET worker_profile_json = worker_isolation_json \
                 WHERE worker_profile_json = '{\"mode\":\"strict\"}'::jsonb",
            )
            .execute(pool)
            .await?;
            sqlx::query("ALTER TABLE project_config DROP COLUMN worker_isolation_json")
                .execute(pool)
                .await?;
        } else {
            sqlx::query(
                "ALTER TABLE project_config RENAME COLUMN worker_isolation_json TO worker_profile_json",
            )
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    /// `settings_json` fc* → e2b* (`fcOvs`, `fcNasApi`, `clawTap.fcObserveSandboxId`). Author: kejiqing
    async fn migrate_settings_json_e2b_keys(pool: &PgPool) -> Result<(), SqlxError> {
        let singleton = Self::gateway_global_settings_has_singleton_id(pool).await?;
        let scope = if singleton {
            "WHERE singleton_id = 1"
        } else {
            ""
        };
        let sql1 = format!(
            r"UPDATE gateway_global_settings SET settings_json =
                CASE
                  WHEN settings_json ? 'fcOvs' AND NOT (settings_json ? 'e2bOvs')
                  THEN jsonb_set(settings_json - 'fcOvs', '{{e2bOvs}}', settings_json->'fcOvs', true)
                  WHEN settings_json ? 'fcOvs'
                  THEN settings_json - 'fcOvs'
                  ELSE settings_json
                END
              {scope}"
        );
        sqlx::query(&sql1).execute(pool).await?;

        let sql2 = format!(
            r"UPDATE gateway_global_settings SET settings_json =
                CASE
                  WHEN settings_json ? 'fcNasApi' AND NOT (settings_json ? 'e2bNasApi')
                  THEN jsonb_set(settings_json - 'fcNasApi', '{{e2bNasApi}}', settings_json->'fcNasApi', true)
                  WHEN settings_json ? 'fcNasApi'
                  THEN settings_json - 'fcNasApi'
                  ELSE settings_json
                END
              {scope}"
        );
        sqlx::query(&sql2).execute(pool).await?;

        let sql3 = format!(
            r"UPDATE gateway_global_settings SET settings_json = jsonb_set(
                settings_json,
                '{{clawTap,e2bObserveSandboxId}}',
                settings_json #> '{{clawTap,fcObserveSandboxId}}',
                true
              )
              {prefix} settings_json #>> '{{clawTap,fcObserveSandboxId}}' IS NOT NULL
                AND settings_json #>> '{{clawTap,e2bObserveSandboxId}}' IS NULL",
            prefix = if singleton {
                "WHERE singleton_id = 1 AND"
            } else {
                "WHERE"
            }
        );
        sqlx::query(&sql3).execute(pool).await?;

        let sql4 = format!(
            r"UPDATE gateway_global_settings SET settings_json = jsonb_set(
                settings_json,
                '{{clawTap}}',
                (settings_json->'clawTap') - 'fcObserveSandboxId',
                false
              )
              {prefix} settings_json->'clawTap' ? 'fcObserveSandboxId'",
            prefix = if singleton {
                "WHERE singleton_id = 1 AND"
            } else {
                "WHERE"
            }
        );
        sqlx::query(&sql4).execute(pool).await?;

        Ok(())
    }

    /// Seed `settings_json.strictLandlockDefault` when absent. Author: kejiqing
    async fn migrate_strict_landlock_default(pool: &PgPool) -> Result<(), SqlxError> {
        let seed = system_landlock_default_json();
        let singleton = Self::gateway_global_settings_has_singleton_id(pool).await?;
        if singleton {
            sqlx::query(
                r"UPDATE gateway_global_settings SET settings_json = jsonb_set(
                    settings_json,
                    '{strictLandlockDefault}',
                    $1::jsonb,
                    true
                  )
                  WHERE singleton_id = 1
                    AND NOT (settings_json ? 'strictLandlockDefault')",
            )
            .bind(Json(seed))
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                r"UPDATE gateway_global_settings SET settings_json = jsonb_set(
                    settings_json,
                    '{strictLandlockDefault}',
                    $1::jsonb,
                    true
                  )
                  WHERE NOT (settings_json ? 'strictLandlockDefault')",
            )
            .bind(Json(seed))
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    /// Historical turn metadata: `fc-cloud` / `fc-interactive` / `fc:sbx_*` → e2b names. Author: kejiqing
    async fn migrate_gateway_turns_e2b_ids(pool: &PgPool) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET pool_id = 'e2b-cloud' WHERE pool_id = 'fc-cloud'")
            .execute(pool)
            .await?;
        sqlx::query(
            "UPDATE gateway_turns SET pool_id = 'e2b-interactive' WHERE pool_id = 'fc-interactive'",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE gateway_turns SET worker_name = 'e2b:' || substring(worker_name FROM 4) \
             WHERE worker_name LIKE 'fc:%'",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Drop leading `--` comment lines so `ALTER` after a file header is not skipped. Author: kejiqing
    fn migration_stmt_ddl(stmt: &str) -> String {
        stmt.lines()
            .filter(|line| {
                let t = line.trim();
                !t.is_empty() && !t.starts_with("--")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn run_sql_migration_file(pool: &PgPool, sql: &str) -> Result<(), SqlxError> {
        for stmt in sql.split(';') {
            let ddl = Self::migration_stmt_ddl(stmt);
            if ddl.is_empty() {
                continue;
            }
            if let Err(e) = sqlx::query(&ddl).execute(pool).await {
                eprintln!("http-gateway-rs: schema migration failed: {e}");
                eprintln!("http-gateway-rs: failed SQL:\n{ddl}");
                return Err(e);
            }
        }
        Ok(())
    }

    /// Add `proj_id` columns and backfill from legacy `ds_id` (idempotent). Author: kejiqing
    async fn migrate_proj_id_columns(pool: &PgPool) -> Result<(), SqlxError> {
        Self::run_sql_migration_file(pool, include_str!("../migrations/005_proj_id.sql")).await?;
        for table in [
            "gateway_async_tasks",
            "gateway_projects",
            "gateway_project_git",
        ] {
            Self::migrate_proj_id_optional_table(pool, table).await?;
        }
        Ok(())
    }

    async fn migrate_proj_id_optional_table(pool: &PgPool, table: &str) -> Result<(), SqlxError> {
        let regclass = format!("public.{table}");
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(&regclass)
            .fetch_one(pool)
            .await?;
        if !exists {
            return Ok(());
        }
        let alter = format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS proj_id BIGINT");
        sqlx::query(&alter).execute(pool).await?;
        let update = format!("UPDATE {table} SET proj_id = ds_id WHERE proj_id IS NULL");
        sqlx::query(&update).execute(pool).await?;
        let not_null = format!("ALTER TABLE {table} ALTER COLUMN proj_id SET NOT NULL");
        sqlx::query(&not_null).execute(pool).await?;
        let idx = format!("CREATE INDEX IF NOT EXISTS idx_{table}_proj_id ON {table} (proj_id)");
        sqlx::query(&idx).execute(pool).await?;
        if table == "gateway_async_tasks" {
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_gateway_async_tasks_session_proj ON gateway_async_tasks (session_id, proj_id)",
            )
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    /// System-level default scaffold (no public write API; update via DB migration). Author: kejiqing
    pub async fn get_gateway_system_prompt_default(&self) -> Result<(String, String), SqlxError> {
        let row = sqlx::query(
            r"SELECT system_prompt_default, system_prompt_version
               FROM gateway_global_settings WHERE cluster_id = $1",
        )
        .bind(self.cluster_id())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok((String::new(), "v1".to_string()));
        };
        let text: String = row.try_get("system_prompt_default")?;
        let version: String = row.try_get("system_prompt_version")?;
        Ok((text, version))
    }

    /// Gateway-wide settings row (PAT vault, etc.). Author: kejiqing
    pub async fn get_gateway_global_settings_raw(&self) -> Result<(Value, Value, i64), SqlxError> {
        let row = sqlx::query(
            r"SELECT settings_json, git_pat_tokens_json, updated_at_ms
               FROM gateway_global_settings WHERE cluster_id = $1",
        )
        .bind(self.cluster_id())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok((json!({"gitPats": []}), json!({}), 0));
        };
        let settings: Value = row.try_get::<Json<Value>, _>("settings_json")?.0;
        let tokens: Value = row.try_get::<Json<Value>, _>("git_pat_tokens_json")?.0;
        let updated_at_ms: i64 = row.try_get("updated_at_ms")?;
        Ok((settings, tokens, updated_at_ms))
    }

    /// LLM model list + active id in `gateway_global_settings` (api keys in `llm_model_api_keys_json`). Author: kejiqing
    pub async fn get_gateway_llm_models_raw(
        &self,
    ) -> Result<(Value, Value, String, String, Option<i64>), SqlxError> {
        let row = sqlx::query(
            r"SELECT llm_models_json, llm_model_api_keys_json, active_llm_model_id,
                      active_llm_model_rev, active_llm_applied_at_ms
               FROM gateway_global_settings WHERE cluster_id = $1",
        )
        .bind(self.cluster_id())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok((json!([]), json!({}), String::new(), String::new(), None));
        };
        let models: Value = row.try_get::<Json<Value>, _>("llm_models_json")?.0;
        let keys: Value = row.try_get::<Json<Value>, _>("llm_model_api_keys_json")?.0;
        let active_id: String = row.try_get("active_llm_model_id")?;
        let active_rev: String = row.try_get("active_llm_model_rev")?;
        let applied: Option<i64> = row.try_get("active_llm_applied_at_ms")?;
        Ok((models, keys, active_id, active_rev, applied))
    }

    pub async fn save_gateway_llm_models_raw(
        &self,
        models_json: &Value,
        api_keys_json: &Value,
        active_llm_model_id: &str,
        active_llm_model_rev: &str,
        active_llm_applied_at_ms: Option<i64>,
        updated_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_global_settings (
                 cluster_id, llm_models_json, llm_model_api_keys_json,
                 active_llm_model_id, active_llm_model_rev, active_llm_applied_at_ms, updated_at_ms
               ) VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (cluster_id) DO UPDATE SET
                 llm_models_json = EXCLUDED.llm_models_json,
                 llm_model_api_keys_json = EXCLUDED.llm_model_api_keys_json,
                 active_llm_model_id = EXCLUDED.active_llm_model_id,
                 active_llm_model_rev = EXCLUDED.active_llm_model_rev,
                 active_llm_applied_at_ms = EXCLUDED.active_llm_applied_at_ms,
                 updated_at_ms = GREATEST(gateway_global_settings.updated_at_ms, EXCLUDED.updated_at_ms),
                 settings_json = gateway_global_settings.settings_json - 'llmModel',
                 git_pat_tokens_json = gateway_global_settings.git_pat_tokens_json - '__gateway_llm_api_key__'",
        )
        .bind(self.cluster_id())
        .bind(Json(models_json))
        .bind(Json(api_keys_json))
        .bind(active_llm_model_id)
        .bind(active_llm_model_rev)
        .bind(active_llm_applied_at_ms)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_llm_model_revision(
        &self,
        model_id: &str,
        model_rev: &str,
    ) -> Result<Option<GatewayLlmModelRevisionRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT model_id, model_rev, created_at_ms, name, base_model_url, model_name, note
               FROM gateway_llm_model_revision
               WHERE model_id = $1 AND model_rev = $2",
        )
        .bind(model_id)
        .bind(model_rev)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(GatewayLlmModelRevisionRow {
            cluster_id: String::new(),
            model_id: row.try_get("model_id")?,
            model_rev: row.try_get("model_rev")?,
            created_at_ms: row.try_get("created_at_ms")?,
            name: row.try_get("name")?,
            base_model_url: row.try_get("base_model_url")?,
            model_name: row.try_get("model_name")?,
            note: row.try_get("note")?,
        }))
    }

    pub async fn list_llm_model_revisions(
        &self,
        model_id: &str,
    ) -> Result<Vec<GatewayLlmModelRevisionRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT model_id, model_rev, created_at_ms, name, base_model_url, model_name, note
               FROM gateway_llm_model_revision
               WHERE model_id = $1
               ORDER BY created_at_ms DESC",
        )
        .bind(model_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(GatewayLlmModelRevisionRow {
                    cluster_id: String::new(),
                    model_id: row.try_get("model_id")?,
                    model_rev: row.try_get("model_rev")?,
                    created_at_ms: row.try_get("created_at_ms")?,
                    name: row.try_get("name")?,
                    base_model_url: row.try_get("base_model_url")?,
                    model_name: row.try_get("model_name")?,
                    note: row.try_get("note")?,
                })
            })
            .collect()
    }

    pub async fn insert_llm_model_revision(
        &self,
        row: &GatewayLlmModelRevisionRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_llm_model_revision (
                 model_id, model_rev, created_at_ms, name, base_model_url, model_name, note
               ) VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (model_id, model_rev) DO NOTHING",
        )
        .bind(&row.model_id)
        .bind(&row.model_rev)
        .bind(row.created_at_ms)
        .bind(&row.name)
        .bind(&row.base_model_url)
        .bind(&row.model_name)
        .bind(&row.note)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Upsert global LLM row (no version history). Author: kejiqing
    pub async fn upsert_llm_model_revision(
        &self,
        row: &GatewayLlmModelRevisionRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_llm_model_revision (
                 model_id, model_rev, created_at_ms, name, base_model_url, model_name, note
               ) VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (model_id, model_rev) DO UPDATE SET
                 name = EXCLUDED.name,
                 base_model_url = EXCLUDED.base_model_url,
                 model_name = EXCLUDED.model_name,
                 note = EXCLUDED.note,
                 created_at_ms = EXCLUDED.created_at_ms",
        )
        .bind(&row.model_id)
        .bind(&row.model_rev)
        .bind(row.created_at_ms)
        .bind(&row.name)
        .bind(&row.base_model_url)
        .bind(&row.model_name)
        .bind(&row.note)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_llm_model_revisions(&self, model_id: &str) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM gateway_llm_model_revision WHERE model_id = $1")
            .bind(model_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_llm_cluster_models(&self, cluster_id: &str) -> Result<i64, SqlxError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::bigint FROM gateway_llm_cluster_model WHERE cluster_id = $1",
        )
        .bind(cluster_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub async fn list_llm_cluster_models(
        &self,
        cluster_id: &str,
    ) -> Result<Vec<GatewayLlmClusterModelRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT cluster_id, model_id, name, base_model_url, model_name, current_rev,
                      api_key_ciphertext, created_at_ms, updated_at_ms
               FROM gateway_llm_cluster_model
               WHERE cluster_id = $1
               ORDER BY created_at_ms ASC",
        )
        .bind(cluster_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(GatewayLlmClusterModelRow {
                    cluster_id: row.try_get("cluster_id")?,
                    model_id: row.try_get("model_id")?,
                    name: row.try_get("name")?,
                    base_model_url: row.try_get("base_model_url")?,
                    model_name: row.try_get("model_name")?,
                    current_rev: row.try_get("current_rev")?,
                    api_key_ciphertext: row.try_get("api_key_ciphertext")?,
                    created_at_ms: row.try_get("created_at_ms")?,
                    updated_at_ms: row.try_get("updated_at_ms")?,
                })
            })
            .collect()
    }

    pub async fn upsert_llm_cluster_model(
        &self,
        row: &GatewayLlmClusterModelRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_llm_cluster_model (
                 cluster_id, model_id, name, base_model_url, model_name, current_rev,
                 api_key_ciphertext, created_at_ms, updated_at_ms
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (cluster_id, model_id) DO UPDATE SET
                 name = EXCLUDED.name,
                 base_model_url = EXCLUDED.base_model_url,
                 model_name = EXCLUDED.model_name,
                 current_rev = EXCLUDED.current_rev,
                 api_key_ciphertext = EXCLUDED.api_key_ciphertext,
                 updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(&row.cluster_id)
        .bind(&row.model_id)
        .bind(&row.name)
        .bind(&row.base_model_url)
        .bind(&row.model_name)
        .bind(&row.current_rev)
        .bind(&row.api_key_ciphertext)
        .bind(row.created_at_ms)
        .bind(row.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_llm_cluster_model(
        &self,
        cluster_id: &str,
        model_id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM gateway_llm_cluster_model WHERE cluster_id = $1 AND model_id = $2",
        )
        .bind(cluster_id)
        .bind(model_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_llm_cluster_state(
        &self,
        cluster_id: &str,
    ) -> Result<Option<GatewayLlmClusterStateRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT cluster_id, active_model_id, active_model_rev, active_applied_at_ms, updated_at_ms
               FROM gateway_llm_cluster_state WHERE cluster_id = $1",
        )
        .bind(cluster_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| GatewayLlmClusterStateRow {
            cluster_id: row.try_get("cluster_id").unwrap_or_default(),
            active_model_id: row.try_get("active_model_id").unwrap_or_default(),
            active_model_rev: row.try_get("active_model_rev").unwrap_or_default(),
            active_applied_at_ms: row.try_get("active_applied_at_ms").ok(),
            updated_at_ms: row.try_get("updated_at_ms").unwrap_or(0),
        }))
    }

    pub async fn save_llm_cluster_state(
        &self,
        cluster_id: &str,
        active_model_id: &str,
        active_model_rev: &str,
        active_applied_at_ms: Option<i64>,
        updated_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_llm_cluster_state (
                 cluster_id, active_model_id, active_model_rev, active_applied_at_ms, updated_at_ms
               ) VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (cluster_id) DO UPDATE SET
                 active_model_id = EXCLUDED.active_model_id,
                 active_model_rev = EXCLUDED.active_model_rev,
                 active_applied_at_ms = EXCLUDED.active_applied_at_ms,
                 updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(cluster_id)
        .bind(active_model_id)
        .bind(active_model_rev)
        .bind(active_applied_at_ms)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_llm_cluster_revision(
        &self,
        cluster_id: &str,
        model_id: &str,
        model_rev: &str,
    ) -> Result<Option<GatewayLlmModelRevisionRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT cluster_id, model_id, model_rev, created_at_ms, name, base_model_url, model_name, note
               FROM gateway_llm_cluster_revision
               WHERE cluster_id = $1 AND model_id = $2 AND model_rev = $3",
        )
        .bind(cluster_id)
        .bind(model_id)
        .bind(model_rev)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| GatewayLlmModelRevisionRow {
            cluster_id: row.try_get("cluster_id").unwrap_or_default(),
            model_id: row.try_get("model_id").unwrap_or_default(),
            model_rev: row.try_get("model_rev").unwrap_or_default(),
            created_at_ms: row.try_get("created_at_ms").unwrap_or(0),
            name: row.try_get("name").unwrap_or_default(),
            base_model_url: row.try_get("base_model_url").unwrap_or_default(),
            model_name: row.try_get("model_name").unwrap_or_default(),
            note: row.try_get("note").ok(),
        }))
    }

    pub async fn upsert_llm_cluster_revision(
        &self,
        row: &GatewayLlmModelRevisionRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_llm_cluster_revision (
                 cluster_id, model_id, model_rev, created_at_ms, name, base_model_url, model_name, note
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (cluster_id, model_id, model_rev) DO UPDATE SET
                 name = EXCLUDED.name,
                 base_model_url = EXCLUDED.base_model_url,
                 model_name = EXCLUDED.model_name,
                 note = EXCLUDED.note,
                 created_at_ms = EXCLUDED.created_at_ms",
        )
        .bind(&row.cluster_id)
        .bind(&row.model_id)
        .bind(&row.model_rev)
        .bind(row.created_at_ms)
        .bind(&row.name)
        .bind(&row.base_model_url)
        .bind(&row.model_name)
        .bind(&row.note)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_llm_cluster_revisions(
        &self,
        cluster_id: &str,
        model_id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM gateway_llm_cluster_revision WHERE cluster_id = $1 AND model_id = $2",
        )
        .bind(cluster_id)
        .bind(model_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_llm_cluster_all(&self, cluster_id: &str) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM gateway_llm_cluster_revision WHERE cluster_id = $1")
            .bind(cluster_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM gateway_llm_cluster_model WHERE cluster_id = $1")
            .bind(cluster_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM gateway_llm_cluster_state WHERE cluster_id = $1")
            .bind(cluster_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn save_gateway_global_settings_raw(
        &self,
        settings_json: &Value,
        git_pat_tokens_json: &Value,
        updated_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_global_settings (cluster_id, settings_json, git_pat_tokens_json, updated_at_ms)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (cluster_id) DO UPDATE SET
                 settings_json = EXCLUDED.settings_json,
                 git_pat_tokens_json = EXCLUDED.git_pat_tokens_json,
                 updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(self.cluster_id())
        .bind(Json(settings_json))
        .bind(Json(git_pat_tokens_json))
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_preflight_plugins(
        &self,
    ) -> Result<Vec<preflight_spi::PreflightPluginRecord>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT plugin_id, display_name, spi_version, default_impl, config_schema
             FROM preflight_plugin ORDER BY plugin_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let plugin_id: String = row.try_get("plugin_id")?;
            let display_name: String = row.try_get("display_name")?;
            let spi_version: String = row.try_get("spi_version")?;
            let default_impl: Option<Value> = row
                .try_get::<Option<Json<Value>>, _>("default_impl")?
                .map(|j| j.0);
            let config_schema: Value = row.try_get::<Json<Value>, _>("config_schema")?.0;
            let default_impl = default_impl.and_then(|v| serde_json::from_value(v).ok());
            out.push(preflight_spi::PreflightPluginRecord {
                plugin_id,
                display_name,
                spi_version,
                default_impl,
                config_schema,
            });
        }
        Ok(out)
    }

    pub async fn list_preflight_plugin_ids(&self) -> Result<Vec<String>, SqlxError> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT plugin_id FROM preflight_plugin ORDER BY plugin_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn upsert_preflight_plugin(
        &self,
        record: &preflight_spi::PreflightPluginRecord,
        updated_at_ms: i64,
    ) -> Result<(), SqlxError> {
        let default_impl = record
            .default_impl
            .as_ref()
            .and_then(|v| serde_json::to_value(v).ok());
        sqlx::query(
            r"INSERT INTO preflight_plugin (plugin_id, display_name, spi_version, default_impl, config_schema, updated_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (plugin_id) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               spi_version = EXCLUDED.spi_version,
               default_impl = EXCLUDED.default_impl,
               config_schema = EXCLUDED.config_schema,
               updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(&record.plugin_id)
        .bind(&record.display_name)
        .bind(&record.spi_version)
        .bind(default_impl.map(Json))
        .bind(Json(record.config_schema.clone()))
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_project_config_proj_ids(&self) -> Result<Vec<i64>, SqlxError> {
        let rows = sqlx::query_scalar::<_, i64>(
            "SELECT proj_id FROM project_config WHERE cluster_id = $1 ORDER BY proj_id",
        )
        .bind(self.cluster_id())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Admin list: one row per `project_config` (DB truth for skills / CLAUDE). Author: kejiqing
    pub async fn list_project_config_summaries(
        &self,
    ) -> Result<Vec<ProjectConfigSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT proj_id, content_rev, stable_content_rev, draft_open, updated_at_ms, claude_md,
                      skills_json, rules_json, mcp_servers_json, git_sync_json
               FROM project_config WHERE cluster_id = $1 ORDER BY proj_id",
        )
        .bind(self.cluster_id())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let proj_id: i64 = row.try_get("proj_id")?;
            let content_rev: String = row.try_get("content_rev")?;
            let stable_content_rev: Option<String> = row.try_get("stable_content_rev")?;
            let draft_open: bool = row.try_get("draft_open")?;
            let updated_at_ms: i64 = row.try_get("updated_at_ms")?;
            let claude_md: Option<String> = row.try_get("claude_md")?;
            let skills_json: Value = row.try_get::<Json<Value>, _>("skills_json")?.0;
            let rules_json: Value = row.try_get::<Json<Value>, _>("rules_json")?.0;
            let mcp_servers_json: Value = row.try_get::<Json<Value>, _>("mcp_servers_json")?.0;
            let git_sync_json: Value = row.try_get::<Json<Value>, _>("git_sync_json")?.0;
            let claude_in_db = claude_md.as_deref().is_some_and(|s| !s.trim().is_empty());
            let skills_count_db = skills_json
                .as_array()
                .map_or(0, |a| i64::try_from(a.len()).unwrap_or(i64::MAX));
            let rules_count_db = rules_json
                .as_array()
                .map_or(0, |a| i64::try_from(a.len()).unwrap_or(i64::MAX));
            let mcp_servers_count_db = mcp_servers_json
                .as_object()
                .map_or(0, |o| i64::try_from(o.len()).unwrap_or(i64::MAX));
            out.push(ProjectConfigSummary {
                proj_id,
                content_rev,
                stable_content_rev,
                draft_open,
                updated_at_ms,
                claude_in_db,
                skills_count_db,
                rules_count_db,
                mcp_servers_count_db,
                git_sync_json,
            });
        }
        Ok(out)
    }

    pub async fn get_project_config(
        &self,
        proj_id: i64,
    ) -> Result<Option<ProjectConfigRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT proj_id, content_rev, stable_content_rev, draft_open, updated_at_ms,
                      rules_json, mcp_servers_json, skills_sources_json, skills_json,
                      allowed_tools_json, claude_md, git_sync_json, solve_preflight_json,
                      solve_orchestration_json, language_pipeline_json, extra_session_fields_json,
                      prompt_limits_json, worker_profile_json
               FROM project_config WHERE cluster_id = $1 AND proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let proj_id: i64 = row.try_get("proj_id")?;
        let content_rev: String = row.try_get("content_rev")?;
        let updated_at_ms: i64 = row.try_get("updated_at_ms")?;
        let rules_json: Value = row.try_get::<Json<Value>, _>("rules_json")?.0;
        let mcp_servers_json: Value = row.try_get::<Json<Value>, _>("mcp_servers_json")?.0;
        let skills_sources_json: Value = row.try_get::<Json<Value>, _>("skills_sources_json")?.0;
        let skills_json: Value = row.try_get::<Json<Value>, _>("skills_json")?.0;
        let allowed_tools_json: Value = row.try_get::<Json<Value>, _>("allowed_tools_json")?.0;
        let claude_md: Option<String> = row.try_get("claude_md")?;
        let git_sync_json: Value = row.try_get::<Json<Value>, _>("git_sync_json")?.0;
        let solve_preflight_json: Value = row.try_get::<Json<Value>, _>("solve_preflight_json")?.0;
        let solve_orchestration_json: Value =
            row.try_get::<Json<Value>, _>("solve_orchestration_json")?.0;
        let language_pipeline_json: Value =
            row.try_get::<Json<Value>, _>("language_pipeline_json")?.0;
        let extra_session_fields_json: Value = row
            .try_get::<Json<Value>, _>("extra_session_fields_json")?
            .0;
        let prompt_limits_json: Value = row.try_get::<Json<Value>, _>("prompt_limits_json")?.0;
        let worker_profile_json: Value = row.try_get::<Json<Value>, _>("worker_profile_json")?.0;

        let stable_content_rev: Option<String> = row.try_get("stable_content_rev")?;
        let draft_open: bool = row.try_get("draft_open")?;

        Ok(Some(ProjectConfigRow {
            proj_id,
            content_rev,
            stable_content_rev,
            draft_open,
            updated_at_ms,
            rules_json,
            mcp_servers_json,
            skills_sources_json,
            skills_json,
            allowed_tools_json,
            claude_md,
            git_sync_json,
            solve_preflight_json,
            solve_orchestration_json,
            language_pipeline_json,
            extra_session_fields_json,
            prompt_limits_json,
            worker_profile_json,
        }))
    }

    /// Sidecar for pool acquire: per-ds worker strict/relaxed profile. Author: kejiqing
    pub async fn get_worker_profile_json(&self, proj_id: i64) -> Result<Value, SqlxError> {
        let row: Option<Json<Value>> = sqlx::query_scalar(
            "SELECT worker_profile_json FROM project_config WHERE cluster_id = $1 AND proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|j| j.0)
            .unwrap_or_else(crate::pool::default_worker_profile_json))
    }

    /// Persisted e2b worker sandbox for a project slot (gateway-managed lifecycle). Author: kejiqing
    pub async fn get_project_e2b_worker(
        &self,
        proj_id: i64,
        slot_index: i32,
    ) -> Result<Option<ProjectFcWorkerRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT proj_id, slot_index, sandbox_id, worker_id, template_id, handle_json, updated_at_ms
               FROM project_e2b_worker
               WHERE cluster_id = $1 AND proj_id = $2 AND slot_index = $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(slot_index)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(row_to_project_fc_worker(&row)?))
    }

    /// First slot (legacy callers).
    pub async fn get_project_e2b_worker_slot0(
        &self,
        proj_id: i64,
    ) -> Result<Option<ProjectFcWorkerRow>, SqlxError> {
        self.get_project_e2b_worker(proj_id, 0).await
    }

    pub async fn list_project_e2b_workers(
        &self,
        proj_id: i64,
    ) -> Result<Vec<ProjectFcWorkerRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT proj_id, slot_index, sandbox_id, worker_id, template_id, handle_json, updated_at_ms
               FROM project_e2b_worker
               WHERE cluster_id = $1 AND proj_id = $2
               ORDER BY slot_index ASC",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_project_fc_worker).collect()
    }

    pub async fn upsert_project_e2b_worker(
        &self,
        row: &ProjectFcWorkerRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO project_e2b_worker (
                 proj_id, cluster_id, slot_index, sandbox_id, worker_id, template_id, handle_json, updated_at_ms
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (cluster_id, proj_id, slot_index) DO UPDATE SET
                 sandbox_id = EXCLUDED.sandbox_id,
                 worker_id = EXCLUDED.worker_id,
                 template_id = EXCLUDED.template_id,
                 handle_json = EXCLUDED.handle_json,
                 updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(row.proj_id)
        .bind(self.cluster_id())
        .bind(row.slot_index)
        .bind(&row.sandbox_id)
        .bind(&row.worker_id)
        .bind(&row.template_id)
        .bind(Json(&row.handle_json))
        .bind(row.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_project_e2b_worker_slot(
        &self,
        proj_id: i64,
        slot_index: i32,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM project_e2b_worker WHERE cluster_id = $1 AND proj_id = $2 AND slot_index = $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(slot_index)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_project_e2b_workers_above_slot(
        &self,
        proj_id: i64,
        max_slot_exclusive: i32,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM project_e2b_worker WHERE cluster_id = $1 AND proj_id = $2 AND slot_index >= $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(max_slot_exclusive)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete all slots for a project.
    pub async fn delete_project_e2b_worker(&self, proj_id: i64) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM project_e2b_worker WHERE cluster_id = $1 AND proj_id = $2")
            .bind(self.cluster_id())
            .bind(proj_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_project_e2b_worker_sandbox_ids(&self) -> Result<Vec<String>, SqlxError> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT sandbox_id FROM project_e2b_worker WHERE cluster_id = $1 ORDER BY proj_id",
        )
        .bind(self.cluster_id())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Append one worker rotation audit event (history only; never updated/deleted). Author: kejiqing
    pub async fn insert_worker_rotation_event(
        &self,
        event: &WorkerRotationEvent,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO worker_rotation_log (
                 proj_id, cluster_id, event, sandbox_id, worker_id, template_id, reason, at_ms
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(event.proj_id)
        .bind(self.cluster_id())
        .bind(&event.event)
        .bind(event.sandbox_id.as_deref())
        .bind(event.worker_id.as_deref())
        .bind(event.template_id.as_deref())
        .bind(event.reason.as_deref())
        .bind(event.at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Recent worker rotation events for a project (newest first). Author: kejiqing
    pub async fn list_worker_rotation_log(
        &self,
        proj_id: i64,
        limit: i64,
    ) -> Result<Vec<WorkerRotationEvent>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT proj_id, event, sandbox_id, worker_id, template_id, reason, at_ms
               FROM worker_rotation_log
               WHERE cluster_id = $1 AND proj_id = $2
               ORDER BY at_ms DESC, id DESC
               LIMIT $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(WorkerRotationEvent {
                    proj_id: row.try_get("proj_id")?,
                    event: row.try_get("event")?,
                    sandbox_id: row.try_get("sandbox_id")?,
                    worker_id: row.try_get("worker_id")?,
                    template_id: row.try_get("template_id")?,
                    reason: row.try_get("reason")?,
                    at_ms: row.try_get("at_ms")?,
                })
            })
            .collect()
    }

    pub async fn upsert_project_config(
        &self,
        row: ProjectConfigUpsert<'_>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO project_config (
                ds_id, proj_id, cluster_id, content_rev, stable_content_rev, draft_open, updated_at_ms,
                rules_json, mcp_servers_json, skills_sources_json, skills_json,
                allowed_tools_json, claude_md, git_sync_json, solve_preflight_json,
                solve_orchestration_json, language_pipeline_json, extra_session_fields_json,
                prompt_limits_json, worker_profile_json
            ) VALUES ($1, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
            ON CONFLICT (cluster_id, proj_id) DO UPDATE SET
                ds_id = EXCLUDED.ds_id,
                content_rev = EXCLUDED.content_rev,
                stable_content_rev = EXCLUDED.stable_content_rev,
                draft_open = EXCLUDED.draft_open,
                updated_at_ms = EXCLUDED.updated_at_ms,
                rules_json = EXCLUDED.rules_json,
                mcp_servers_json = EXCLUDED.mcp_servers_json,
                skills_sources_json = EXCLUDED.skills_sources_json,
                skills_json = EXCLUDED.skills_json,
                allowed_tools_json = EXCLUDED.allowed_tools_json,
                claude_md = EXCLUDED.claude_md,
                git_sync_json = EXCLUDED.git_sync_json,
                solve_preflight_json = EXCLUDED.solve_preflight_json,
                solve_orchestration_json = EXCLUDED.solve_orchestration_json,
                language_pipeline_json = EXCLUDED.language_pipeline_json,
                extra_session_fields_json = EXCLUDED.extra_session_fields_json,
                prompt_limits_json = EXCLUDED.prompt_limits_json,
                worker_profile_json = EXCLUDED.worker_profile_json",
        )
        .bind(row.proj_id)
        .bind(self.cluster_id())
        .bind(row.content_rev)
        .bind(row.stable_content_rev)
        .bind(row.draft_open)
        .bind(row.updated_at_ms)
        .bind(Json(row.rules_json))
        .bind(Json(row.mcp_servers_json))
        .bind(Json(row.skills_sources_json))
        .bind(Json(row.skills_json))
        .bind(Json(row.allowed_tools_json))
        .bind(row.claude_md)
        .bind(Json(row.git_sync_json))
        .bind(Json(row.solve_preflight_json))
        .bind(Json(row.solve_orchestration_json))
        .bind(Json(row.language_pipeline_json))
        .bind(Json(row.extra_session_fields_json))
        .bind(Json(row.prompt_limits_json))
        .bind(Json(row.worker_profile_json))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert saved revision; existing rows are not updated (immutable). Author: kejiqing
    pub async fn insert_project_config_revision_immutable(
        &self,
        row: &ProjectConfigRevisionRow,
    ) -> Result<bool, SqlxError> {
        let r = sqlx::query(
            r"INSERT INTO project_config_revision (
                ds_id, proj_id, cluster_id, content_rev, created_at_ms, note, rules_json, mcp_servers_json,
                skills_sources_json, skills_json, allowed_tools_json, claude_md
            ) VALUES ($1, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (ds_id, content_rev) DO NOTHING",
        )
        .bind(row.proj_id)
        .bind(self.cluster_id())
        .bind(&row.content_rev)
        .bind(row.created_at_ms)
        .bind(&row.note)
        .bind(Json(&row.rules_json))
        .bind(Json(&row.mcp_servers_json))
        .bind(Json(&row.skills_sources_json))
        .bind(Json(&row.skills_json))
        .bind(Json(&row.allowed_tools_json))
        .bind(&row.claude_md)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_project_config_revision(
        &self,
        proj_id: i64,
        content_rev: &str,
    ) -> Result<Option<ProjectConfigRevisionRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT proj_id, content_rev, created_at_ms, note, rules_json, mcp_servers_json,
                      skills_sources_json, skills_json, allowed_tools_json, claude_md
               FROM project_config_revision
               WHERE cluster_id = $1 AND proj_id = $2 AND content_rev = $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(content_rev)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ProjectConfigRevisionRow {
            proj_id: row.try_get("proj_id")?,
            content_rev: row.try_get("content_rev")?,
            created_at_ms: row.try_get("created_at_ms")?,
            note: row.try_get("note")?,
            rules_json: row.try_get::<Json<Value>, _>("rules_json")?.0,
            mcp_servers_json: row.try_get::<Json<Value>, _>("mcp_servers_json")?.0,
            skills_sources_json: row.try_get::<Json<Value>, _>("skills_sources_json")?.0,
            skills_json: row.try_get::<Json<Value>, _>("skills_json")?.0,
            allowed_tools_json: row.try_get::<Json<Value>, _>("allowed_tools_json")?.0,
            claude_md: row.try_get("claude_md")?,
        }))
    }

    pub async fn list_project_config_revisions(
        &self,
        proj_id: i64,
    ) -> Result<Vec<ProjectConfigRevisionSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT content_rev, created_at_ms, note, claude_md, skills_json, rules_json, mcp_servers_json
               FROM project_config_revision
               WHERE cluster_id = $1 AND proj_id = $2
               ORDER BY created_at_ms DESC, content_rev DESC",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let claude_md: Option<String> = row.try_get("claude_md")?;
            let skills_json: Value = row.try_get::<Json<Value>, _>("skills_json")?.0;
            let rules_json: Value = row.try_get::<Json<Value>, _>("rules_json")?.0;
            let mcp_servers_json: Value = row.try_get::<Json<Value>, _>("mcp_servers_json")?.0;
            out.push(ProjectConfigRevisionSummary {
                content_rev: row.try_get("content_rev")?,
                created_at_ms: row.try_get("created_at_ms")?,
                note: row.try_get("note")?,
                claude_in_db: claude_md.as_deref().is_some_and(|s| !s.trim().is_empty()),
                skills_count_db: skills_json
                    .as_array()
                    .map_or(0, |a| i64::try_from(a.len()).unwrap_or(i64::MAX)),
                rules_count_db: rules_json
                    .as_array()
                    .map_or(0, |a| i64::try_from(a.len()).unwrap_or(i64::MAX)),
                mcp_servers_count_db: mcp_servers_json
                    .as_object()
                    .map_or(0, |o| i64::try_from(o.len()).unwrap_or(i64::MAX)),
            });
        }
        Ok(out)
    }

    /// Update remark on a formal revision (`note` only; config snapshot stays immutable). Author: kejiqing
    pub async fn update_project_config_revision_note(
        &self,
        proj_id: i64,
        content_rev: &str,
        note: Option<&str>,
    ) -> Result<bool, SqlxError> {
        let r = sqlx::query(
            "UPDATE project_config_revision SET note = $4 WHERE cluster_id = $1 AND proj_id = $2 AND content_rev = $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(content_rev)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Drop one saved revision (not the effective stable rev). Author: kejiqing
    pub async fn delete_project_config_revision(
        &self,
        proj_id: i64,
        content_rev: &str,
    ) -> Result<bool, SqlxError> {
        let r = sqlx::query(
            "DELETE FROM project_config_revision WHERE cluster_id = $1 AND proj_id = $2 AND content_rev = $3",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(content_rev)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_project_config_revisions(&self, proj_id: i64) -> Result<u64, SqlxError> {
        let r = sqlx::query(
            "DELETE FROM project_config_revision WHERE cluster_id = $1 AND proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    pub async fn insert_project_entity_revision_immutable(
        &self,
        row: &ProjectEntityRevisionRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO project_entity_revision (
                ds_id, proj_id, cluster_id, domain, entity_key, entity_rev, created_at_ms, note, body
            ) VALUES ($1, $1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (ds_id, domain, entity_key, entity_rev) DO NOTHING",
        )
        .bind(row.proj_id)
        .bind(self.cluster_id())
        .bind(&row.domain)
        .bind(&row.entity_key)
        .bind(&row.entity_rev)
        .bind(row.created_at_ms)
        .bind(&row.note)
        .bind(Json(&row.body))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_project_entity_revision(
        &self,
        proj_id: i64,
        domain: &str,
        entity_key: &str,
        entity_rev: &str,
    ) -> Result<Option<ProjectEntityRevisionRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT proj_id, domain, entity_key, entity_rev, created_at_ms, note, body
               FROM project_entity_revision
               WHERE cluster_id = $1 AND proj_id = $2 AND domain = $3 AND entity_key = $4 AND entity_rev = $5",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(domain)
        .bind(entity_key)
        .bind(entity_rev)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ProjectEntityRevisionRow {
            proj_id: row.try_get("proj_id")?,
            domain: row.try_get("domain")?,
            entity_key: row.try_get("entity_key")?,
            entity_rev: row.try_get("entity_rev")?,
            created_at_ms: row.try_get("created_at_ms")?,
            note: row.try_get("note")?,
            body: row.try_get::<Json<Value>, _>("body")?.0,
        }))
    }

    pub async fn list_project_entity_revisions(
        &self,
        proj_id: i64,
        domain: &str,
        entity_key: &str,
    ) -> Result<Vec<ProjectEntityRevisionSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT entity_rev, created_at_ms, note
               FROM project_entity_revision
               WHERE cluster_id = $1 AND proj_id = $2 AND domain = $3 AND entity_key = $4
               ORDER BY created_at_ms DESC, entity_rev DESC",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .bind(domain)
        .bind(entity_key)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(ProjectEntityRevisionSummary {
                entity_rev: row.try_get("entity_rev")?,
                created_at_ms: row.try_get("created_at_ms")?,
                note: row.try_get("note")?,
            });
        }
        Ok(out)
    }

    pub async fn delete_project_entity_revisions(&self, proj_id: i64) -> Result<u64, SqlxError> {
        let r = sqlx::query(
            "DELETE FROM project_entity_revision WHERE cluster_id = $1 AND proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(proj_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Remove `project_config` row for a ds (project delete). Author: kejiqing
    pub async fn delete_project_config(&self, proj_id: i64) -> Result<bool, SqlxError> {
        let _ = self.delete_project_config_revisions(proj_id).await?;
        let _ = self.delete_project_entity_revisions(proj_id).await?;
        let r = sqlx::query("DELETE FROM project_config WHERE cluster_id = $1 AND proj_id = $2")
            .bind(self.cluster_id())
            .bind(proj_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Delete all sessions and turns for a ds (optional on project delete). Author: kejiqing
    pub async fn delete_sessions_for_proj(&self, proj_id: i64) -> Result<u64, SqlxError> {
        sqlx::query("DELETE FROM gateway_turns WHERE cluster_id = $1 AND proj_id = $2")
            .bind(self.cluster_id())
            .bind(proj_id)
            .execute(&self.pool)
            .await?;
        let r = sqlx::query("DELETE FROM gateway_sessions WHERE cluster_id = $1 AND proj_id = $2")
            .bind(self.cluster_id())
            .bind(proj_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn get_session_home_rel(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, String>(
            "SELECT session_home FROM gateway_sessions WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn insert_session(
        &self,
        session_id: &str,
        proj_id: i64,
        session_home_rel: &str,
        now_ms: i64,
        client_origin: Option<&str>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_sessions (session_id, ds_id, proj_id, cluster_id, session_home, created_at_ms, updated_at_ms, client_origin)
              VALUES ($1, $2, $2, $3, $4, $5, $6, $7)",
        )
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(session_home_rel)
        .bind(now_ms)
        .bind(now_ms)
        .bind(client_origin)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn touch_updated(
        &self,
        session_id: &str,
        proj_id: i64,
        now_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE gateway_sessions SET updated_at_ms = $1 WHERE cluster_id = $2 AND session_id = $3 AND proj_id = $4",
        )
        .bind(now_ms)
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_exists(&self, session_id: &str, proj_id: i64) -> Result<bool, SqlxError> {
        let row: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM gateway_sessions WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3 LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Reject **new** turn enqueue when another turn is inflight or prior succeeded lacks artifacts.
    /// Author: kejiqing
    /// Whether this turn already has pool readback committed (`artifacts_ready`). Author: kejiqing
    pub async fn turn_artifacts_ready(&self, turn_id: &str) -> Result<bool, SqlxError> {
        let row: Option<(bool,)> =
            sqlx::query_as("SELECT artifacts_ready FROM gateway_turns WHERE turn_id = $1")
                .bind(turn_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(v,)| v).unwrap_or(false))
    }

    pub async fn assert_session_can_enqueue(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<(), String> {
        self.assert_session_enqueue_gate(session_id, proj_id, None)
            .await
    }

    /// Pool `acquire` for an **already-enqueued** turn: same gate but ignore this `turn_id`
    /// (it is already `queued`/`running` on this worker). Author: kejiqing
    pub async fn assert_session_can_acquire_for_turn(
        &self,
        session_id: &str,
        proj_id: i64,
        active_turn_id: &str,
    ) -> Result<(), String> {
        self.assert_session_enqueue_gate(session_id, proj_id, Some(active_turn_id))
            .await
    }

    async fn assert_session_enqueue_gate(
        &self,
        session_id: &str,
        proj_id: i64,
        exclude_turn_id: Option<&str>,
    ) -> Result<(), String> {
        let inflight: bool = if let Some(tid) = exclude_turn_id {
            sqlx::query_scalar(
                r"SELECT EXISTS(
                    SELECT 1 FROM gateway_turns
                    WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3
                      AND status IN ('queued', 'running')
                      AND turn_id <> $4
                  )",
            )
            .bind(self.cluster_id())
            .bind(session_id)
            .bind(proj_id)
            .bind(tid)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())?
        } else {
            sqlx::query_scalar(
                r"SELECT EXISTS(
                    SELECT 1 FROM gateway_turns
                    WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3
                      AND status IN ('queued', 'running')
                  )",
            )
            .bind(self.cluster_id())
            .bind(session_id)
            .bind(proj_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())?
        };
        if inflight {
            return Err("inflight".into());
        }
        let blocked: bool = sqlx::query_scalar(
            r"SELECT EXISTS(
                SELECT 1 FROM gateway_turns
                WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3
                  AND status = 'succeeded' AND artifacts_ready = FALSE
              )",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        if blocked {
            return Err("artifacts_not_ready".into());
        }
        Ok(())
    }

    pub async fn upsert_solve_task_json(
        &self,
        turn_id: &str,
        task: &Value,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET solve_task_json = $2 WHERE turn_id = $1")
            .bind(turn_id)
            .bind(Json(task))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_solve_task_json(&self, turn_id: &str) -> Result<Option<Value>, SqlxError> {
        let row: Option<(Option<Json<Value>>,)> =
            sqlx::query_as("SELECT solve_task_json FROM gateway_turns WHERE turn_id = $1")
                .bind(turn_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(j,)| j.map(|v| v.0)))
    }

    pub fn get_turn_runtime_settings_json(
        &self,
        _turn_id: &str,
    ) -> Result<Option<Value>, SqlxError> {
        Ok(None)
    }

    pub async fn get_turn_solve_timing_json(
        &self,
        turn_id: &str,
    ) -> Result<Option<Value>, SqlxError> {
        let row: Option<(Option<Value>,)> =
            sqlx::query_as("SELECT solve_timing_jsonb FROM gateway_turns WHERE turn_id = $1")
                .bind(turn_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(v,)| v))
    }

    fn empty_turn_timing_store() -> Value {
        serde_json::json!({
            "solveTimingEvents": [],
            "orchestrationEvents": [],
            "progressEvents": [],
            "taskProgress": null
        })
    }

    /// Worker container name while a pool turn is executing. Author: kejiqing
    pub async fn get_turn_worker_name(&self, turn_id: &str) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT worker_name FROM gateway_turns WHERE turn_id = $1",
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await
        .map(|opt| opt.flatten())
    }

    /// `podman exec --user` recorded at pool acquire. Author: kejiqing
    pub async fn get_turn_worker_exec_user(
        &self,
        turn_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT worker_exec_user FROM gateway_turns WHERE turn_id = $1",
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await
        .map(|opt| opt.flatten())
    }

    #[must_use]
    pub fn progress_events_from_timing_store(
        store: &serde_json::Value,
        limit: usize,
    ) -> Vec<gateway_solve_turn::ProgressEvent> {
        let Some(arr) = store
            .get("progressEvents")
            .and_then(serde_json::Value::as_array)
        else {
            return Vec::new();
        };
        let mut out: Vec<gateway_solve_turn::ProgressEvent> = arr
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        if out.len() > limit {
            out = out.split_off(out.len() - limit);
        }
        out
    }

    #[must_use]
    pub fn task_progress_from_timing_store(
        store: &serde_json::Value,
    ) -> Option<gateway_solve_turn::TaskProgressFile> {
        store
            .get("taskProgress")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Replace worker progress snapshot in `solve_timing_jsonb` (pool tmpfs → PG). Author: kejiqing
    pub async fn replace_turn_progress_snapshot(
        &self,
        turn_id: &str,
        progress_ndjson: &str,
        task_progress_json: &str,
    ) -> Result<(), SqlxError> {
        const LIMIT: usize = 500;
        let mut store = self
            .get_turn_solve_timing_json(turn_id)
            .await?
            .unwrap_or_else(Self::empty_turn_timing_store);
        let mut events = Vec::new();
        for line in progress_ndjson.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<gateway_solve_turn::ProgressEvent>(line) {
                events.push(v);
            }
        }
        if events.len() > LIMIT {
            events = events.split_off(events.len() - LIMIT);
        }
        store["progressEvents"] = serde_json::json!(events);
        if task_progress_json.trim().is_empty() {
            store["taskProgress"] = serde_json::Value::Null;
        } else if let Ok(v) = serde_json::from_str::<serde_json::Value>(task_progress_json) {
            store["taskProgress"] = v;
        }
        self.upsert_turn_timing_json(turn_id, &store).await
    }

    /// Append one bootstrap milestone to `solve_timing_jsonb`. Author: kejiqing
    pub async fn append_turn_solve_timing_bootstrap(
        &self,
        turn_id: &str,
        kind: &str,
    ) -> Result<(), SqlxError> {
        let mut store = self
            .get_turn_solve_timing_json(turn_id)
            .await?
            .unwrap_or_else(Self::empty_turn_timing_store);
        let event = serde_json::json!({
            "kind": kind,
            "tsMs": crate::persistence::transcript::now_ms(),
            "turnId": turn_id,
            "source": "bootstrap"
        });
        if let Some(arr) = store
            .get_mut("solveTimingEvents")
            .and_then(Value::as_array_mut)
        {
            arr.push(event);
        }
        self.upsert_turn_timing_json(turn_id, &store).await
    }

    fn append_ndjson_events(store: &mut Value, key: &str, ndjson: &str, limit: usize) {
        let Some(arr) = store.get_mut(key).and_then(Value::as_array_mut) else {
            return;
        };
        for line in ndjson.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                arr.push(v);
            }
        }
        if arr.len() > limit {
            let drop_n = arr.len() - limit;
            arr.drain(0..drop_n);
        }
    }

    /// Merge worker solve/orchestration NDJSON into `solve_timing_jsonb`. Progress uses [`Self::replace_turn_progress_snapshot`]. Author: kejiqing
    pub async fn merge_turn_timing_worker_readback(
        &self,
        turn_id: &str,
        solve_timing_ndjson: &str,
        orchestration_ndjson: &str,
    ) -> Result<(), SqlxError> {
        const LIMIT: usize = 500;
        let mut store = self
            .get_turn_solve_timing_json(turn_id)
            .await?
            .unwrap_or_else(Self::empty_turn_timing_store);
        Self::append_ndjson_events(&mut store, "solveTimingEvents", solve_timing_ndjson, LIMIT);
        Self::append_ndjson_events(
            &mut store,
            "orchestrationEvents",
            orchestration_ndjson,
            LIMIT,
        );
        self.upsert_turn_timing_json(turn_id, &store).await
    }

    pub async fn upsert_turn_timing_json(
        &self,
        turn_id: &str,
        timing: &Value,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET solve_timing_jsonb = $2 WHERE turn_id = $1")
            .bind(turn_id)
            .bind(Json(timing))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn render_session_jsonl(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<String, SqlxError> {
        let rows: Vec<(String, Json<Value>, Option<Json<Value>>)> = sqlx::query_as(
            r"SELECT m.role, m.blocks, m.usage
              FROM cc_messages m
              JOIN gateway_turns t ON m.turn_id = t.turn_id
              WHERE m.cluster_id = $1 AND m.session_id = $2 AND m.proj_id = $3
              ORDER BY t.created_at_ms ASC, m.seq ASC",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_all(&self.pool)
        .await?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut lines = vec![serde_json::json!({
            "type": "session_meta",
            "session_id": format!("session-{session_id}"),
            "version": 1,
            "created_at_ms": now,
            "updated_at_ms": now,
        })
        .to_string()];
        for (role, blocks, usage) in rows {
            let mut message = serde_json::json!({
                "role": role,
                "blocks": blocks.0,
            });
            if let Some(u) = usage {
                message["usage"] = u.0;
            }
            lines.push(
                serde_json::json!({
                    "type": "message",
                    "message": message,
                })
                .to_string(),
            );
        }
        let body = lines.join("\n");
        Ok(if body.is_empty() {
            body
        } else {
            format!("{body}\n")
        })
    }

    /// True when PG-rendered jsonl includes at least one `type: message` line (not just `session_meta`).
    #[must_use]
    pub fn session_jsonl_has_messages(body: &str) -> bool {
        body.lines().filter(|l| !l.trim().is_empty()).any(|line| {
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => v.get("type").and_then(|t| t.as_str()) == Some("message"),
                Err(_) => false,
            }
        })
    }

    /// Store gzip-tar snapshot of `/claw_host_root` (base64) for pool readback. Author: kejiqing
    pub async fn upsert_workspace_tar_b64(
        &self,
        session_id: &str,
        proj_id: i64,
        turn_id: &str,
        tar_path: &str,
        tar_kind: &str,
        tar_b64: &str,
        raw_tar_bytes: usize,
        created_at_ms: i64,
    ) -> Result<(), SqlxError> {
        let size_bytes = i64::try_from(raw_tar_bytes).unwrap_or(i64::MAX);
        let artifact_id = uuid::Uuid::new_v4();
        sqlx::query(
            r"INSERT INTO gateway_session_artifacts
                (artifact_id, session_id, ds_id, proj_id, cluster_id, turn_id, kind, relative_path, content, size_bytes, created_at_ms)
              VALUES ($1, $2, $3, $3, $4, $5, $6, $7, $8, $9, $10)
              ON CONFLICT (session_id, ds_id, turn_id, relative_path) DO UPDATE SET
                kind = EXCLUDED.kind,
                content = EXCLUDED.content,
                size_bytes = EXCLUDED.size_bytes,
                created_at_ms = EXCLUDED.created_at_ms",
        )
        .bind(artifact_id)
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(tar_kind)
        .bind(tar_path)
        .bind(tar_b64)
        .bind(size_bytes)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Latest ready workspace tar (base64) for `materialize_in`. Author: kejiqing
    pub async fn get_latest_workspace_tar_b64(
        &self,
        session_id: &str,
        proj_id: i64,
        tar_path: &str,
        tar_kind: &str,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            r"SELECT a.content
              FROM gateway_session_artifacts a
              INNER JOIN gateway_turns t ON t.turn_id = a.turn_id
              WHERE a.cluster_id = $1 AND a.session_id = $2 AND a.proj_id = $3
                AND a.kind = $4 AND a.relative_path = $5
                AND a.content IS NOT NULL AND t.artifacts_ready = TRUE
              ORDER BY COALESCE(t.finished_at_ms, t.created_at_ms) DESC, t.turn_id DESC
              LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .bind(tar_kind)
        .bind(tar_path)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(c,)| c))
    }

    pub async fn delete_messages_for_turn(&self, turn_id: &str) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM cc_messages WHERE turn_id = $1")
            .bind(turn_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove all transcript rows for a session (before full jsonl reconcile). Author: kejiqing
    pub async fn delete_messages_for_session(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM cc_messages WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ensure_runtime_iteration(
        &self,
        turn_id: &str,
        iteration_index: i32,
        started_at_ms: i64,
    ) -> Result<uuid::Uuid, SqlxError> {
        let existing: Option<(uuid::Uuid,)> = sqlx::query_as(
            "SELECT iteration_id FROM gateway_runtime_iterations WHERE turn_id = $1 AND iteration_index = $2",
        )
        .bind(turn_id)
        .bind(iteration_index)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((id,)) = existing {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4();
        sqlx::query(
            r"INSERT INTO gateway_runtime_iterations (iteration_id, turn_id, iteration_index, started_at_ms)
              VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(turn_id)
        .bind(iteration_index)
        .bind(started_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn insert_message(
        &self,
        session_id: &str,
        proj_id: i64,
        turn_id: &str,
        iteration_id: Option<uuid::Uuid>,
        seq: i32,
        role: &str,
        blocks: &Value,
        usage: Option<&Value>,
        created_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO cc_messages (session_id, ds_id, proj_id, cluster_id, turn_id, iteration_id, seq, role, blocks, usage, created_at_ms)
              VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(iteration_id)
        .bind(seq)
        .bind(role)
        .bind(Json(blocks))
        .bind(usage.map(Json))
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn finalize_turn_with_artifacts_ready(
        &self,
        turn_id: &str,
        status: &str,
        finished_at_ms: Option<i64>,
        claw_exit_code: i32,
        report_message: Option<&str>,
        output_json: Option<&Value>,
        artifacts_ready: bool,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"UPDATE gateway_turns SET
                status = $1,
                finished_at_ms = $2,
                report_message = $3,
                output_json = $4,
                claw_exit_code = $5,
                artifacts_ready = $6
              WHERE turn_id = $7",
        )
        .bind(status)
        .bind(finished_at_ms)
        .bind(report_message)
        .bind(output_json.map(Json))
        .bind(claw_exit_code)
        .bind(artifacts_ready)
        .bind(turn_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_turn(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
        status: &str,
        created_at_ms: i64,
        user_prompt: Option<&str>,
        client_origin: Option<&str>,
        entry_params_json: Option<&Value>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_turns (turn_id, session_id, ds_id, proj_id, cluster_id, status, created_at_ms, finished_at_ms, user_prompt, client_origin, entry_params_json)
              VALUES ($1, $2, $3, $3, $4, $5, $6, NULL, $7, $8, $9)",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(status)
        .bind(created_at_ms)
        .bind(user_prompt)
        .bind(client_origin)
        .bind(entry_params_json.map(Json))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_turn_client_origin(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT client_origin FROM gateway_turns WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4 LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(o,)| o))
    }

    /// Register or refresh a legacy `claw_pool` row. Author: kejiqing
    pub async fn upsert_claw_pool(&self, row: &ClawPoolUpsert<'_>) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO claw_pool (
                pool_id, cluster_id, registration_time_ms, slots_max, slots_min,
                advertise_ip, sse_port, gateway_base, last_heartbeat_ms
              ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
              ON CONFLICT (pool_id) DO UPDATE SET
                cluster_id = EXCLUDED.cluster_id,
                slots_max = EXCLUDED.slots_max,
                slots_min = EXCLUDED.slots_min,
                advertise_ip = EXCLUDED.advertise_ip,
                sse_port = EXCLUDED.sse_port,
                gateway_base = EXCLUDED.gateway_base,
                last_heartbeat_ms = EXCLUDED.last_heartbeat_ms",
        )
        .bind(row.pool_id)
        .bind(self.cluster_id())
        .bind(row.registration_time_ms)
        .bind(row.slots_max)
        .bind(row.slots_min)
        .bind(row.advertise_ip)
        .bind(row.sse_port)
        .bind(row.gateway_base)
        .bind(row.last_heartbeat_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn touch_claw_pool_heartbeat(
        &self,
        pool_id: &str,
        last_heartbeat_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE claw_pool SET last_heartbeat_ms = $2 WHERE cluster_id = $1 AND pool_id = $3",
        )
        .bind(self.cluster_id())
        .bind(last_heartbeat_ms)
        .bind(pool_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a pool row only when heartbeat is stale (offline). Author: kejiqing
    pub async fn delete_claw_pool_if_offline(
        &self,
        pool_id: &str,
        advertise_ip: &str,
        now_ms: i64,
    ) -> Result<bool, SqlxError> {
        let stale_before = now_ms.saturating_sub(120_000);
        let result = sqlx::query(
            "DELETE FROM claw_pool WHERE cluster_id = $1 AND pool_id = $2 AND advertise_ip = $3 AND last_heartbeat_ms < $4",
        )
        .bind(self.cluster_id())
        .bind(pool_id)
        .bind(advertise_ip)
        .bind(stale_before)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Admin: remove stale `claw_pool` row; pool-daemon re-registers on next start. Author: kejiqing
    pub async fn delete_claw_pool(&self, pool_id: &str) -> Result<bool, SqlxError> {
        let result = sqlx::query("DELETE FROM claw_pool WHERE cluster_id = $1 AND pool_id = $2")
            .bind(self.cluster_id())
            .bind(pool_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// All registered pool nodes (multi-host observability). Author: kejiqing
    pub async fn list_claw_pools(&self) -> Result<Vec<ClawPoolRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT pool_id, registration_time_ms, slots_max, slots_min,
                     advertise_ip, sse_port, gateway_base, last_heartbeat_ms
              FROM claw_pool
              WHERE cluster_id = $1
              ORDER BY last_heartbeat_ms DESC, pool_id ASC",
        )
        .bind(self.cluster_id())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(ClawPoolRow {
                pool_id: r.try_get("pool_id")?,
                registration_time_ms: r.try_get("registration_time_ms")?,
                slots_max: r.try_get("slots_max")?,
                slots_min: r.try_get("slots_min")?,
                advertise_ip: r.try_get("advertise_ip")?,
                sse_port: r.try_get("sse_port")?,
                gateway_base: r.try_get("gateway_base")?,
                last_heartbeat_ms: r.try_get("last_heartbeat_ms")?,
            });
        }
        Ok(out)
    }

    /// Pre-bind co-located pool at turn enqueue (before worker slot). Live SSE can JOIN `claw_pool`. Author: kejiqing
    pub async fn assign_turn_pool_id(&self, turn_id: &str, pool_id: &str) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET pool_id = $2 WHERE turn_id = $1")
            .bind(turn_id)
            .bind(pool_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Bind turn to executing pool + worker container name when exec starts. Author: kejiqing
    pub async fn assign_turn_pool_worker(
        &self,
        turn_id: &str,
        pool_id: &str,
        worker_name: &str,
        worker_exec_user: Option<&str>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE gateway_turns SET pool_id = $2, worker_name = $3, worker_exec_user = $4 WHERE turn_id = $1",
        )
        .bind(turn_id)
        .bind(pool_id)
        .bind(worker_name)
        .bind(worker_exec_user)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_turn_user_prompt(
        &self,
        turn_id: &str,
        user_prompt: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET user_prompt = $2 WHERE turn_id = $1")
            .bind(turn_id)
            .bind(user_prompt)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_project(
        &self,
        proj_id: i64,
        project_name: &str,
        workspace_rel: &str,
    ) -> Result<(), SqlxError> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            r"INSERT INTO gateway_projects (ds_id, proj_id, project_name, workspace_rel, created_at_ms, updated_at_ms)
              VALUES ($1, $1, $2, $3, $4, $4)
              ON CONFLICT (ds_id) DO UPDATE SET
                proj_id = EXCLUDED.proj_id,
                project_name = EXCLUDED.project_name,
                workspace_rel = EXCLUDED.workspace_rel,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(proj_id)
        .bind(project_name)
        .bind(workspace_rel)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn finish_turn(
        &self,
        turn_id: &str,
        claw_exit_code: i32,
        report_message: Option<&str>,
        output_json: Option<&Value>,
        has_report: bool,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"UPDATE gateway_turns SET
                claw_exit_code = $2,
                report_message = $3,
                output_json = $4,
                has_report = $5
              WHERE turn_id = $1",
        )
        .bind(turn_id)
        .bind(claw_exit_code)
        .bind(report_message)
        .bind(output_json.map(Json))
        .bind(has_report)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_turn_user_prompt(&self, turn_id: &str) -> Result<Option<String>, SqlxError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT user_prompt FROM gateway_turns WHERE turn_id = $1")
                .bind(turn_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(p,)| p))
    }

    pub async fn get_session_id_for_turn(
        &self,
        turn_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar("SELECT session_id FROM gateway_turns WHERE turn_id = $1 LIMIT 1")
            .bind(turn_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_turn_pool_id(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar(
            "SELECT pool_id FROM gateway_turns WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4 LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// `http://{advertise_ip}:{sse_port}` for live SSE proxy when turn has `pool_id`. Author: kejiqing
    pub async fn resolve_pool_http_base_for_turn(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(String, i32)> = sqlx::query_as(
            r"SELECT p.advertise_ip, p.sse_port
              FROM gateway_turns t
              JOIN claw_pool p ON t.pool_id = p.pool_id AND t.cluster_id = p.cluster_id
              WHERE t.cluster_id = $1 AND t.turn_id = $2 AND t.session_id = $3 AND t.proj_id = $4
              LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(ip, port)| format!("http://{ip}:{port}")))
    }

    /// Terminal (or running) status update; does not clear `report_message` / `output_json` unless
    /// [`Self::finalize_turn_terminal`] is used for terminal transitions that should set them.
    pub async fn update_turn_status(
        &self,
        turn_id: &str,
        status: &str,
        finished_at_ms: Option<i64>,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET status = $1, finished_at_ms = $2 WHERE turn_id = $3")
            .bind(status)
            .bind(finished_at_ms)
            .bind(turn_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Writes terminal solve outcome for one user turn (`T_*`). Safe to call for `failed` /
    /// `cancelled` with `report_message` / `output_json` / `claw_exit_code` all `None`.
    pub async fn finalize_turn_terminal(
        &self,
        turn_id: &str,
        status: &str,
        finished_at_ms: Option<i64>,
        report_message: Option<&str>,
        output_json: Option<&Value>,
        claw_exit_code: Option<i32>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"UPDATE gateway_turns SET
                status = $1,
                finished_at_ms = $2,
                report_message = $3,
                output_json = $4,
                claw_exit_code = $5
              WHERE turn_id = $6",
        )
        .bind(status)
        .bind(finished_at_ms)
        .bind(report_message)
        .bind(output_json.map(Json))
        .bind(claw_exit_code)
        .bind(turn_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Formal report body persisted for this turn (post-solve), if any.
    pub async fn get_turn_report_message(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, String>(
            r"SELECT report_message FROM gateway_turns
              WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4
                AND report_message IS NOT NULL AND btrim(report_message) <> ''",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// Terminal solve `output_json` snapshot for this turn, if persisted (`finalize_turn_terminal`).
    pub async fn get_turn_output_json(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<Value>, SqlxError> {
        let row = sqlx::query(
            r"SELECT output_json FROM gateway_turns
              WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4
                AND output_json IS NOT NULL",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = row else {
            return Ok(None);
        };
        r.try_get("output_json")
    }

    /// Session home + turn times + user turn index for `GET .../tools` (one query). Author: kejiqing
    pub async fn get_turn_tools_context(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<TurnToolsContext>, SqlxError> {
        let row = sqlx::query(
            r"SELECT s.session_home, t.created_at_ms, t.finished_at_ms,
                     (SELECT COUNT(*)::bigint FROM gateway_turns t2
                      WHERE t2.cluster_id = t.cluster_id AND t2.session_id = t.session_id AND t2.proj_id = t.proj_id
                        AND (t2.created_at_ms < t.created_at_ms
                             OR (t2.created_at_ms = t.created_at_ms AND t2.turn_id <= t.turn_id))
                     ) AS user_turn_index
              FROM gateway_turns t
              INNER JOIN gateway_sessions s
                ON s.cluster_id = t.cluster_id AND s.session_id = t.session_id AND s.proj_id = t.proj_id
              WHERE t.cluster_id = $1 AND t.turn_id = $2 AND t.session_id = $3 AND t.proj_id = $4",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        use sqlx::Row;
        Ok(Some(TurnToolsContext {
            session_home_rel: row.try_get("session_home")?,
            created_at_ms: row.try_get("created_at_ms")?,
            finished_at_ms: row.try_get("finished_at_ms")?,
            user_turn_index: row.try_get("user_turn_index")?,
        }))
    }

    /// `created_at_ms` for this turn (ordering within a session; tests / future callers).
    pub async fn get_turn_created_at_ms(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<i64>, SqlxError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT created_at_ms FROM gateway_turns WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// 1-based index of this turn among rows in `gateway_turns` for the same session, ordered by
    /// `(created_at_ms, turn_id)` (stable under concurrent inserts for disjoint `turn_id`s).
    pub async fn turn_index_in_session(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
        created_at_ms: i64,
    ) -> Result<i64, SqlxError> {
        let v: i64 = sqlx::query_scalar(
            r"SELECT COUNT(*)::bigint FROM gateway_turns
              WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3
                AND (created_at_ms < $4 OR (created_at_ms = $4 AND turn_id <= $5))",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .bind(created_at_ms)
        .bind(turn_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(v)
    }

    /// Latest turn row for a session (`task_id` == `session_id` in async solve). Used when the
    /// in-memory task map was lost (e.g. gateway restart).
    /// Marks every in-flight turn as **failed** (interrupted). Run once when this gateway
    /// process starts: rows `queued` / `running` cannot represent live work after a full process
    /// restart (no in-memory task or pool lease). Author: kejiqing
    pub async fn reconcile_interrupted_turns_on_startup(
        &self,
        now_ms: i64,
    ) -> Result<u64, SqlxError> {
        let detail = json!({
            "detail": "gateway restarted; in-flight turn was interrupted",
            "outcome": "aborted",
            "restartReconciled": true,
        });
        let r = sqlx::query(
            r"UPDATE gateway_turns SET
                status = 'failed',
                finished_at_ms = $1,
                report_message = NULL,
                output_json = $2,
                claw_exit_code = NULL
              WHERE cluster_id = $3 AND status IN ('queued', 'running')",
        )
        .bind(now_ms)
        .bind(detail)
        .bind(self.cluster_id())
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Escape `%` / `_` for SQL `LIKE` / `ILIKE` patterns. Author: kejiqing
    fn escape_like_pattern(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        for ch in raw.chars() {
            match ch {
                '%' | '_' | '\\' => {
                    out.push('\\');
                    out.push(ch);
                }
                other => out.push(other),
            }
        }
        out
    }

    /// `session_id_q`: full `T_<32 hex>` → exact turn match; otherwise session_id ILIKE substring.
    fn parse_session_list_id_filter(raw: Option<&str>) -> (Option<String>, Option<String>) {
        let Some(q) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return (None, None);
        };
        if let Some(rest) = q
            .strip_prefix(TURN_ID_PREFIX)
            .or_else(|| q.strip_prefix("t_"))
        {
            let candidate = format!("{TURN_ID_PREFIX}{}", rest.to_ascii_lowercase());
            if turn_id::validate_turn_id(&candidate) {
                return (None, Some(candidate));
            }
        }
        (Some(format!("%{}%", Self::escape_like_pattern(q))), None)
    }

    /// Recent sessions for admin chat history (keyset page + optional filters). Author: kejiqing
    pub async fn list_sessions_for_proj(
        &self,
        proj_id: i64,
        limit: i64,
        before_updated_at_ms: Option<i64>,
        before_session_id: Option<&str>,
        updated_from_ms: Option<i64>,
        updated_to_ms: Option<i64>,
        title_q: Option<&str>,
        session_id_q: Option<&str>,
        extra_session_filter: Option<&BTreeMap<String, String>>,
    ) -> Result<Vec<GatewaySessionSummary>, SqlxError> {
        let limit = limit.clamp(1, 100);
        let like_pat = title_q
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|q| format!("%{}%", Self::escape_like_pattern(q)));
        let (session_id_pat, turn_id_exact) = Self::parse_session_list_id_filter(session_id_q);

        let mut qb = QueryBuilder::new(
            r"SELECT s.session_id, s.created_at_ms, s.updated_at_ms, s.client_origin,
                     (SELECT COUNT(*)::bigint FROM gateway_turns t
                        WHERE t.cluster_id = s.cluster_id AND t.session_id = s.session_id AND t.proj_id = s.proj_id) AS turn_count,
                     (SELECT t.user_prompt FROM gateway_turns t
                        WHERE t.cluster_id = s.cluster_id AND t.session_id = s.session_id AND t.proj_id = s.proj_id
                        ORDER BY t.created_at_ms ASC, t.turn_id ASC
                        LIMIT 1) AS preview_prompt,
                     EXISTS (
                       SELECT 1 FROM gateway_feedback f
                       WHERE f.cluster_id = s.cluster_id AND f.session_id = s.session_id AND f.proj_id = s.proj_id
                         AND f.feedback = 'bad'
                     ) AS has_bad_feedback,
                     EXISTS (
                       SELECT 1 FROM gateway_feedback f
                       WHERE f.cluster_id = s.cluster_id AND f.session_id = s.session_id AND f.proj_id = s.proj_id
                         AND f.feedback = 'good'
                     ) AS has_good_feedback
              FROM gateway_sessions s
              WHERE s.cluster_id = ",
        );
        qb.push_bind(self.cluster_id());
        qb.push(" AND s.proj_id = ");
        qb.push_bind(proj_id);
        if let Some(from) = updated_from_ms {
            qb.push(" AND s.updated_at_ms >= ");
            qb.push_bind(from);
        }
        if let Some(to) = updated_to_ms {
            qb.push(" AND s.updated_at_ms <= ");
            qb.push_bind(to);
        }
        if let Some(ref pat) = like_pat {
            qb.push(
                " AND (
                    SELECT t.user_prompt FROM gateway_turns t
                      WHERE t.cluster_id = s.cluster_id AND t.session_id = s.session_id AND t.proj_id = s.proj_id
                      ORDER BY t.created_at_ms ASC, t.turn_id ASC
                      LIMIT 1
                  ) ILIKE ",
            );
            qb.push_bind(pat);
            qb.push(" ESCAPE '\\'");
        }
        if let Some(before_ms) = before_updated_at_ms {
            qb.push(" AND (s.updated_at_ms < ");
            qb.push_bind(before_ms);
            qb.push(" OR (s.updated_at_ms = ");
            qb.push_bind(before_ms);
            qb.push(" AND s.session_id < ");
            qb.push_bind(before_session_id.unwrap_or(""));
            qb.push("))");
        }
        if session_id_pat.is_some() || turn_id_exact.is_some() {
            qb.push(" AND (");
            let mut id_or = false;
            if let Some(ref pat) = session_id_pat {
                qb.push("s.session_id ILIKE ");
                qb.push_bind(pat);
                qb.push(" ESCAPE '\\'");
                id_or = true;
            }
            if let Some(ref tid) = turn_id_exact {
                if id_or {
                    qb.push(" OR ");
                }
                qb.push(
                    "EXISTS (
                      SELECT 1 FROM gateway_turns t
                      WHERE t.cluster_id = s.cluster_id AND t.proj_id = s.proj_id
                        AND t.session_id = s.session_id
                        AND t.turn_id = ",
                );
                qb.push_bind(tid);
                qb.push(")");
            }
            qb.push(")");
        }
        if let Some(filters) = extra_session_filter {
            for (key, val) in filters {
                let pat = format!("%{}%", Self::escape_like_pattern(val));
                qb.push(
                    " AND EXISTS (
                      SELECT 1 FROM gateway_turns t
                      WHERE t.cluster_id = s.cluster_id AND t.session_id = s.session_id AND t.proj_id = s.proj_id
                        AND COALESCE(t.entry_params_json->'extraSession'->>",
                );
                qb.push_bind(key);
                qb.push(", '') ILIKE ");
                qb.push_bind(pat);
                qb.push(" ESCAPE '\\')");
            }
        }
        qb.push(" ORDER BY s.updated_at_ms DESC, s.session_id DESC LIMIT ");
        qb.push_bind(limit);

        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(GatewaySessionSummary {
                session_id: r.try_get("session_id")?,
                created_at_ms: r.try_get("created_at_ms")?,
                updated_at_ms: r.try_get("updated_at_ms")?,
                turn_count: r.try_get("turn_count")?,
                preview_prompt: r.try_get("preview_prompt")?,
                client_origin: r.try_get("client_origin")?,
                has_bad_feedback: r.try_get("has_bad_feedback")?,
                has_good_feedback: r.try_get("has_good_feedback")?,
            });
        }
        Ok(out)
    }

    /// Turns in chronological order for replay in admin chat. Author: kejiqing
    pub async fn list_turns_for_session(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Vec<GatewayTurnSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT t.turn_id, t.user_prompt, t.status, t.created_at_ms, t.finished_at_ms,
                     t.report_message, t.output_json, t.client_origin, t.entry_params_json, f.feedback,
                     t.pool_id, t.worker_name, t.worker_exec_user,
                     (
                       (t.report_message IS NOT NULL AND btrim(t.report_message) <> '')
                       OR t.output_json IS NOT NULL
                     ) AS has_report
              FROM gateway_turns t
              LEFT JOIN gateway_feedback f
                ON f.cluster_id = t.cluster_id AND f.turn_id = t.turn_id AND f.session_id = t.session_id AND f.proj_id = t.proj_id
              WHERE t.cluster_id = $1 AND t.session_id = $2 AND t.proj_id = $3
              ORDER BY t.created_at_ms ASC, t.turn_id ASC",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let report_message: Option<String> = r.try_get("report_message")?;
            let output_json: Option<Json<Value>> = r.try_get("output_json")?;
            let output_value = output_json.map(|Json(v)| v);
            let status: String = r.try_get("status")?;
            let failure_detail = if status == "failed" {
                solve_failure_detail_from_output_json(output_value.as_ref())
            } else {
                None
            };
            let report_body = if failure_detail.is_some() {
                None
            } else {
                report_body_from_persisted(report_message.as_deref(), output_value.as_ref())
            };
            let entry_params_json: Option<Json<Value>> = r.try_get("entry_params_json")?;
            let extra_session = entry_params_json
                .map(|Json(v)| v)
                .and_then(|v| v.get("extraSession").cloned());
            out.push(GatewayTurnSummary {
                turn_id: r.try_get("turn_id")?,
                user_prompt: r.try_get("user_prompt")?,
                status,
                created_at_ms: r.try_get("created_at_ms")?,
                finished_at_ms: r.try_get("finished_at_ms")?,
                has_report: r.try_get("has_report")?,
                report_body,
                failure_detail,
                client_origin: r.try_get("client_origin")?,
                feedback: r.try_get("feedback")?,
                extra_session,
                pool_id: r.try_get("pool_id")?,
                worker_name: r.try_get("worker_name")?,
                worker_exec_user: r.try_get("worker_exec_user")?,
            });
        }
        Ok(out)
    }

    pub async fn fetch_latest_turn_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<LatestTurnRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT turn_id, session_id, proj_id, status, created_at_ms, finished_at_ms,
                     report_message, output_json, claw_exit_code, user_prompt, pool_id, worker_name,
                     worker_exec_user
              FROM gateway_turns
              WHERE session_id = $1
              ORDER BY created_at_ms DESC, turn_id DESC
              LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = row else {
            return Ok(None);
        };
        Ok(Some(LatestTurnRow {
            turn_id: r.try_get("turn_id")?,
            session_id: r.try_get("session_id")?,
            proj_id: r.try_get("proj_id")?,
            status: r.try_get("status")?,
            created_at_ms: r.try_get("created_at_ms")?,
            finished_at_ms: r.try_get("finished_at_ms")?,
            report_message: r.try_get("report_message")?,
            output_json: r.try_get("output_json")?,
            claw_exit_code: r.try_get("claw_exit_code")?,
            user_prompt: r.try_get("user_prompt")?,
            pool_id: r.try_get("pool_id")?,
            worker_name: r.try_get("worker_name")?,
            worker_exec_user: r.try_get("worker_exec_user")?,
        }))
    }

    pub async fn get_turn_status(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT status FROM gateway_turns WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4 LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(status,)| status))
    }

    pub async fn turn_belongs_to_session(
        &self,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<bool, SqlxError> {
        let row: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM gateway_turns WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4 LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn upsert_feedback(
        &self,
        session_id: &str,
        proj_id: i64,
        turn_id: &str,
        feedback: &str,
        updated_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_feedback (session_id, ds_id, proj_id, cluster_id, turn_id, feedback, updated_at_ms)
              VALUES ($1, $2, $2, $3, $4, $5, $6)
              ON CONFLICT (session_id, ds_id, turn_id) DO UPDATE SET
                feedback = EXCLUDED.feedback,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(feedback)
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_feedback(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<BTreeMap<String, String>, SqlxError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT turn_id, feedback FROM gateway_feedback WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3 ORDER BY turn_id",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn get_conversation_translate_snapshot(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<ConversationTranslateSnapshotRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT session_id, proj_id, source_fingerprint, turns_json, markdown,
                     target_language, model_id, status, error_text, created_at_ms, updated_at_ms
              FROM gateway_conversation_translate
              WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = row else {
            return Ok(None);
        };
        Ok(Some(ConversationTranslateSnapshotRow {
            session_id: r.try_get("session_id")?,
            proj_id: r.try_get("proj_id")?,
            source_fingerprint: r.try_get("source_fingerprint")?,
            turns_json: r.try_get("turns_json")?,
            markdown: r.try_get("markdown")?,
            target_language: r.try_get("target_language")?,
            model_id: r.try_get("model_id")?,
            status: r.try_get("status")?,
            error_text: r.try_get("error_text")?,
            created_at_ms: r.try_get("created_at_ms")?,
            updated_at_ms: r.try_get("updated_at_ms")?,
        }))
    }

    /// Single-flight claim: flip the row to `translating` only if it is not already
    /// translating, OR a previous `translating` row has gone stale (older than
    /// `stale_before_ms`, e.g. the worker died on restart). Returns true when this
    /// caller acquired the slot. Author: kejiqing
    pub async fn begin_conversation_translate(
        &self,
        session_id: &str,
        proj_id: i64,
        source_fingerprint: &str,
        target_language: &str,
        now_ms: i64,
        stale_before_ms: i64,
    ) -> Result<bool, SqlxError> {
        let res = sqlx::query(
            r"INSERT INTO gateway_conversation_translate (
                session_id, ds_id, proj_id, cluster_id, source_fingerprint, turns_json, markdown,
                target_language, model_id, status, error_text, created_at_ms, updated_at_ms
              ) VALUES ($1, $2, $2, $3, $4, '[]'::jsonb, '', $5, NULL, 'translating', NULL, $6, $6)
              ON CONFLICT (session_id, ds_id) DO UPDATE SET
                source_fingerprint = EXCLUDED.source_fingerprint,
                target_language = EXCLUDED.target_language,
                status = 'translating',
                error_text = NULL,
                updated_at_ms = EXCLUDED.updated_at_ms
              WHERE gateway_conversation_translate.status <> 'translating'
                 OR gateway_conversation_translate.updated_at_ms < $7",
        )
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(source_fingerprint)
        .bind(target_language)
        .bind(now_ms)
        .bind(stale_before_ms)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() >= 1)
    }

    /// Persist a finished translation snapshot (status = `ready`). Author: kejiqing
    pub async fn complete_conversation_translate(
        &self,
        session_id: &str,
        proj_id: i64,
        source_fingerprint: &str,
        turns_json: &Value,
        markdown: &str,
        target_language: &str,
        model_id: Option<&str>,
        now_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_conversation_translate (
                session_id, ds_id, proj_id, cluster_id, source_fingerprint, turns_json, markdown,
                target_language, model_id, status, error_text, created_at_ms, updated_at_ms
              ) VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, 'ready', NULL, $9, $9)
              ON CONFLICT (session_id, ds_id) DO UPDATE SET
                source_fingerprint = EXCLUDED.source_fingerprint,
                turns_json = EXCLUDED.turns_json,
                markdown = EXCLUDED.markdown,
                target_language = EXCLUDED.target_language,
                model_id = EXCLUDED.model_id,
                status = 'ready',
                error_text = NULL,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(session_id)
        .bind(proj_id)
        .bind(self.cluster_id())
        .bind(source_fingerprint)
        .bind(Json(turns_json))
        .bind(markdown)
        .bind(target_language)
        .bind(model_id)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark an in-flight translation as failed (status = `error`). Author: kejiqing
    pub async fn fail_conversation_translate(
        &self,
        session_id: &str,
        proj_id: i64,
        error_text: &str,
        now_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"UPDATE gateway_conversation_translate
              SET status = 'error', error_text = $4, updated_at_ms = $5
              WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .bind(error_text)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[cfg(test)]
    async fn fetch_updated_at_ms_for_test(
        &self,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Option<i64>, SqlxError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT updated_at_ms FROM gateway_sessions WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(&self.pool)
        .await
    }
}

/// Hide password in URLs for logs and `/healthz`.
#[must_use]
pub fn redact_database_url(url: &str) -> String {
    let Some(after_scheme) = url.split("://").nth(1) else {
        return "<invalid-database-url>".to_string();
    };
    let scheme = url.split("://").next().unwrap_or("postgres");
    if let Some((_user_pass, host_rest)) = after_scheme.split_once('@') {
        let user = after_scheme
            .split('@')
            .next()
            .and_then(|s| s.split(':').next());
        let user_label = user.unwrap_or("user");
        return format!("{scheme}://{user_label}:***@{host_rest}");
    }
    format!("{scheme}://{after_scheme}")
}

/// Integration / optional PG tests: `postgres://` host:port must accept TCP within `timeout`.
pub fn pg_tcp_reachable(database_url: &str, timeout: std::time::Duration) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::str::FromStr;

    use sqlx::postgres::PgConnectOptions;

    let Ok(opts) = PgConnectOptions::from_str(database_url.trim()) else {
        return false;
    };
    let host = opts.get_host();
    let port = opts.get_port();
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
}

fn gateway_integration_database_url() -> Option<String> {
    std::env::var("CLAW_GATEWAY_TEST_DATABASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Open PG when configured and TCP-reachable; `None` → integration test should skip.
/// GitHub `rust-ci.yml` has no `services.postgres` — do not block on sqlx connect retries.
pub async fn try_open_integration_database() -> Option<GatewaySessionDb> {
    let url = gateway_integration_database_url()?;
    if !pg_tcp_reachable(&url, std::time::Duration::from_secs(2)) {
        return None;
    }
    GatewaySessionDb::connect(&url).await.ok()
}

/// Open PG when `CLAW_GATEWAY_TEST_DATABASE_URL` is set and TCP-reachable; `None` → skip integration test.
#[cfg(test)]
pub async fn connect_gateway_test_db() -> Option<GatewaySessionDb> {
    try_open_integration_database().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn test_db() -> Option<GatewaySessionDb> {
        connect_gateway_test_db().await
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    /// Unique `gateway_turns.turn_id` for integration tests (PK is global, not per-session).
    fn test_turn_id() -> String {
        format!("T_{}", uuid::Uuid::new_v4().simple())
    }

    /// Random proj id for ephemeral PG integration tests (not small e2e ids like 1/2/10).
    fn ephemeral_test_proj_id() -> i64 {
        i64::try_from(uuid::Uuid::new_v4().as_u128() % 900_000_000).unwrap_or(42) + 1
    }

    async fn cleanup_ephemeral_project(db: &GatewaySessionDb, proj_id: i64) {
        if let Err(e) = db.delete_project_config(proj_id).await {
            eprintln!("warn: ephemeral project cleanup failed proj_id={proj_id}: {e}");
        }
    }

    #[test]
    fn redact_hides_password() {
        let r =
            redact_database_url("postgres://claw_gateway:clawGw9Dev_Pg@postgres:5432/claw_gateway");
        assert!(r.contains("claw_gateway:***@postgres"));
        assert!(!r.contains("secret"));
    }

    #[test]
    fn migration_stmt_ddl_keeps_alter_after_file_comment() {
        let raw = "-- header comment\n\nALTER TABLE t ADD COLUMN c BIGINT;\n";
        let ddl = GatewaySessionDb::migration_stmt_ddl(raw.split(';').next().unwrap_or(""));
        assert!(ddl.starts_with("ALTER TABLE t ADD COLUMN c BIGINT"));
    }

    #[tokio::test]
    async fn insert_get_touch_flow() {
        let Some(db) = test_db().await else {
            eprintln!("skip insert_get_touch_flow: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };

        let sid = format!("s1_{}", uuid::Uuid::new_v4().simple());
        assert!(db.get_session_home_rel(&sid, 7).await.unwrap().is_none());

        db.insert_session(&sid, 7, "proj_7/sessions/u1", now_ms(), None)
            .await
            .unwrap();
        assert_eq!(
            db.get_session_home_rel(&sid, 7).await.unwrap().as_deref(),
            Some("proj_7/sessions/u1")
        );

        let t2 = now_ms() + 10_000;
        db.touch_updated(&sid, 7, t2).await.unwrap();
        assert_eq!(
            db.fetch_updated_at_ms_for_test(&sid, 7).await.unwrap(),
            Some(t2)
        );
    }

    #[tokio::test]
    async fn primary_key_per_ds_id() {
        let Some(db) = test_db().await else {
            eprintln!("skip primary_key_per_ds_id: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let t = now_ms();
        let sid = format!("same_sid_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "a", t, None).await.unwrap();
        db.insert_session(&sid, 2, "b", t, None).await.unwrap();
        assert_eq!(
            db.get_session_home_rel(&sid, 1).await.unwrap().as_deref(),
            Some("a")
        );
        assert!(db.insert_session(&sid, 1, "c", t, None).await.is_err());
    }

    #[tokio::test]
    async fn turn_and_feedback_flow() {
        let Some(db) = test_db().await else {
            eprintln!("skip turn_and_feedback_flow: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let t = now_ms();
        let sid = format!("s1_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "proj_1/sessions/u1", t, None)
            .await
            .unwrap();
        let tid = test_turn_id();
        db.insert_turn(
            &tid,
            &sid,
            1,
            "queued",
            t,
            Some("hello"),
            Some("gateway-admin"),
            None,
        )
        .await
        .unwrap();
        assert!(db.turn_belongs_to_session(&tid, &sid, 1).await.unwrap());
        db.upsert_feedback(&sid, 1, &tid, "good", t).await.unwrap();
        db.upsert_feedback(&sid, 1, &tid, "bad", t + 1)
            .await
            .unwrap();
        let items = db.list_feedback(&sid, 1).await.unwrap();
        assert_eq!(items.get(&tid).map(String::as_str), Some("bad"));
        let listed = db
            .list_sessions_for_proj(1, 100, None, None, None, None, None, None, None)
            .await
            .unwrap();
        let summary = listed.iter().find(|s| s.session_id == sid).unwrap();
        assert!(summary.has_bad_feedback);
        assert!(!summary.has_good_feedback);
    }

    #[tokio::test]
    async fn finalize_turn_report_and_turn_index() {
        let Some(db) = test_db().await else {
            eprintln!(
                "skip finalize_turn_report_and_turn_index: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let sid = format!("sfin_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "proj_1/sessions/x", t, None)
            .await
            .unwrap();
        let tid1 = test_turn_id();
        let tid2 = test_turn_id();
        db.insert_turn(&tid1, &sid, 1, "queued", t, Some("a"), None, None)
            .await
            .unwrap();
        db.insert_turn(&tid2, &sid, 1, "queued", t + 100, Some("b"), None, None)
            .await
            .unwrap();
        db.finalize_turn_terminal(
            &tid1,
            "succeeded",
            Some(t + 10),
            Some("report-one"),
            None,
            Some(0),
        )
        .await
        .unwrap();
        let msg = db
            .get_turn_report_message(&tid1, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg, "report-one");
        let t2 = db
            .get_turn_created_at_ms(&tid2, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        let idx = db.turn_index_in_session(&tid2, &sid, 1, t2).await.unwrap();
        assert_eq!(idx, 2);
        let tools_ctx = db
            .get_turn_tools_context(&tid2, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tools_ctx.user_turn_index, 2);
        assert_eq!(tools_ctx.session_home_rel, "proj_1/sessions/x");
        assert_eq!(tools_ctx.created_at_ms, t2);

        let sessions = db
            .list_sessions_for_proj(1, 50, None, None, None, None, None, None, None)
            .await
            .unwrap();
        assert!(sessions.iter().any(|s| s.session_id == sid));
        let by_id = db
            .list_sessions_for_proj(1, 50, None, None, None, None, None, Some(&sid), None)
            .await
            .unwrap();
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].session_id, sid);
        let by_turn = db
            .list_sessions_for_proj(1, 50, None, None, None, None, None, Some(&tid1), None)
            .await
            .unwrap();
        assert_eq!(by_turn.len(), 1);
        assert_eq!(by_turn[0].session_id, sid);
        assert!(db
            .list_sessions_for_proj(
                1,
                50,
                None,
                None,
                None,
                None,
                None,
                Some("no-such-session"),
                None,
            )
            .await
            .unwrap()
            .is_empty());
        let listed = db.list_turns_for_session(&sid, 1).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].user_prompt.as_deref(), Some("a"));
        assert_eq!(listed[1].status, "queued");

        db.finalize_turn_terminal(
            &tid2,
            "succeeded",
            Some(t + 11),
            None,
            Some(&json!({"message": "only-json-body"})),
            Some(0),
        )
        .await
        .unwrap();
        assert!(db
            .get_turn_report_message(&tid2, &sid, 1)
            .await
            .unwrap()
            .is_none());
        let oj = db
            .get_turn_output_json(&tid2, &sid, 1)
            .await
            .unwrap()
            .expect("output_json expected");
        assert_eq!(oj["message"].as_str(), Some("only-json-body"));
    }

    #[tokio::test]
    async fn prebind_pool_id_resolves_http_base() {
        let Some(db) = test_db().await else {
            eprintln!(
                "skip prebind_pool_id_resolves_http_base: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let sid = format!("spool_{}", uuid::Uuid::new_v4().simple());
        let tid = test_turn_id();
        let pool_id = format!("pool-test-{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "proj_1/sessions/pool", t, None)
            .await
            .unwrap();
        db.insert_turn(&tid, &sid, 1, "queued", t, Some("q"), None, None)
            .await
            .unwrap();
        db.upsert_claw_pool(&ClawPoolUpsert {
            pool_id: &pool_id,
            registration_time_ms: t,
            slots_max: 4,
            slots_min: 1,
            advertise_ip: "10.0.0.8",
            sse_port: 9944,
            gateway_base: "http://10.0.0.8:18088",
            last_heartbeat_ms: t,
        })
        .await
        .unwrap();
        db.assign_turn_pool_id(&tid, &pool_id).await.unwrap();
        let base = db
            .resolve_pool_http_base_for_turn(&tid, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(base, "http://10.0.0.8:9944");
        db.assign_turn_pool_worker(&tid, &pool_id, "claw-worker-test-0", Some("claw"))
            .await
            .unwrap();
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT pool_id, worker_name FROM gateway_turns WHERE turn_id = $1")
                .bind(&tid)
                .fetch_optional(&db.pool)
                .await
                .unwrap();
        assert_eq!(
            row,
            Some((
                Some(pool_id.clone()),
                Some("claw-worker-test-0".to_string())
            ))
        );
    }

    #[tokio::test]
    async fn project_config_upsert_get() {
        let Some(db) = test_db().await else {
            eprintln!("skip project_config_upsert_get: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let proj_id = ephemeral_test_proj_id();
        let outcome = {
            use futures_util::FutureExt;
            std::panic::AssertUnwindSafe(async {
                assert!(db.get_project_config(proj_id).await.unwrap().is_none());

                let rules = json!([{
                    "ruleId": "r1",
                    "relativePath": ".cursor/rules/r1.mdc",
                    "content": "# R"
                }]);
                let mcp = json!({"demo": {"type": "http", "url": "http://127.0.0.1:9"}});
                let skills = json!([{
                    "skillName": "demo-skill",
                    "skillContent": "# Demo\n"
                }]);
                let t = now_ms();
                let tools = json!(["bash", "read_file"]);
                db.upsert_project_config(ProjectConfigUpsert {
                    proj_id,
                    content_rev: "rev-1",
                    stable_content_rev: Some("rev-1"),
                    draft_open: false,
                    updated_at_ms: t,
                    rules_json: &rules,
                    mcp_servers_json: &mcp,
                    skills_sources_json: &json!([]),
                    skills_json: &skills,
                    allowed_tools_json: &tools,
                    claude_md: Some("# Claude\n"),
                    git_sync_json: &json!({}),
                    solve_preflight_json: &json!({"kind": "sqlbot_mcp_start"}),
                    solve_orchestration_json: &json!({"kind": "single_turn"}),
                    language_pipeline_json: &json!({}),
                    extra_session_fields_json: &json!([]),
                    prompt_limits_json: &json!({}),
                    worker_profile_json: &json!({"mode": "strict"}),
                })
                .await
                .unwrap();

                let row = db.get_project_config(proj_id).await.unwrap().unwrap();
                assert_eq!(row.content_rev, "rev-1");
                assert_eq!(row.rules_json, rules);
                assert_eq!(row.mcp_servers_json, mcp);
                assert_eq!(row.skills_json, skills);
                assert_eq!(row.allowed_tools_json, tools);
                assert_eq!(row.claude_md.as_deref(), Some("# Claude\n"));

                db.upsert_project_config(ProjectConfigUpsert {
                    proj_id,
                    content_rev: "rev-2",
                    stable_content_rev: Some("rev-2"),
                    draft_open: false,
                    updated_at_ms: t + 1,
                    rules_json: &json!([]),
                    mcp_servers_json: &json!({}),
                    skills_sources_json: &json!([]),
                    skills_json: &json!([]),
                    allowed_tools_json: &json!([]),
                    claude_md: None,
                    git_sync_json: &json!({}),
                    solve_preflight_json: &json!({"kind": "none"}),
                    solve_orchestration_json: &json!({"kind": "single_turn"}),
                    language_pipeline_json: &json!({}),
                    extra_session_fields_json: &json!([]),
                    prompt_limits_json: &json!({}),
                    worker_profile_json: &json!({"mode": "strict"}),
                })
                .await
                .unwrap();
                let row2 = db.get_project_config(proj_id).await.unwrap().unwrap();
                assert_eq!(row2.content_rev, "rev-2");
                assert!(row2.claude_md.is_none());
            })
            .catch_unwind()
            .await
        };
        cleanup_ephemeral_project(&db, proj_id).await;
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    #[tokio::test]
    async fn list_sessions_filters_by_extra_session() {
        let Some(db) = test_db().await else {
            eprintln!(
                "skip list_sessions_filters_by_extra_session: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let sid_match = format!("es_match_{}", uuid::Uuid::new_v4().simple());
        let sid_other = format!("es_other_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid_match, 1, "proj_1/sessions/es_m", t, None)
            .await
            .unwrap();
        db.insert_session(&sid_other, 1, "proj_1/sessions/es_o", t, None)
            .await
            .unwrap();
        let entry_match = json!({"extraSession": {"store_id": "SH001"}, "projId": 1});
        let entry_other = json!({"extraSession": {"store_id": "SH999"}, "projId": 1});
        let tid_match = test_turn_id();
        let tid_other = test_turn_id();
        db.insert_turn(
            &tid_match,
            &sid_match,
            1,
            "queued",
            t,
            Some("q1"),
            None,
            Some(&entry_match),
        )
        .await
        .unwrap();
        db.insert_turn(
            &tid_other,
            &sid_other,
            1,
            "queued",
            t,
            Some("q2"),
            None,
            Some(&entry_other),
        )
        .await
        .unwrap();
        let mut filt = BTreeMap::new();
        filt.insert("store_id".to_string(), "SH001".to_string());
        let hits = db
            .list_sessions_for_proj(1, 50, None, None, None, None, None, None, Some(&filt))
            .await
            .unwrap();
        assert!(hits.iter().any(|s| s.session_id == sid_match));
        assert!(!hits.iter().any(|s| s.session_id == sid_other));
    }

    #[tokio::test]
    async fn session_enqueue_gate_blocks_inflight() {
        let Some(db) = test_db().await else {
            eprintln!(
                "skip session_enqueue_gate_blocks_inflight: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let sid = format!("gate_inflight_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "proj_1/sessions/gate_inflight", t, None)
            .await
            .unwrap();
        let tid_running = test_turn_id();
        db.insert_turn(&tid_running, &sid, 1, "running", t, Some("q1"), None, None)
            .await
            .unwrap();
        let err = db.assert_session_can_enqueue(&sid, 1).await.unwrap_err();
        assert_eq!(err, "inflight");
        db.assert_session_can_acquire_for_turn(&sid, 1, &tid_running)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn session_enqueue_gate_blocks_artifacts_not_ready() {
        let Some(db) = test_db().await else {
            eprintln!(
                "skip session_enqueue_gate_blocks_artifacts_not_ready: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let sid = format!("gate_art_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "proj_1/sessions/gate_art", t, None)
            .await
            .unwrap();
        let tid = test_turn_id();
        db.insert_turn(&tid, &sid, 1, "succeeded", t, Some("q1"), None, None)
            .await
            .unwrap();
        let err = db.assert_session_can_enqueue(&sid, 1).await.unwrap_err();
        assert_eq!(err, "artifacts_not_ready");
        db.finalize_turn_with_artifacts_ready(&tid, "succeeded", Some(t + 1), 0, None, None, true)
            .await
            .unwrap();
        db.assert_session_can_enqueue(&sid, 1).await.unwrap();
    }

    #[tokio::test]
    async fn workspace_tar_materialize_latest_turn() {
        use crate::pool::{WORKSPACE_TAR_ARTIFACT_KIND, WORKSPACE_TAR_ARTIFACT_PATH};

        let Some(db) = test_db().await else {
            eprintln!(
                "skip workspace_tar_materialize_latest_turn: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let sid = format!("art_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "proj_1/sessions/art", t, None)
            .await
            .unwrap();
        let tid1 = test_turn_id();
        let tid2 = test_turn_id();
        db.insert_turn(&tid1, &sid, 1, "succeeded", t, Some("q1"), None, None)
            .await
            .unwrap();
        db.insert_turn(&tid2, &sid, 1, "queued", t + 100, Some("q2"), None, None)
            .await
            .unwrap();
        db.finalize_turn_with_artifacts_ready(
            &tid1,
            "succeeded",
            Some(t + 10),
            0,
            None,
            None,
            true,
        )
        .await
        .unwrap();
        db.upsert_workspace_tar_b64(
            &sid,
            1,
            &tid1,
            WORKSPACE_TAR_ARTIFACT_PATH,
            WORKSPACE_TAR_ARTIFACT_KIND,
            "b2xkMQ==",
            4,
            t,
        )
        .await
        .unwrap();
        db.upsert_workspace_tar_b64(
            &sid,
            1,
            &tid2,
            WORKSPACE_TAR_ARTIFACT_PATH,
            WORKSPACE_TAR_ARTIFACT_KIND,
            "b2xkMg==",
            4,
            t + 100,
        )
        .await
        .unwrap();
        db.finalize_turn_with_artifacts_ready(
            &tid2,
            "succeeded",
            Some(t + 110),
            0,
            None,
            None,
            true,
        )
        .await
        .unwrap();
        let latest = db
            .get_latest_workspace_tar_b64(
                &sid,
                1,
                WORKSPACE_TAR_ARTIFACT_PATH,
                WORKSPACE_TAR_ARTIFACT_KIND,
            )
            .await
            .unwrap();
        assert_eq!(latest.as_deref(), Some("b2xkMg=="));
    }
}
