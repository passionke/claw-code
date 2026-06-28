//! `claw_pool` PostgreSQL operations — extracted from [`session_db`](super::session_db) for domain separation.
//! Author: kejiqing

use sqlx::{Error as SqlxError, PgPool, Row};

/// Payload for [`upsert_claw_pool`].
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

/// One row from [`list_claw_pools`].
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

/// Pool heartbeat fresh if within 120s.
#[must_use]
pub fn is_claw_pool_online(last_heartbeat_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_heartbeat_ms) < 120_000
}

/// Millisecond timestamp for pool registry.
#[must_use]
pub fn now_ms_for_registry() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Register or refresh a legacy `claw_pool` row.
pub async fn upsert_claw_pool(pg: &PgPool, row: &ClawPoolUpsert<'_>) -> Result<(), SqlxError> {
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
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn touch_claw_pool_heartbeat(
    pg: &PgPool,
    pool_id: &str,
    last_heartbeat_ms: i64,
) -> Result<(), SqlxError> {
    sqlx::query("UPDATE claw_pool SET last_heartbeat_ms = $2 WHERE pool_id = $1")
        .bind(pool_id)
        .bind(last_heartbeat_ms)
        .execute(pg)
        .await?;
    Ok(())
}

/// Delete a pool row only when heartbeat is stale (offline).
pub async fn delete_claw_pool_if_offline(
    pg: &PgPool,
    pool_id: &str,
    advertise_ip: &str,
    now_ms: i64,
) -> Result<bool, SqlxError> {
    let stale_before = now_ms.saturating_sub(120_000);
    let result = sqlx::query(
        "DELETE FROM claw_pool WHERE pool_id = $1 AND advertise_ip = $2 AND last_heartbeat_ms < $3",
    )
    .bind(pool_id)
    .bind(advertise_ip)
    .bind(stale_before)
    .execute(pg)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Admin: remove stale `claw_pool` row; pool-daemon re-registers on next start.
pub async fn delete_claw_pool(pg: &PgPool, pool_id: &str) -> Result<bool, SqlxError> {
    let result = sqlx::query("DELETE FROM claw_pool WHERE pool_id = $1")
        .bind(pool_id)
        .execute(pg)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// All registered pool nodes (multi-host observability).
pub async fn list_claw_pools(pg: &PgPool) -> Result<Vec<ClawPoolRow>, SqlxError> {
    let rows = sqlx::query(
        r"SELECT pool_id, registration_time_ms, slots_max, slots_min,
                 advertise_ip, sse_port, gateway_base, last_heartbeat_ms
          FROM claw_pool
          ORDER BY last_heartbeat_ms DESC, pool_id ASC",
    )
    .fetch_all(pg)
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
