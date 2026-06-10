//! Thin PostgreSQL client for `claw_pool` registry only. Author: kejiqing

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};

#[derive(Debug, Clone)]
pub struct ClawPoolUpsert<'a> {
    pub pool_id: &'a str,
    pub registration_time_ms: i64,
    pub slots_max: i32,
    pub slots_min: i32,
    pub advertise_ip: &'a str,
    pub sse_port: i32,
    pub gateway_base: &'a str,
    pub last_heartbeat_ms: i64,
}

pub struct PoolRegistryDb {
    pool: PgPool,
}

impl PoolRegistryDb {
    pub async fn open() -> Result<Self, String> {
        let url = std::env::var("CLAW_GATEWAY_DATABASE_URL")
            .map_err(|_| "CLAW_GATEWAY_DATABASE_URL required for claw_pool registry".to_string())?;
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .map_err(|e| format!("registry db connect: {e}"))?;
        Ok(Self { pool })
    }

    pub async fn upsert_claw_pool(&self, row: &ClawPoolUpsert<'_>) -> Result<(), String> {
        sqlx::query(
            r"INSERT INTO claw_pool (
                pool_id, registration_time_ms, slots_max, slots_min,
                advertise_ip, sse_port, gateway_base, last_heartbeat_ms
              ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
              ON CONFLICT (pool_id) DO UPDATE SET
                slots_max = EXCLUDED.slots_max,
                slots_min = EXCLUDED.slots_min,
                advertise_ip = EXCLUDED.advertise_ip,
                sse_port = EXCLUDED.sse_port,
                gateway_base = EXCLUDED.gateway_base,
                last_heartbeat_ms = EXCLUDED.last_heartbeat_ms",
        )
        .bind(row.pool_id)
        .bind(row.registration_time_ms)
        .bind(row.slots_max)
        .bind(row.slots_min)
        .bind(row.advertise_ip)
        .bind(row.sse_port)
        .bind(row.gateway_base)
        .bind(row.last_heartbeat_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("upsert claw_pool: {e}"))?;
        Ok(())
    }

    pub async fn touch_claw_pool_heartbeat(
        &self,
        pool_id: &str,
        last_heartbeat_ms: i64,
    ) -> Result<(), String> {
        sqlx::query("UPDATE claw_pool SET last_heartbeat_ms = $2 WHERE pool_id = $1")
            .bind(pool_id)
            .bind(last_heartbeat_ms)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("touch claw_pool heartbeat: {e}"))?;
        Ok(())
    }

    pub async fn delete_claw_pool_if_offline(
        &self,
        pool_id: &str,
        advertise_ip: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let stale_before = now_ms.saturating_sub(120_000);
        let result = sqlx::query(
            "DELETE FROM claw_pool WHERE pool_id = $1 AND advertise_ip = $2 AND last_heartbeat_ms < $3",
        )
        .bind(pool_id)
        .bind(advertise_ip)
        .bind(stale_before)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("delete claw_pool: {e}"))?;
        Ok(result.rows_affected() > 0)
    }
}

#[must_use]
pub fn now_ms_for_registry() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

#[allow(dead_code)]
pub type RegistryTx<'a> = Transaction<'a, Postgres>;
