//! Versioned PostgreSQL schema migrations via `sqlx::migrate!`.
//!
//! Empty DBs apply pending revisions from `migrations/`.
//! Legacy DBs (tables from the old replay-all migrator, no `_sqlx_migrations`)
//! are stamped at version 1 so baseline is not re-executed.
//!
//! Author: kejiqing

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::PgPool;
use sqlx::Error as SqlxError;

/// Embedded migrator (`migrations/*.sql`, sequential integer versions).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const MIGRATE_ADVISORY_LOCK: i64 = 0x434C_4157_4D49;

/// Run versioned migrations under the same advisory lock as the legacy migrator.
pub async fn run(pool: &PgPool) -> Result<(), SqlxError> {
    let mut conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATE_ADVISORY_LOCK)
        .execute(&mut *conn)
        .await?;
    let result = run_unlocked(pool).await;
    if sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATE_ADVISORY_LOCK)
        .execute(&mut *conn)
        .await
        .is_err()
    {
        conn.close().await.ok();
    }
    result
}

async fn run_unlocked(pool: &PgPool) -> Result<(), SqlxError> {
    stamp_legacy_baseline_if_needed(pool)
        .await
        .map_err(|e| SqlxError::Migrate(Box::new(e)))?;
    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| SqlxError::Migrate(Box::new(e)))?;
    Ok(())
}

/// If `gateway_sessions` already exists and no migration versions are recorded,
/// insert version 1 as applied (Alembic-style stamp). Author: kejiqing
async fn stamp_legacy_baseline_if_needed(pool: &PgPool) -> Result<(), MigrateError> {
    let has_sessions: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'gateway_sessions'
        )",
    )
    .fetch_one(pool)
    .await?;

    if !has_sessions {
        return Ok(());
    }

    ensure_sqlx_migrations_table(pool).await?;

    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM _sqlx_migrations")
        .fetch_one(pool)
        .await?;
    if applied > 0 {
        return Ok(());
    }

    let Some(baseline) = MIGRATOR.iter().find(|m| m.version == 1) else {
        return Err(MigrateError::VersionMissing(1));
    };

    sqlx::query(
        r"INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
          VALUES ($1, $2, TRUE, $3, 0)
          ON CONFLICT (version) DO NOTHING",
    )
    .bind(baseline.version)
    .bind(&*baseline.description)
    .bind(&*baseline.checksum)
    .execute(pool)
    .await?;

    Ok(())
}

async fn ensure_sqlx_migrations_table(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::query(
        r"CREATE TABLE IF NOT EXISTS _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
            success BOOLEAN NOT NULL,
            checksum BYTEA NOT NULL,
            execution_time BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrator_has_baseline_version_one() {
        let baseline = MIGRATOR
            .iter()
            .find(|m| m.version == 1)
            .expect("version 1 baseline must exist");
        assert!(
            baseline.description.contains("baseline"),
            "unexpected description: {}",
            baseline.description
        );
        assert!(!baseline.checksum.is_empty());
    }

    #[test]
    fn migrator_versions_are_sequential_integers_not_timestamps() {
        for m in MIGRATOR.iter() {
            assert!(
                m.version < 1_000_000,
                "migration version {} looks like a timestamp; use sequential integers",
                m.version
            );
        }
    }
}
