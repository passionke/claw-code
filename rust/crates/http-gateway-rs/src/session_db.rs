//! `SQLite` persistence for gateway `sessionId` → workspace path. Author: kejiqing

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Error as SqlxError, SqlitePool};

/// Gateway session index: one row per `(session_id, ds_id)` with a workspace-relative `session_home`.
pub struct GatewaySessionDb {
    pool: SqlitePool,
    path: PathBuf,
}

impl GatewaySessionDb {
    /// Opens or creates the `SQLite` file. Uses `CLAW_GATEWAY_SESSION_DB` when set (absolute path
    /// recommended for host persistence); otherwise `work_root/gateway-sessions.sqlite` so it
    /// stays on the same volume as `CLAW_WORK_ROOT` when that is bind-mounted.
    pub async fn open(work_root: &Path) -> Result<Self, SqlxError> {
        let db_path = std::env::var("CLAW_GATEWAY_SESSION_DB")
            .ok()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| work_root.join("gateway-sessions.sqlite"));
        Self::open_at_path(&db_path).await
    }

    /// Opens or creates the session index at `db_path` (creates parent directories). Ignores
    /// `CLAW_GATEWAY_SESSION_DB` (tests and deterministic deployments).
    pub async fn open_at_path(db_path: &Path) -> Result<Self, SqlxError> {
        let db_path = db_path.to_path_buf();
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(SqlxError::Io)?;
        }

        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;

        sqlx::query("PRAGMA foreign_keys = ON;")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA journal_mode = WAL;")
            .execute(&pool)
            .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS gateway_sessions (
                session_id TEXT NOT NULL,
                ds_id INTEGER NOT NULL,
                session_home TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY (session_id, ds_id)
            );",
        )
        .execute(&pool)
        .await?;

        Ok(Self {
            pool,
            path: db_path,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn get_session_home_rel(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar::<_, String>(
            "SELECT session_home FROM gateway_sessions WHERE session_id = ? AND ds_id = ?",
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
              VALUES (?, ?, ?, ?, ?)",
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
            "UPDATE gateway_sessions SET updated_at_ms = ? WHERE session_id = ? AND ds_id = ?",
        )
        .bind(now_ms)
        .bind(session_id)
        .bind(ds_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[cfg(test)]
    async fn fetch_updated_at_ms_for_test(
        &self,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<i64>, SqlxError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT updated_at_ms FROM gateway_sessions WHERE session_id = ? AND ds_id = ?",
        )
        .bind(session_id)
        .bind(ds_id)
        .fetch_optional(&self.pool)
        .await
    }
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

    #[tokio::test]
    async fn insert_get_touch_flow() {
        let dir =
            std::env::temp_dir().join(format!("claw_gw_sess_{}", uuid::Uuid::new_v4().simple()));
        let db_file = dir.join("idx.sqlite");
        let db = GatewaySessionDb::open_at_path(&db_file).await.unwrap();
        assert_eq!(db.path(), db_file.as_path());

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
        assert_eq!(
            db.get_session_home_rel("s1", 7).await.unwrap().as_deref(),
            Some("ds_7/sessions/u1")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn primary_key_per_ds_id() {
        let dir =
            std::env::temp_dir().join(format!("claw_gw_sess_pk_{}", uuid::Uuid::new_v4().simple()));
        let db_file = dir.join("idx.sqlite");
        let db = GatewaySessionDb::open_at_path(&db_file).await.unwrap();
        let t = now_ms();
        db.insert_session("same_sid", 1, "a", t).await.unwrap();
        db.insert_session("same_sid", 2, "b", t).await.unwrap();
        assert_eq!(
            db.get_session_home_rel("same_sid", 1)
                .await
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            db.get_session_home_rel("same_sid", 2)
                .await
                .unwrap()
                .as_deref(),
            Some("b")
        );

        assert!(db.insert_session("same_sid", 1, "c", t).await.is_err());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
