//! `PostgreSQL` persistence for gateway sessions, turns, and feedback. Author: kejiqing

use std::collections::BTreeMap;

use sqlx::postgres::PgPoolOptions;
use sqlx::{Error as SqlxError, PgPool};

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
    ) -> Result<(), SqlxError> {
        sqlx::query(
            r"INSERT INTO gateway_turns (turn_id, session_id, ds_id, status, created_at_ms, finished_at_ms)
              VALUES ($1, $2, $3, $4, $5, NULL)",
        )
        .bind(turn_id)
        .bind(session_id)
        .bind(ds_id)
        .bind(status)
        .bind(created_at_ms)
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
        db.insert_turn("T_a1b2c3d4e5f6478990abcdef12345678", &sid, 1, "queued", t)
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
