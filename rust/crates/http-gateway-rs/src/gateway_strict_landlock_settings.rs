//! Gateway global strict Landlock default DSL (system config). Author: kejiqing

use gateway_solve_turn::{default_landlock_dsl, LandlockDsl};
use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::pool::validate_system_landlock_default;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrictLandlockDefaultPublic {
    #[serde(flatten)]
    pub dsl: LandlockDsl,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PutStrictLandlockDefaultInput {
    #[serde(flatten)]
    pub dsl: LandlockDsl,
}

#[derive(Debug, Clone, Serialize)]
pub struct PutStrictLandlockDefaultResponse {
    #[serde(rename = "strictLandlockDefault")]
    pub strict_landlock_default: LandlockDsl,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Effective system default from PG (falls back to code seed when unset).
pub async fn load_system_landlock_default(
    db: &GatewaySessionDb,
) -> Result<LandlockDsl, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings
        .strict_landlock_default
        .unwrap_or_else(default_landlock_dsl))
}

pub async fn put_strict_landlock_default(
    db: &GatewaySessionDb,
    input: PutStrictLandlockDefaultInput,
) -> Result<PutStrictLandlockDefaultResponse, String> {
    validate_system_landlock_default(&input.dsl)?;
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| format!("load global settings: {e}"))?;
    settings.strict_landlock_default = Some(input.dsl.clone());
    let updated_at_ms = now_ms();
    save_gateway_global_settings(db, &settings, &tokens, updated_at_ms)
        .await
        .map_err(|e| format!("save global settings: {e}"))?;
    Ok(PutStrictLandlockDefaultResponse {
        strict_landlock_default: input.dsl,
        updated_at_ms,
    })
}

/// Resolve effective Landlock DSL for a strict project.
pub async fn resolve_landlock_dsl_for_proj(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<Option<gateway_solve_turn::LandlockDsl>, String> {
    let worker_profile = db
        .get_worker_profile_json(proj_id)
        .await
        .map_err(|e| format!("load worker_profile_json for proj {proj_id}: {e}"))?;
    let system_default = load_system_landlock_default(db)
        .await
        .map_err(|e| format!("load system landlock default: {e}"))?;
    let resolved = gateway_solve_turn::resolve_landlock_dsl(&worker_profile, &system_default)?;
    Ok(resolved.map(|(dsl, _)| dsl))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_solve_turn::validate_landlock_dsl;

    #[test]
    fn default_validates() {
        validate_landlock_dsl(&default_landlock_dsl()).unwrap();
    }
}
