//! Multi-gateway shared PostgreSQL: `cluster_id` is the top-level data root. Author: kejiqing
//!
//! # Hierarchy
//!
//! ```text
//! cluster_id          ← CLAW_CLUSTER_ID (one gateway instance = one cluster)
//!   └─ proj_id        ← project within cluster (NAS: {cluster}/proj_{id}/)
//!        └─ session_id
//!             └─ turn_id
//!                  └─ cc_messages, artifacts, runtime_iterations, …
//! ```
//!
//! # Table tiers
//!
//! **Tier A — cluster-native (PK includes `cluster_id`)**
//! - `gateway_llm_cluster_model`, `gateway_llm_cluster_revision`, `gateway_llm_cluster_state`
//!
//! **Tier B — project/session tree (PK/uniqueness includes `cluster_id`)**
//! - Session: `gateway_sessions`, `gateway_turns`, `gateway_feedback`,
//!   `cc_messages`, `gateway_session_artifacts`, `gateway_runtime_iterations`,
//!   `gateway_conversation_translate`
//! - Project: `project_config`, `project_config_revision`, `project_entity_revision`
//! - Infra: `project_e2b_worker`, `worker_rotation_log`
//!
//! **Tier C — cluster-tagged operational (PK includes `cluster_id`)**
//! - `claw_pool` — pool registration; each row belongs to one gateway cluster
//! - `gateway_global_settings` — per-cluster row (`settings_json` + LLM cluster tables)
//!
//! **Tier D — global (no `cluster_id`)**
//! - `preflight_plugin` — installable plugin catalog, shared across clusters
//! - `gateway_llm_model_revision` — legacy single-tenant LLM revisions (superseded by tier A)
//!
//! # Runtime contract
//!
//! - [`GatewaySessionDb`] binds `cluster_id` at connect from [`crate::cluster_identity::gateway_cluster_id`].
//! - All reads/writes on tier B/C tables MUST filter or set `cluster_id = db.cluster_id()`.
//! - Rows that cannot inherit a cluster during migration are isolated under `__legacy__`.
//! - NAS paths: `{cluster_id}/proj_{proj_id}/sessions/{segment}` — backfill derives cluster from prefix.

use sqlx::Error as SqlxError;

/// Resolve this gateway's cluster root (required for tier B/C persistence). Author: kejiqing
pub fn resolve_gateway_cluster_id() -> Result<String, SqlxError> {
    crate::cluster_identity::gateway_cluster_id().map_err(|e| {
        SqlxError::Configuration(format!("CLAW_CLUSTER_ID is required for shared PG: {e}").into())
    })
}

/// Like [`resolve_gateway_cluster_id`] but integration tests default to `test-cluster`. Author: kejiqing
pub(crate) fn resolve_gateway_cluster_id_for_connect() -> Result<String, SqlxError> {
    if let Ok(raw) = std::env::var("CLAW_CLUSTER_ID") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return crate::cluster_identity::validate_cluster_id(trimmed)
                .map_err(|e| SqlxError::Configuration(format!("CLAW_CLUSTER_ID: {e}").into()))
                .map(|()| trimmed.to_string());
        }
    }
    #[cfg(test)]
    {
        return Ok("test-cluster".to_string());
    }
    #[cfg(not(test))]
    {
        resolve_gateway_cluster_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_for_connect_defaults_in_test_builds() {
        let saved = std::env::var("CLAW_CLUSTER_ID").ok();
        std::env::remove_var("CLAW_CLUSTER_ID");
        assert_eq!(
            resolve_gateway_cluster_id_for_connect().unwrap(),
            "test-cluster"
        );
        if let Some(v) = saved {
            std::env::set_var("CLAW_CLUSTER_ID", v);
        }
    }
}
