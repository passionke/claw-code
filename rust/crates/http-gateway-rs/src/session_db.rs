//! `PostgreSQL` persistence for gateway sessions, turns, and feedback. Author: kejiqing
#![allow(clippy::too_many_arguments)]

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Error as SqlxError, PgPool};

/// One row from `cc_messages` for transcript export.
#[derive(Debug, Clone)]
pub struct CcMessageRow {
    pub role: String,
    pub blocks: Value,
    pub usage: Option<Value>,
}

/// Persisted async task row (`task_id` == `session_id`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AsyncTaskRow {
    pub task_id: String,
    pub session_id: String,
    pub ds_id: i64,
    pub active_turn_id: String,
    pub status: String,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub current_task_desc: Option<String>,
    pub progress_updated_at_ms: Option<i64>,
    pub has_report: bool,
}

const MIGRATION_001: &str = include_str!("../migrations/001_baseline.sql");
const MIGRATION_002: &str = include_str!("../migrations/002_persistence.sql");

async fn run_migrations(pool: &PgPool) -> Result<(), SqlxError> {
    sqlx::query(
        r"CREATE TABLE IF NOT EXISTS gateway_schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at_ms BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    let now = super_persistence_now_ms();
    for (version, sql) in [
        ("001_baseline", MIGRATION_001),
        ("002_persistence", MIGRATION_002),
    ] {
        let applied: Option<String> =
            sqlx::query_scalar("SELECT version FROM gateway_schema_migrations WHERE version = $1")
                .bind(version)
                .fetch_optional(pool)
                .await?;
        if applied.is_some() {
            continue;
        }
        for stmt in split_sql_statements(sql) {
            if stmt.trim().is_empty() {
                continue;
            }
            sqlx::query(&stmt).execute(pool).await?;
        }
        sqlx::query(
            "INSERT INTO gateway_schema_migrations (version, applied_at_ms) VALUES ($1, $2)",
        )
        .bind(version)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for ch in sql.chars() {
        match ch {
            '\'' => {
                in_quote = !in_quote;
                cur.push(ch);
            }
            ';' if !in_quote => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn super_persistence_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
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

    async fn migrate(pool: &PgPool) -> Result<(), SqlxError> {
        run_migrations(pool).await
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn upsert_project(
        &self,
        ds_id: i64,
        project_name: &str,
        workspace_rel: &str,
    ) -> Result<(), SqlxError> {
        let now = super_persistence_now_ms();
        sqlx::query(
            r"INSERT INTO gateway_projects (ds_id, project_name, workspace_rel, created_at_ms, updated_at_ms)
              VALUES ($1, $2, $3, $4, $5)
              ON CONFLICT (ds_id) DO UPDATE SET
                project_name = EXCLUDED.project_name,
                workspace_rel = EXCLUDED.workspace_rel,
                updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(ds_id)
        .bind(project_name)
        .bind(workspace_rel)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_turn_user_prompt(
        &self,
        turn_id: &str,
        user_prompt: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE gateway_turns SET user_prompt = $1 WHERE turn_id = $2")
            .bind(user_prompt)
            .bind(turn_id)
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
                claw_exit_code = $1,
                report_message = $2,
                output_json = $3,
                has_report = $4
              WHERE turn_id = $5",
        )
        .bind(claw_exit_code)
        .bind(report_message)
        .bind(output_json)
        .bind(has_report)
        .bind(turn_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_turn_output_json(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<Value>, SqlxError> {
        sqlx::query_scalar::<_, Value>(
            r"SELECT output_json FROM gateway_turns
              WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn get_turn_report_message(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, String>(
            r"SELECT report_message FROM gateway_turns
              WHERE turn_id = $1 AND session_id = $2 AND ds_id = $3",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn delete_messages_for_turn(&self, turn_id: &str) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM cc_messages WHERE turn_id = $1")
            .bind(turn_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_message(
        &self,
        session_id: &str,
        ds_id: i64,
        turn_id: &str,
        iteration_id: Option<uuid::Uuid>,
        seq: i32,
        role: &str,
        blocks: &Value,
        usage: Option<&Value>,
        created_at_ms: i64,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO cc_messages
              (session_id, ds_id, turn_id, iteration_id, seq, role, blocks, usage, created_at_ms)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(session_id)
        .bind(ds_id)
        .bind(turn_id)
        .bind(iteration_id)
        .bind(seq)
        .bind(role)
        .bind(blocks)
        .bind(usage)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_messages_for_session(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Vec<CcMessageRow>, SqlxError> {
        let rows: Vec<(String, Value, Option<Value>)> = sqlx::query_as(
            r"SELECT m.role, m.blocks, m.usage FROM cc_messages m
              INNER JOIN gateway_turns t ON t.turn_id = m.turn_id
              WHERE m.session_id = $1 AND m.ds_id = $2
              ORDER BY t.created_at_ms ASC, m.seq ASC",
        )
        .bind(session_id)
        .bind(ds_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(role, blocks, usage)| CcMessageRow {
                role,
                blocks,
                usage,
            })
            .collect())
    }

    pub async fn list_messages_for_turn(
        &self,
        turn_id: &str,
    ) -> Result<Vec<CcMessageRow>, SqlxError> {
        let rows: Vec<(String, Value, Option<Value>)> = sqlx::query_as(
            "SELECT role, blocks, usage FROM cc_messages WHERE turn_id = $1 ORDER BY seq ASC",
        )
        .bind(turn_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(role, blocks, usage)| CcMessageRow {
                role,
                blocks,
                usage,
            })
            .collect())
    }

    pub async fn ensure_runtime_iteration(
        &self,
        turn_id: &str,
        iteration_index: i32,
        started_at_ms: i64,
    ) -> Result<uuid::Uuid, SqlxError> {
        if let Some((id,)) = sqlx::query_as::<_, (uuid::Uuid,)>(
            "SELECT iteration_id FROM gateway_runtime_iterations WHERE turn_id = $1 AND iteration_index = $2",
        )
        .bind(turn_id)
        .bind(iteration_index)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4();
        sqlx::query(
            r"INSERT INTO gateway_runtime_iterations
              (iteration_id, turn_id, iteration_index, started_at_ms)
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

    pub async fn upsert_turn_container_run(
        &self,
        turn_id: &str,
        session_mount_path: &str,
        started_at_ms: i64,
        finished_at_ms: i64,
        duration_ms: i64,
        worker_container_id: Option<&str>,
        worker_image: Option<&str>,
        pool_slot_index: Option<i32>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_turn_container_runs
              (turn_id, pool_slot_index, worker_container_id, worker_image, session_mount_path,
               started_at_ms, finished_at_ms, duration_ms)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
              ON CONFLICT (turn_id) DO UPDATE SET
                pool_slot_index = EXCLUDED.pool_slot_index,
                worker_container_id = EXCLUDED.worker_container_id,
                worker_image = EXCLUDED.worker_image,
                session_mount_path = EXCLUDED.session_mount_path,
                started_at_ms = EXCLUDED.started_at_ms,
                finished_at_ms = EXCLUDED.finished_at_ms,
                duration_ms = EXCLUDED.duration_ms",
        )
        .bind(turn_id)
        .bind(pool_slot_index)
        .bind(worker_container_id)
        .bind(worker_image)
        .bind(session_mount_path)
        .bind(started_at_ms)
        .bind(finished_at_ms)
        .bind(duration_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_model_usage(
        &self,
        turn_id: &str,
        provider: Option<&str>,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
        cache_creation_input_tokens: i32,
        cache_read_input_tokens: i32,
        latency_ms: Option<i64>,
        source: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_model_usage
              (turn_id, provider, model, input_tokens, output_tokens,
               cache_creation_input_tokens, cache_read_input_tokens, latency_ms, source)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(turn_id)
        .bind(provider)
        .bind(model)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_creation_input_tokens)
        .bind(cache_read_input_tokens)
        .bind(latency_ms)
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_async_task(
        &self,
        task_id: &str,
        session_id: &str,
        ds_id: i64,
        active_turn_id: &str,
        status: &str,
        created_at_ms: i64,
        started_at_ms: Option<i64>,
        finished_at_ms: Option<i64>,
        current_task_desc: Option<&str>,
        progress_updated_at_ms: Option<i64>,
        has_report: bool,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_async_tasks
              (task_id, session_id, ds_id, active_turn_id, status, created_at_ms,
               started_at_ms, finished_at_ms, current_task_desc, progress_updated_at_ms, has_report)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
              ON CONFLICT (task_id) DO UPDATE SET
                active_turn_id = EXCLUDED.active_turn_id,
                status = EXCLUDED.status,
                started_at_ms = EXCLUDED.started_at_ms,
                finished_at_ms = EXCLUDED.finished_at_ms,
                current_task_desc = EXCLUDED.current_task_desc,
                progress_updated_at_ms = EXCLUDED.progress_updated_at_ms,
                has_report = EXCLUDED.has_report",
        )
        .bind(task_id)
        .bind(session_id)
        .bind(ds_id)
        .bind(active_turn_id)
        .bind(status)
        .bind(created_at_ms)
        .bind(started_at_ms)
        .bind(finished_at_ms)
        .bind(current_task_desc)
        .bind(progress_updated_at_ms)
        .bind(has_report)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_async_task(&self, task_id: &str) -> Result<Option<AsyncTaskRow>, SqlxError> {
        sqlx::query_as::<_, AsyncTaskRow>(
            r"SELECT task_id, session_id, ds_id, active_turn_id, status, created_at_ms,
                    started_at_ms, finished_at_ms, current_task_desc, progress_updated_at_ms, has_report
              FROM gateway_async_tasks WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn list_turn_ids_for_session(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Vec<(String, Option<String>, i64)>, SqlxError> {
        sqlx::query_as(
            r"SELECT turn_id, user_prompt, created_at_ms FROM gateway_turns
              WHERE session_id = $1 AND ds_id = $2 ORDER BY created_at_ms ASC",
        )
        .bind(session_id)
        .bind(ds_id)
        .fetch_all(&self.pool)
        .await
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
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_sessions (session_id, ds_id, session_home, created_at_ms, updated_at_ms)
              VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(session_id)
        .bind(ds_id)
        .bind(session_home_rel)
        .bind(now_ms)
        .bind(now_ms)
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
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_turns
              (turn_id, session_id, ds_id, status, created_at_ms, finished_at_ms, user_prompt)
              VALUES ($1, $2, $3, $4, $5, NULL, $6)",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .bind(status)
        .bind(created_at_ms)
        .bind(user_prompt)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    async fn test_db() -> Option<GatewaySessionDb> {
        let url = std::env::var("CLAW_GATEWAY_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("CLAW_GATEWAY_DATABASE_URL"))
            .ok()?;
        GatewaySessionDb::connect(url.trim()).await.ok()
    }

    #[test]
    fn split_sql_statements_basic() {
        let parts = split_sql_statements("SELECT 1; SELECT 2;");
        assert_eq!(parts.len(), 2);
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

        db.insert_session("s1", 7, "ds_7/sessions/u1", now_ms())
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
        db.insert_session(&sid, 1, "a", t).await.unwrap();
        db.insert_session(&sid, 2, "b", t).await.unwrap();
        assert_eq!(
            db.get_session_home_rel(&sid, 1).await.unwrap().as_deref(),
            Some("a")
        );
        assert!(db.insert_session(&sid, 1, "c", t).await.is_err());
    }

    #[tokio::test]
    async fn turn_and_feedback_flow() {
        let Some(db) = test_db().await else {
            eprintln!("skip turn_and_feedback_flow: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let t = now_ms();
        let sid = format!("s1_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "ds_1/sessions/u1", t)
            .await
            .unwrap();
        db.insert_turn(
            "T_a1b2c3d4e5f6478990abcdef12345678",
            &sid,
            1,
            "queued",
            t,
            None,
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
}
