//! `PostgreSQL` persistence for gateway sessions, turns, and feedback. Author: kejiqing
//!
//! **Persistence split (see `docs/persistence-model.md`):** conversation jsonl remains the
//! runtime source of truth on disk; `gateway_turns` stores per-`turn_id` terminal snapshots
//! (`report_message`, `output_json`, …) so gateway restarts and `GET /v1/tasks` handoff stay
//! consistent at **turn** granularity.
//!
//! **Per-`ds_id` agent bundle:** `project_config` stores rules / MCP / skills sources for
//! materializing `ds_<id>/home` (see `docs/project-config-model.md`). Author: kejiqing

use std::collections::BTreeMap;

use crate::biz_advice_report::{report_body_from_persisted, solve_failure_detail_from_output_json};
use crate::turn_id::{self, TURN_ID_PREFIX};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{Error as SqlxError, PgPool, Row};

/// One row for [`GatewaySessionDb::list_sessions_for_ds`]. Author: kejiqing
#[derive(Debug, Clone)]
pub struct GatewaySessionSummary {
    pub session_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub turn_count: i64,
    pub preview_prompt: Option<String>,
    /// Who created the session (`gateway-admin`, external app, …). Author: kejiqing
    pub client_origin: Option<String>,
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
    pub ds_id: i64,
    pub status: String,
    pub created_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub report_message: Option<String>,
    pub output_json: Option<Value>,
    pub claw_exit_code: Option<i32>,
    pub user_prompt: Option<String>,
}

/// One row per `ds_id`: rules, MCP map, inline skills, optional `CLAUDE.md` body. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectConfigRow {
    pub ds_id: i64,
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
    /// Per-project one-way git push: `{ gitUrl, gitRef, gitToken, enabled, lastPush* }`. Author: kejiqing
    pub git_sync_json: Value,
    /// First-turn solve preflight: `{ "kind": "none" | "sqlbot_mcp_start" }`. Materialized to disk. Author: kejiqing
    pub solve_preflight_json: Value,
    /// Solve orchestration pipeline: `{ "kind": "single_turn" | "multi_agent_analysis", ... }`. Author: kejiqing
    pub solve_orchestration_json: Value,
}

/// Row summary for [`GatewaySessionDb::list_project_config_summaries`]. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProjectConfigSummary {
    pub ds_id: i64,
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
    pub ds_id: i64,
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
    pub ds_id: i64,
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
    pub last_heartbeat_ms: i64,
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
    pub ds_id: i64,
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
}

