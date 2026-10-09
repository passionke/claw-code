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

    /// sqlx stores SHA-384(sql bytes) in `_sqlx_migrations.checksum`.
    /// Once a version is released / applied, its `migrations/N_*.sql` is immutable:
    /// editing it fails upgrades with "migration N was previously applied but has been modified".
    /// Add schema changes only as a new `N+1_*.sql`, then append that checksum here.
    /// Author: kejiqing
    const PINNED_MIGRATION_CHECKSUMS: &[(i64, &str)] = &[
        (
            1,
            "b3ac2736f281ddec6a1dc41050637d1db5db5975d4f4fdcc9b3f9aa80d42d8e843b07bcc5ce54b3cc147a091ea6b4254",
        ),
        (
            2,
            "1e569a9b928a6d2193db51ce1b0e594083e7c55d8e31fd7eda45e3db73e9517695dd4ec74995ae90d72fce5b06df5cab",
        ),
        (
            3,
            "be3406c2f930160f78684fda685c0e53fee2c07db0d5a7a2c7f73aa782f0ee523ad944b189355ccb97fe91d337280d38",
        ),
        (
            4,
            "e236a6bb8ae80e7c2a9de18d8e8f49c872772beaa41efd9dd37ba4fad79c70baea75a8f9062f404db13e102b1c47b035",
        ),
        (
            5,
            "c0bc061983d68738f76ac31b4d745cc420ecd1e82b27513de17d924059e7d8de08c2250a11bbc4d997d7502ae2529a56",
        ),
        (
            6,
            "8b735e481b10640e11ab83e9dd1731c8ff46293804cabc41bdf60b45daf56cd5d2169d89f54e576bf466fc6e293dfe66",
        ),
        (
            7,
            "5c6d5acc86cf97244b8925949415f2654d7abd66a74cdc766bcd7f6467f785a93e1558c00bc83669e5245e44e049e22d",
        ),
        (
            8,
            "f779344e2aeaeeb5ab4b4dfe4184c6594f16085c822fe4aca294ae3bb9cd8eaeab50972b14c8758588783440a18e0eb0",
        ),
        (
            9,
            "8539b16282ddedc13b26d363b072c6e9e5cf079d3c8da0a700674bc495edfccc8881ec9386b426216ef2ffc4c0a8387e",
        ),
    ];

    fn checksum_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

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

    #[test]
    fn published_migration_checksums_are_pinned() {
        let mut by_version = std::collections::BTreeMap::new();
        for m in MIGRATOR.iter() {
            assert!(
                by_version.insert(m.version, m).is_none(),
                "duplicate migration version {}",
                m.version
            );
        }

        assert_eq!(
            by_version.len(),
            PINNED_MIGRATION_CHECKSUMS.len(),
            "migrator has {} versions but PINNED_MIGRATION_CHECKSUMS has {}; \
             add new migrations as N+1 and append their SHA-384 checksum (do not edit older files)",
            by_version.len(),
            PINNED_MIGRATION_CHECKSUMS.len()
        );

        for &(version, expected_hex) in PINNED_MIGRATION_CHECKSUMS {
            let m = by_version
                .get(&version)
                .unwrap_or_else(|| panic!("pinned version {version} missing from migrator"));
            let actual = checksum_hex(&m.checksum);
            assert_eq!(
                actual, expected_hex,
                "migration {version} SQL was modified after release; \
                 revert the file and ship schema changes as a new versioned migration instead"
            );
        }

        let versions: Vec<i64> = by_version.keys().copied().collect();
        let expected: Vec<i64> = (1..=PINNED_MIGRATION_CHECKSUMS.len() as i64).collect();
        assert_eq!(
            versions, expected,
            "migration versions must be contiguous 1..=N with no gaps"
        );
    }
}
