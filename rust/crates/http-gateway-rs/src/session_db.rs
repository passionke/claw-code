//! `PostgreSQL` persistence for gateway sessions, turns, and feedback. Author: kejiqing
//!
//! **Persistence split (see `docs/persistence-model.md`):** conversation jsonl remains the
//! runtime source of truth on disk; `gateway_turns` stores per-`turn_id` terminal snapshots
//! (`report_message`, `output_json`, …) so gateway restarts and `GET /v1/tasks` handoff stay
//! consistent at **turn** granularity.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{Error as SqlxError, PgPool, Row};

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
        ] {
            sqlx::query(ddl).execute(pool).await?;
        }
        Ok(())
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
            r"INSERT INTO gateway_turns (turn_id, session_id, ds_id, status, created_at_ms, finished_at_ms, user_prompt)
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
            Some("hello"),
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
        db.insert_session(&sid, 1, "ds_1/sessions/x", t)
            .await
            .unwrap();
        let tid1 = "T_10000000000000000000000000000001";
        let tid2 = "T_20000000000000000000000000000002";
        db.insert_turn(tid1, &sid, 1, "queued", t, Some("a"))
            .await
            .unwrap();
        db.insert_turn(tid2, &sid, 1, "queued", t + 100, Some("b"))
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
}