/// Gateway session index: one row per `(session_id, ds_id)` with a workspace-relative `session_home`.
pub struct GatewaySessionDb {
    pool: PgPool,
    database_url_redacted: String,
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
        Self::connect(url).await
    }

    /// Connect and run schema migration (tests and explicit URLs).
    pub async fn connect(database_url: &str) -> Result<Self, SqlxError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Self::migrate(&pool).await?;
        Ok(Self {
            pool,
            database_url_redacted: redact_database_url(database_url),
        })
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

    /// `session_id` + `ds_id` for a `turn_id` (report relay terminal `done`). Author: kejiqing
    pub async fn turn_session_scope(
        &self,
        turn_id: &str,
    ) -> Result<Option<(String, i64)>, SqlxError> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT session_id, ds_id FROM gateway_turns WHERE turn_id = $1 LIMIT 1",
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    #[allow(clippy::too_many_lines)]
    async fn migrate(pool: &PgPool) -> Result<(), SqlxError> {
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

        for ddl in [
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS user_prompt TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS report_message TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS output_json JSONB",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS claw_exit_code INT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS pool_id TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS worker_name TEXT",
            "ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS client_origin TEXT",
            "ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS client_origin TEXT",
        ] {
            sqlx::query(ddl).execute(pool).await?;
        }

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS claw_pool (
                pool_id TEXT PRIMARY KEY,
                registration_time_ms BIGINT NOT NULL,
                slots_max INT NOT NULL,
                slots_min INT NOT NULL,
                advertise_ip TEXT NOT NULL,
                sse_port INT NOT NULL,
                last_heartbeat_ms BIGINT NOT NULL
            )",
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
        sqlx::query(
            r"INSERT INTO project_config_revision (
                ds_id, content_rev, created_at_ms, rules_json, mcp_servers_json,
                skills_sources_json, skills_json, allowed_tools_json, claude_md
            )
            SELECT ds_id, content_rev, updated_at_ms, rules_json, mcp_servers_json,
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
        sqlx::query(
            r"INSERT INTO gateway_global_settings (singleton_id)
             VALUES (1) ON CONFLICT (singleton_id) DO NOTHING",
        )
        .execute(pool)
        .await?;
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

        Ok(())
    }

    /// System-level default scaffold (no public write API; update via DB migration). Author: kejiqing
    pub async fn get_gateway_system_prompt_default(&self) -> Result<(String, String), SqlxError> {
        let row = sqlx::query(
            r"SELECT system_prompt_default, system_prompt_version
               FROM gateway_global_settings WHERE singleton_id = 1",
        )
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
               FROM gateway_global_settings WHERE singleton_id = 1",
        )
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
               FROM gateway_global_settings WHERE singleton_id = 1",
        )
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
                 singleton_id, llm_models_json, llm_model_api_keys_json,
                 active_llm_model_id, active_llm_model_rev, active_llm_applied_at_ms, updated_at_ms
               ) VALUES (1, $1, $2, $3, $4, $5, $6)
               ON CONFLICT (singleton_id) DO UPDATE SET
                 llm_models_json = EXCLUDED.llm_models_json,
                 llm_model_api_keys_json = EXCLUDED.llm_model_api_keys_json,
                 active_llm_model_id = EXCLUDED.active_llm_model_id,
                 active_llm_model_rev = EXCLUDED.active_llm_model_rev,
                 active_llm_applied_at_ms = EXCLUDED.active_llm_applied_at_ms,
                 updated_at_ms = GREATEST(gateway_global_settings.updated_at_ms, EXCLUDED.updated_at_ms),
                 settings_json = gateway_global_settings.settings_json - 'llmModel',
                 git_pat_tokens_json = gateway_global_settings.git_pat_tokens_json - '__gateway_llm_api_key__'",
        )
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
            r"INSERT INTO gateway_global_settings (singleton_id, settings_json, git_pat_tokens_json, updated_at_ms)
               VALUES (1, $1, $2, $3)
               ON CONFLICT (singleton_id) DO UPDATE SET
                 settings_json = EXCLUDED.settings_json,
                 git_pat_tokens_json = EXCLUDED.git_pat_tokens_json,
                 updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(Json(settings_json))
        .bind(Json(git_pat_tokens_json))
        .bind(updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_project_config_ds_ids(&self) -> Result<Vec<i64>, SqlxError> {
        let rows = sqlx::query_scalar::<_, i64>("SELECT ds_id FROM project_config ORDER BY ds_id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Admin list: one row per `project_config` (DB truth for skills / CLAUDE). Author: kejiqing
    pub async fn list_project_config_summaries(
        &self,
    ) -> Result<Vec<ProjectConfigSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT ds_id, content_rev, stable_content_rev, draft_open, updated_at_ms, claude_md,
                      skills_json, rules_json, mcp_servers_json, git_sync_json
               FROM project_config ORDER BY ds_id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let ds_id: i64 = row.try_get("ds_id")?;
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
                ds_id,
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
        ds_id: i64,
    ) -> Result<Option<ProjectConfigRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT ds_id, content_rev, stable_content_rev, draft_open, updated_at_ms,
                      rules_json, mcp_servers_json, skills_sources_json, skills_json,
                      allowed_tools_json, claude_md, git_sync_json, solve_preflight_json,
                      solve_orchestration_json
               FROM project_config WHERE ds_id = $1",
        )
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let ds_id: i64 = row.try_get("ds_id")?;
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

        let stable_content_rev: Option<String> = row.try_get("stable_content_rev")?;
        let draft_open: bool = row.try_get("draft_open")?;

        Ok(Some(ProjectConfigRow {
            ds_id,
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
        }))
    }

    pub async fn upsert_project_config(
        &self,
        row: ProjectConfigUpsert<'_>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO project_config (
                ds_id, content_rev, stable_content_rev, draft_open, updated_at_ms,
                rules_json, mcp_servers_json, skills_sources_json, skills_json,
                allowed_tools_json, claude_md, git_sync_json, solve_preflight_json,
                solve_orchestration_json
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (ds_id) DO UPDATE SET
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
                solve_orchestration_json = EXCLUDED.solve_orchestration_json",
        )
        .bind(row.ds_id)
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
                ds_id, content_rev, created_at_ms, note, rules_json, mcp_servers_json,
                skills_sources_json, skills_json, allowed_tools_json, claude_md
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (ds_id, content_rev) DO NOTHING",
        )
        .bind(row.ds_id)
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
        ds_id: i64,
        content_rev: &str,
    ) -> Result<Option<ProjectConfigRevisionRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT ds_id, content_rev, created_at_ms, note, rules_json, mcp_servers_json,
                      skills_sources_json, skills_json, allowed_tools_json, claude_md
               FROM project_config_revision
               WHERE ds_id = $1 AND content_rev = $2",
        )
        .bind(ds_id)
        .bind(content_rev)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ProjectConfigRevisionRow {
            ds_id: row.try_get("ds_id")?,
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
        ds_id: i64,
    ) -> Result<Vec<ProjectConfigRevisionSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT content_rev, created_at_ms, note, claude_md, skills_json, rules_json, mcp_servers_json
               FROM project_config_revision
               WHERE ds_id = $1
               ORDER BY created_at_ms DESC, content_rev DESC",
        )
        .bind(ds_id)
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
        ds_id: i64,
        content_rev: &str,
        note: Option<&str>,
    ) -> Result<bool, SqlxError> {
        let r = sqlx::query(
            "UPDATE project_config_revision SET note = $3 WHERE ds_id = $1 AND content_rev = $2",
        )
        .bind(ds_id)
        .bind(content_rev)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Drop one saved revision (not the effective stable rev). Author: kejiqing
    pub async fn delete_project_config_revision(
        &self,
        ds_id: i64,
        content_rev: &str,
    ) -> Result<bool, SqlxError> {
        let r = sqlx::query(
            "DELETE FROM project_config_revision WHERE ds_id = $1 AND content_rev = $2",
        )
        .bind(ds_id)
        .bind(content_rev)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_project_config_revisions(&self, ds_id: i64) -> Result<u64, SqlxError> {
        let r = sqlx::query("DELETE FROM project_config_revision WHERE ds_id = $1")
            .bind(ds_id)
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
                ds_id, domain, entity_key, entity_rev, created_at_ms, note, body
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (ds_id, domain, entity_key, entity_rev) DO NOTHING",
        )
        .bind(row.ds_id)
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
        ds_id: i64,
        domain: &str,
        entity_key: &str,
        entity_rev: &str,
    ) -> Result<Option<ProjectEntityRevisionRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT ds_id, domain, entity_key, entity_rev, created_at_ms, note, body
               FROM project_entity_revision
               WHERE ds_id = $1 AND domain = $2 AND entity_key = $3 AND entity_rev = $4",
        )
        .bind(ds_id)
        .bind(domain)
        .bind(entity_key)
        .bind(entity_rev)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ProjectEntityRevisionRow {
            ds_id: row.try_get("ds_id")?,
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
        ds_id: i64,
        domain: &str,
        entity_key: &str,
    ) -> Result<Vec<ProjectEntityRevisionSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT entity_rev, created_at_ms, note
               FROM project_entity_revision
               WHERE ds_id = $1 AND domain = $2 AND entity_key = $3
               ORDER BY created_at_ms DESC, entity_rev DESC",
        )
        .bind(ds_id)
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

    pub async fn delete_project_entity_revisions(&self, ds_id: i64) -> Result<u64, SqlxError> {
        let r = sqlx::query("DELETE FROM project_entity_revision WHERE ds_id = $1")
            .bind(ds_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Remove `project_config` row for a ds (project delete). Author: kejiqing
    pub async fn delete_project_config(&self, ds_id: i64) -> Result<bool, SqlxError> {
        let _ = self.delete_project_config_revisions(ds_id).await?;
        let _ = self.delete_project_entity_revisions(ds_id).await?;
        let r = sqlx::query("DELETE FROM project_config WHERE ds_id = $1")
            .bind(ds_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Delete all sessions and turns for a ds (optional on project delete). Author: kejiqing
    pub async fn delete_sessions_for_ds(&self, ds_id: i64) -> Result<u64, SqlxError> {
        sqlx::query("DELETE FROM gateway_turns WHERE ds_id = $1")
            .bind(ds_id)
            .execute(&self.pool)
            .await?;
        let r = sqlx::query("DELETE FROM gateway_sessions WHERE ds_id = $1")
            .bind(ds_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn get_session_home_rel(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, String>(
            "SELECT session_home FROM gateway_sessions WHERE session_id = $1 AND ds_id = $2",
        )
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn insert_session(
        &self,
        session_id: &str,
        ds_id: i64,
        session_home_rel: &str,
        now_ms: i64,
        client_origin: Option<&str>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_sessions (session_id, ds_id, session_home, created_at_ms, updated_at_ms, client_origin)
              VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session_id)
        .bind(ds_id)
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
        ds_id: i64,
        now_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE gateway_sessions SET updated_at_ms = $1 WHERE session_id = $2 AND ds_id = $3",
        )
        .bind(now_ms)
        .bind(session_id)
        .bind(ds_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_exists(&self, session_id: &str, ds_id: i64) -> Result<bool, SqlxError> {
        let row: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM gateway_sessions WHERE session_id = $1 AND ds_id = $2 LIMIT 1",
        )
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn insert_turn(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
        status: &str,
        created_at_ms: i64,
        user_prompt: Option<&str>,
        client_origin: Option<&str>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_turns (turn_id, session_id, ds_id, status, created_at_ms, finished_at_ms, user_prompt, client_origin)
              VALUES ($1, $2, $3, $4, $5, NULL, $6, $7)",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .bind(status)
        .bind(created_at_ms)
        .bind(user_prompt)
        .bind(client_origin)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_turn_client_origin(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT client_origin FROM gateway_turns WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3 LIMIT 1",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(o,)| o))
    }

    /// Register or refresh a pool node (`claw-pool-daemon` startup). Author: kejiqing
    pub async fn upsert_claw_pool(&self, row: &ClawPoolUpsert<'_>) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO claw_pool (
                pool_id, registration_time_ms, slots_max, slots_min,
                advertise_ip, sse_port, last_heartbeat_ms
              ) VALUES ($1, $2, $3, $4, $5, $6, $7)
              ON CONFLICT (pool_id) DO UPDATE SET
                slots_max = EXCLUDED.slots_max,
                slots_min = EXCLUDED.slots_min,
                advertise_ip = EXCLUDED.advertise_ip,
                sse_port = EXCLUDED.sse_port,
                last_heartbeat_ms = EXCLUDED.last_heartbeat_ms",
        )
        .bind(row.pool_id)
        .bind(row.registration_time_ms)
        .bind(row.slots_max)
        .bind(row.slots_min)
        .bind(row.advertise_ip)
        .bind(row.sse_port)
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
        sqlx::query("UPDATE claw_pool SET last_heartbeat_ms = $2 WHERE pool_id = $1")
            .bind(pool_id)
            .bind(last_heartbeat_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
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
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET pool_id = $2, worker_name = $3 WHERE turn_id = $1")
            .bind(turn_id)
            .bind(pool_id)
            .bind(worker_name)
            .execute(&self.pool)
            .await?;
        Ok(())
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
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar(
            "SELECT pool_id FROM gateway_turns WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3 LIMIT 1",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// `http://{advertise_ip}:{sse_port}` for live SSE proxy when turn has `pool_id`. Author: kejiqing
    pub async fn resolve_pool_http_base_for_turn(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(String, i32)> = sqlx::query_as(
            r"SELECT p.advertise_ip, p.sse_port
              FROM gateway_turns t
              JOIN claw_pool p ON t.pool_id = p.pool_id
              WHERE t.turn_id = $1 AND t.session_id = $2 AND t.ds_id = $3
              LIMIT 1",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
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
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, String>(
            r"SELECT report_message FROM gateway_turns
              WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3
                AND report_message IS NOT NULL AND btrim(report_message) <> ''",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// Terminal solve `output_json` snapshot for this turn, if persisted (`finalize_turn_terminal`).
    pub async fn get_turn_output_json(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<Value>, SqlxError> {
        let row = sqlx::query(
            r"SELECT output_json FROM gateway_turns
              WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3
                AND output_json IS NOT NULL",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
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
        ds_id: i64,
    ) -> Result<Option<TurnToolsContext>, SqlxError> {
        let row = sqlx::query(
            r"SELECT s.session_home, t.created_at_ms, t.finished_at_ms,
                     (SELECT COUNT(*)::bigint FROM gateway_turns t2
                      WHERE t2.session_id = t.session_id AND t2.ds_id = t.ds_id
                        AND (t2.created_at_ms < t.created_at_ms
                             OR (t2.created_at_ms = t.created_at_ms AND t2.turn_id <= t.turn_id))
                     ) AS user_turn_index
              FROM gateway_turns t
              INNER JOIN gateway_sessions s
                ON s.session_id = t.session_id AND s.ds_id = t.ds_id
              WHERE t.turn_id = $1 AND t.session_id = $2 AND t.ds_id = $3",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
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
        ds_id: i64,
    ) -> Result<Option<i64>, SqlxError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT created_at_ms FROM gateway_turns WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// 1-based index of this turn among rows in `gateway_turns` for the same session, ordered by
    /// `(created_at_ms, turn_id)` (stable under concurrent inserts for disjoint `turn_id`s).
    pub async fn turn_index_in_session(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
        created_at_ms: i64,
    ) -> Result<i64, SqlxError> {
        let v: i64 = sqlx::query_scalar(
            r"SELECT COUNT(*)::bigint FROM gateway_turns
              WHERE session_id = $1 AND ds_id = $2
                AND (created_at_ms < $3 OR (created_at_ms = $3 AND turn_id <= $4))",
        )
        .bind(session_id)
        .bind(ds_id)
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
              WHERE status IN ('queued', 'running')",
        )
        .bind(now_ms)
        .bind(detail)
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
    pub async fn list_sessions_for_ds(
        &self,
        ds_id: i64,
        limit: i64,
        before_updated_at_ms: Option<i64>,
        before_session_id: Option<&str>,
        updated_from_ms: Option<i64>,
        updated_to_ms: Option<i64>,
        title_q: Option<&str>,
        session_id_q: Option<&str>,
    ) -> Result<Vec<GatewaySessionSummary>, SqlxError> {
        let limit = limit.clamp(1, 100);
        let like_pat = title_q
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|q| format!("%{}%", Self::escape_like_pattern(q)));
        let (session_id_pat, turn_id_exact) = Self::parse_session_list_id_filter(session_id_q);

        let rows = sqlx::query(
            r"SELECT s.session_id, s.created_at_ms, s.updated_at_ms, s.client_origin,
                     (SELECT COUNT(*)::bigint FROM gateway_turns t
                        WHERE t.session_id = s.session_id AND t.ds_id = s.ds_id) AS turn_count,
                     (SELECT t.user_prompt FROM gateway_turns t
                        WHERE t.session_id = s.session_id AND t.ds_id = s.ds_id
                        ORDER BY t.created_at_ms ASC, t.turn_id ASC
                        LIMIT 1) AS preview_prompt
              FROM gateway_sessions s
              WHERE s.ds_id = $1
                AND ($2::bigint IS NULL OR s.updated_at_ms >= $2)
                AND ($3::bigint IS NULL OR s.updated_at_ms <= $3)
                AND (
                  $4::text IS NULL
                  OR (
                    SELECT t.user_prompt FROM gateway_turns t
                      WHERE t.session_id = s.session_id AND t.ds_id = s.ds_id
                      ORDER BY t.created_at_ms ASC, t.turn_id ASC
                      LIMIT 1
                  ) ILIKE $4 ESCAPE '\'
                )
                AND (
                  $5::bigint IS NULL
                  OR s.updated_at_ms < $5
                  OR (s.updated_at_ms = $5 AND s.session_id < COALESCE($6, ''))
                )
                AND (
                  ($8::text IS NULL AND $9::text IS NULL)
                  OR ($8::text IS NOT NULL AND s.session_id ILIKE $8 ESCAPE '\')
                  OR (
                    $9::text IS NOT NULL
                    AND EXISTS (
                      SELECT 1 FROM gateway_turns t
                      WHERE t.ds_id = s.ds_id
                        AND t.session_id = s.session_id
                        AND t.turn_id = $9
                    )
                  )
                )
              ORDER BY s.updated_at_ms DESC, s.session_id DESC
              LIMIT $7",
        )
        .bind(ds_id)
        .bind(updated_from_ms)
        .bind(updated_to_ms)
        .bind(like_pat.as_deref())
        .bind(before_updated_at_ms)
        .bind(before_session_id)
        .bind(limit)
        .bind(session_id_pat.as_deref())
        .bind(turn_id_exact.as_deref())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(GatewaySessionSummary {
                session_id: r.try_get("session_id")?,
                created_at_ms: r.try_get("created_at_ms")?,
                updated_at_ms: r.try_get("updated_at_ms")?,
                turn_count: r.try_get("turn_count")?,
                preview_prompt: r.try_get("preview_prompt")?,
                client_origin: r.try_get("client_origin")?,
            });
        }
        Ok(out)
    }

    /// Turns in chronological order for replay in admin chat. Author: kejiqing
    pub async fn list_turns_for_session(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Vec<GatewayTurnSummary>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT t.turn_id, t.user_prompt, t.status, t.created_at_ms, t.finished_at_ms,
                     t.report_message, t.output_json, t.client_origin, f.feedback,
                     (
                       (t.report_message IS NOT NULL AND btrim(t.report_message) <> '')
                       OR t.output_json IS NOT NULL
                     ) AS has_report
              FROM gateway_turns t
              LEFT JOIN gateway_feedback f
                ON f.turn_id = t.turn_id AND f.session_id = t.session_id AND f.ds_id = t.ds_id
              WHERE t.session_id = $1 AND t.ds_id = $2
              ORDER BY t.created_at_ms ASC, t.turn_id ASC",
        )
        .bind(session_id)
        .bind(ds_id)
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
            });
        }
        Ok(out)
    }

    pub async fn fetch_latest_turn_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<LatestTurnRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT turn_id, session_id, ds_id, status, created_at_ms, finished_at_ms,
                     report_message, output_json, claw_exit_code, user_prompt
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
            ds_id: r.try_get("ds_id")?,
            status: r.try_get("status")?,
            created_at_ms: r.try_get("created_at_ms")?,
            finished_at_ms: r.try_get("finished_at_ms")?,
            report_message: r.try_get("report_message")?,
            output_json: r.try_get("output_json")?,
            claw_exit_code: r.try_get("claw_exit_code")?,
            user_prompt: r.try_get("user_prompt")?,
        }))
    }

    pub async fn get_turn_status(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT status FROM gateway_turns WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3 LIMIT 1",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(status,)| status))
    }

    pub async fn turn_belongs_to_session(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<bool, SqlxError> {
        let row: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM gateway_turns WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3 LIMIT 1",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn upsert_feedback(
        &self,
        session_id: &str,
        ds_id: i64,
        turn_id: &str,
        feedback: &str,
        updated_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_feedback (session_id, ds_id, turn_id, feedback, updated_at_ms)
              VALUES ($1, $2, $3, $4, $5)
              ON CONFLICT (session_id, ds_id, turn_id) DO UPDATE SET
                feedback = EXCLUDED.feedback,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(session_id)
        .bind(ds_id)
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
        ds_id: i64,
    ) -> Result<BTreeMap<String, String>, SqlxError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT turn_id, feedback FROM gateway_feedback WHERE session_id = $1 AND ds_id = $2 ORDER BY turn_id",
        )
        .bind(session_id)
        .bind(ds_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    #[cfg(test)]
    async fn fetch_updated_at_ms_for_test(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<i64>, SqlxError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT updated_at_ms FROM gateway_sessions WHERE session_id = $1 AND ds_id = $2",
        )
        .bind(session_id)
        .bind(ds_id)
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

/// Test DB from `CLAW_GATEWAY_TEST_DATABASE_URL` or `CLAW_GATEWAY_DATABASE_URL`. Author: kejiqing
#[cfg(test)]
pub async fn connect_gateway_test_db() -> Option<GatewaySessionDb> {
    let url = std::env::var("CLAW_GATEWAY_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("CLAW_GATEWAY_DATABASE_URL"))
        .ok()?;
    GatewaySessionDb::connect(url.trim()).await.ok()
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

    #[test]
    fn redact_hides_password() {
        let r =
            redact_database_url("postgres://claw_gateway:clawGw9Dev_Pg@postgres:5432/claw_gateway");
        assert!(r.contains("claw_gateway:***@postgres"));
        assert!(!r.contains("secret"));
    }

    #[tokio::test]
    async fn insert_get_touch_flow() {
        let Some(db) = test_db().await else {
            eprintln!("skip insert_get_touch_flow: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };

        assert!(db.get_session_home_rel("s1", 7).await.unwrap().is_none());

        db.insert_session("s1", 7, "ds_7/sessions/u1", now_ms(), None)
            .await
            .unwrap();
        assert_eq!(
            db.get_session_home_rel("s1", 7).await.unwrap().as_deref(),
            Some("ds_7/sessions/u1")
        );

        let t2 = now_ms() + 10_000;
        db.touch_updated("s1", 7, t2).await.unwrap();
        assert_eq!(
            db.fetch_updated_at_ms_for_test("s1", 7).await.unwrap(),
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
        db.insert_session(&sid, 1, "ds_1/sessions/u1", t, None)
            .await
            .unwrap();
        db.insert_turn(
            "T_a1b2c3d4e5f6478990abcdef12345678",
            &sid,
            1,
            "queued",
            t,
            Some("hello"),
            Some("gateway-admin"),
        )
        .await
        .unwrap();
        assert!(db
            .turn_belongs_to_session("T_a1b2c3d4e5f6478990abcdef12345678", &sid, 1)
            .await
            .unwrap());
        db.upsert_feedback(&sid, 1, "T_a1b2c3d4e5f6478990abcdef12345678", "good", t)
            .await
            .unwrap();
        db.upsert_feedback(&sid, 1, "T_a1b2c3d4e5f6478990abcdef12345678", "bad", t + 1)
            .await
            .unwrap();
        let items = db.list_feedback(&sid, 1).await.unwrap();
        assert_eq!(
            items
                .get("T_a1b2c3d4e5f6478990abcdef12345678")
                .map(String::as_str),
            Some("bad")
        );
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
        db.insert_session(&sid, 1, "ds_1/sessions/x", t, None)
            .await
            .unwrap();
        let tid1 = "T_10000000000000000000000000000001";
        let tid2 = "T_20000000000000000000000000000002";
        db.insert_turn(tid1, &sid, 1, "queued", t, Some("a"), None)
            .await
            .unwrap();
        db.insert_turn(tid2, &sid, 1, "queued", t + 100, Some("b"), None)
            .await
            .unwrap();
        db.finalize_turn_terminal(
            tid1,
            "succeeded",
            Some(t + 10),
            Some("report-one"),
            None,
            Some(0),
        )
        .await
        .unwrap();
        let msg = db
            .get_turn_report_message(tid1, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg, "report-one");
        let t2 = db
            .get_turn_created_at_ms(tid2, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        let idx = db.turn_index_in_session(tid2, &sid, 1, t2).await.unwrap();
        assert_eq!(idx, 2);
        let tools_ctx = db
            .get_turn_tools_context(tid2, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tools_ctx.user_turn_index, 2);
        assert_eq!(tools_ctx.session_home_rel, "ds_1/sessions/x");
        assert_eq!(tools_ctx.created_at_ms, t2);

        let sessions = db
            .list_sessions_for_ds(1, 50, None, None, None, None, None, None)
            .await
            .unwrap();
        assert!(sessions.iter().any(|s| s.session_id == sid));
        let by_id = db
            .list_sessions_for_ds(1, 50, None, None, None, None, None, Some(&sid))
            .await
            .unwrap();
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].session_id, sid);
        let by_turn = db
            .list_sessions_for_ds(1, 50, None, None, None, None, None, Some(tid1))
            .await
            .unwrap();
        assert_eq!(by_turn.len(), 1);
        assert_eq!(by_turn[0].session_id, sid);
        assert!(db
            .list_sessions_for_ds(1, 50, None, None, None, None, None, Some("no-such-session"))
            .await
            .unwrap()
            .is_empty());
        let listed = db.list_turns_for_session(&sid, 1).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].user_prompt.as_deref(), Some("a"));
        assert_eq!(listed[1].status, "queued");

        db.finalize_turn_terminal(
            tid2,
            "succeeded",
            Some(t + 11),
            None,
            Some(&json!({"message": "only-json-body"})),
            Some(0),
        )
        .await
        .unwrap();
        assert!(db
            .get_turn_report_message(tid2, &sid, 1)
            .await
            .unwrap()
            .is_none());
        let oj = db
            .get_turn_output_json(tid2, &sid, 1)
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
        let tid = "T_30000000000000000000000000000003";
        let pool_id = format!("pool-test-{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "ds_1/sessions/pool", t, None)
            .await
            .unwrap();
        db.insert_turn(tid, &sid, 1, "queued", t, Some("q"), None)
            .await
            .unwrap();
        db.upsert_claw_pool(&ClawPoolUpsert {
            pool_id: &pool_id,
            registration_time_ms: t,
            slots_max: 4,
            slots_min: 1,
            advertise_ip: "10.0.0.8",
            sse_port: 9944,
            last_heartbeat_ms: t,
        })
        .await
        .unwrap();
        db.assign_turn_pool_id(tid, &pool_id).await.unwrap();
        let base = db
            .resolve_pool_http_base_for_turn(tid, &sid, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(base, "http://10.0.0.8:9944");
        db.assign_turn_pool_worker(tid, &pool_id, "claw-worker-test-0")
            .await
            .unwrap();
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT pool_id, worker_name FROM gateway_turns WHERE turn_id = $1")
                .bind(tid)
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
        let ds_id = i64::try_from(uuid::Uuid::new_v4().as_u128() % 900_000_000).unwrap_or(42) + 1;

        assert!(db.get_project_config(ds_id).await.unwrap().is_none());

        let rules =
            json!([{"ruleId": "r1", "relativePath": ".cursor/rules/r1.mdc", "content": "# R"}]);
        let mcp = json!({"demo": {"type": "http", "url": "http://127.0.0.1:9"}});
        let skills = json!([{
            "skillName": "demo-skill",
            "skillContent": "# Demo\n"
        }]);
        let t = now_ms();
        let tools = json!(["bash", "read_file"]);
        db.upsert_project_config(ProjectConfigUpsert {
            ds_id,
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
        })
        .await
        .unwrap();

        let row = db.get_project_config(ds_id).await.unwrap().unwrap();
        assert_eq!(row.content_rev, "rev-1");
        assert_eq!(row.rules_json, rules);
        assert_eq!(row.mcp_servers_json, mcp);
        assert_eq!(row.skills_json, skills);
        assert_eq!(row.allowed_tools_json, tools);
        assert_eq!(row.claude_md.as_deref(), Some("# Claude\n"));

        db.upsert_project_config(ProjectConfigUpsert {
            ds_id,
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
        })
        .await
        .unwrap();
        let row2 = db.get_project_config(ds_id).await.unwrap().unwrap();
        assert_eq!(row2.content_rev, "rev-2");
        assert!(row2.claude_md.is_none());
    }
}
