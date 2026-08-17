//! Router delegate targets + session link persistence. Author: kejiqing

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Error as SqlxError, Row};
use uuid::Uuid;

use crate::master_observer::PROJECT_ROLE_ROUTER;
use crate::session_db::{now_ms_for_registry, GatewaySessionDb, ProjectConfigRow};
use crate::session_merge;

const SPECIALIST_REGISTRY_SKILL: &str = "specialist-registry";
const REGISTRY_APPENDIX_MARKER: &str = "## Active delegate targets (auto-generated on activate)";

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GatewayDelegateTargetRow {
    pub initiator_proj_id: i64,
    pub target_proj_id: i64,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_hint: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DelegateTargetSpec {
    #[serde(rename = "targetProjId")]
    pub target_proj_id: i64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default, rename = "capabilityHint")]
    pub capability_hint: Option<String>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PutDelegateTargetsRequest {
    pub targets: Vec<DelegateTargetSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DelegateTargetsResponse {
    pub initiator_proj_id: i64,
    pub targets: Vec<GatewayDelegateTargetRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDelegateSessionRequest {
    pub parent_session_id: String,
    pub delegate_proj_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDelegateSessionResponse {
    pub delegate_session_id: String,
    pub root_session_id: String,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub struct GatewayDelegateSessionLinkRow {
    pub root_session_id: String,
    pub parent_session_id: String,
    pub parent_proj_id: i64,
    pub delegate_proj_id: i64,
    pub delegate_session_id: String,
}

impl GatewaySessionDb {
    pub async fn list_delegate_targets(
        &self,
        initiator_proj_id: i64,
    ) -> Result<Vec<GatewayDelegateTargetRow>, SqlxError> {
        let rows = sqlx::query(
            r"SELECT initiator_proj_id, target_proj_id, enabled, label, capability_hint,
                     created_at_ms, updated_at_ms
              FROM gateway_delegate_target
              WHERE cluster_id = $1 AND initiator_proj_id = $2
              ORDER BY target_proj_id",
        )
        .bind(self.cluster_id())
        .bind(initiator_proj_id)
        .fetch_all(self.pg_pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| GatewayDelegateTargetRow {
                initiator_proj_id: r.get("initiator_proj_id"),
                target_proj_id: r.get("target_proj_id"),
                enabled: r.get("enabled"),
                label: r.get("label"),
                capability_hint: r.get("capability_hint"),
                created_at_ms: r.get("created_at_ms"),
                updated_at_ms: r.get("updated_at_ms"),
            })
            .collect())
    }

    pub async fn replace_delegate_targets(
        &self,
        initiator_proj_id: i64,
        targets: &[DelegateTargetSpec],
    ) -> Result<(), SqlxError> {
        let now = now_ms_for_registry();
        let mut tx = self.pg_pool().begin().await?;
        sqlx::query(
            "DELETE FROM gateway_delegate_target WHERE cluster_id = $1 AND initiator_proj_id = $2",
        )
        .bind(self.cluster_id())
        .bind(initiator_proj_id)
        .execute(&mut *tx)
        .await?;
        for t in targets {
            sqlx::query(
                r"INSERT INTO gateway_delegate_target (
                    cluster_id, initiator_proj_id, target_proj_id, enabled, label,
                    capability_hint, created_at_ms, updated_at_ms
                  ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(self.cluster_id())
            .bind(initiator_proj_id)
            .bind(t.target_proj_id)
            .bind(t.enabled)
            .bind(t.label.as_deref())
            .bind(t.capability_hint.as_deref())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_delegate_target(
        &self,
        initiator_proj_id: i64,
        target_proj_id: i64,
    ) -> Result<Option<GatewayDelegateTargetRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT initiator_proj_id, target_proj_id, enabled, label, capability_hint,
                     created_at_ms, updated_at_ms
              FROM gateway_delegate_target
              WHERE cluster_id = $1 AND initiator_proj_id = $2 AND target_proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(initiator_proj_id)
        .bind(target_proj_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(row.map(|r| GatewayDelegateTargetRow {
            initiator_proj_id: r.get("initiator_proj_id"),
            target_proj_id: r.get("target_proj_id"),
            enabled: r.get("enabled"),
            label: r.get("label"),
            capability_hint: r.get("capability_hint"),
            created_at_ms: r.get("created_at_ms"),
            updated_at_ms: r.get("updated_at_ms"),
        }))
    }

    pub async fn get_delegate_session_link(
        &self,
        parent_session_id: &str,
        parent_proj_id: i64,
        delegate_proj_id: i64,
    ) -> Result<Option<GatewayDelegateSessionLinkRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT root_session_id, parent_session_id, parent_proj_id, delegate_proj_id,
                     delegate_session_id
              FROM gateway_delegate_session_link
              WHERE cluster_id = $1 AND parent_session_id = $2
                AND parent_proj_id = $3 AND delegate_proj_id = $4",
        )
        .bind(self.cluster_id())
        .bind(parent_session_id)
        .bind(parent_proj_id)
        .bind(delegate_proj_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(row.map(|r| GatewayDelegateSessionLinkRow {
            root_session_id: r.get("root_session_id"),
            parent_session_id: r.get("parent_session_id"),
            parent_proj_id: r.get("parent_proj_id"),
            delegate_proj_id: r.get("delegate_proj_id"),
            delegate_session_id: r.get("delegate_session_id"),
        }))
    }

    /// Resolve root anchor for nested delegate (parent may be a prior delegate session). Author: kejiqing
    pub async fn resolve_delegate_root_session_id(
        &self,
        parent_session_id: &str,
        parent_proj_id: i64,
    ) -> Result<String, SqlxError> {
        let by_delegate = sqlx::query_scalar::<_, String>(
            r"SELECT root_session_id FROM gateway_delegate_session_link
              WHERE cluster_id = $1 AND delegate_session_id = $2
              LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(parent_session_id)
        .fetch_optional(self.pg_pool())
        .await?;
        if let Some(root) = by_delegate {
            return Ok(root);
        }
        let role = self.get_project_role(parent_proj_id).await?;
        if role == PROJECT_ROLE_ROUTER {
            return Ok(parent_session_id.to_string());
        }
        Ok(parent_session_id.to_string())
    }

    pub async fn resolve_or_create_delegate_session(
        &self,
        initiator_proj_id: i64,
        parent_session_id: &str,
        delegate_proj_id: i64,
        client_origin: Option<&str>,
    ) -> Result<(String, String, bool), String> {
        if let Some(link) = self
            .get_delegate_session_link(parent_session_id, initiator_proj_id, delegate_proj_id)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok((link.delegate_session_id, link.root_session_id, false));
        }
        let root = self
            .resolve_delegate_root_session_id(parent_session_id, initiator_proj_id)
            .await
            .map_err(|e| e.to_string())?;
        let delegate_session_id = format!("dgt_{}", Uuid::new_v4().simple());
        let seg = session_merge::sessions_directory_segment(&delegate_session_id);
        let session_home_rel = format!("proj_{delegate_proj_id}/sessions/{seg}");
        let now = now_ms_for_registry();
        self.insert_session(
            &delegate_session_id,
            delegate_proj_id,
            &session_home_rel,
            now,
            client_origin,
        )
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query(
            r"INSERT INTO gateway_delegate_session_link (
                cluster_id, root_session_id, parent_session_id, parent_proj_id,
                delegate_proj_id, delegate_session_id, created_at_ms, updated_at_ms
              ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(self.cluster_id())
        .bind(&root)
        .bind(parent_session_id)
        .bind(initiator_proj_id)
        .bind(delegate_proj_id)
        .bind(&delegate_session_id)
        .bind(now)
        .bind(now)
        .execute(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok((delegate_session_id, root, true))
    }

    pub async fn assert_delegate_target_allowed(
        &self,
        initiator_proj_id: i64,
        target_proj_id: i64,
    ) -> Result<(), String> {
        let row = self
            .get_delegate_target(initiator_proj_id, target_proj_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!("target projId={target_proj_id} not registered for initiator {initiator_proj_id}")
            })?;
        if !row.enabled {
            return Err(format!("target projId={target_proj_id} is disabled"));
        }
        let role = self
            .get_project_role(target_proj_id)
            .await
            .map_err(|e| e.to_string())?;
        if role != crate::master_observer::PROJECT_ROLE_NORMAL {
            return Err(format!(
                "target projId={target_proj_id} must have project_role=normal (got {role})"
            ));
        }
        Ok(())
    }
}

/// Markdown appendix listing delegate targets for specialist-registry skill. Author: kejiqing
#[must_use]
pub fn build_delegate_targets_registry_appendix(targets: &[GatewayDelegateTargetRow]) -> String {
    let mut s = format!("\n\n{REGISTRY_APPENDIX_MARKER}\n\n");
    if targets.is_empty() {
        s.push_str("_(none registered — configure Admin delegate-targets)_\n");
        return s;
    }
    s.push_str("| targetProjId | label | capabilityHint | enabled |\n");
    s.push_str("|--------------|-------|----------------|--------|\n");
    for t in targets {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} |",
            t.target_proj_id,
            t.label.as_deref().unwrap_or("-"),
            t.capability_hint.as_deref().unwrap_or("-"),
            if t.enabled { "yes" } else { "no" },
        );
    }
    s
}

/// Append registry appendix to specialist-registry skill content (idempotent on re-activate). Author: kejiqing
#[must_use]
pub fn merge_specialist_registry_appendix(skills_json: &Value, appendix: &str) -> Value {
    let Some(arr) = skills_json.as_array() else {
        return skills_json.clone();
    };
    let mut out: Vec<Value> = arr.clone();
    for item in &mut out {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        if obj.get("skillName").and_then(Value::as_str) != Some(SPECIALIST_REGISTRY_SKILL) {
            continue;
        }
        let content = obj
            .get("skillContent")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let base = content
            .split(REGISTRY_APPENDIX_MARKER)
            .next()
            .unwrap_or(content);
        obj.insert(
            "skillContent".into(),
            Value::String(format!("{}{appendix}", base.trim_end())),
        );
    }
    Value::Array(out)
}

/// Ensure router materialize exposes delegate_project (symmetric to master MCP injection). Author: kejiqing
#[must_use]
pub fn ensure_delegate_project_allowed_tools(allowed_tools_json: &Value) -> Value {
    let mut tools: Vec<String> =
        serde_json::from_value(allowed_tools_json.clone()).unwrap_or_default();
    if !tools.iter().any(|t| t == "delegate_project") {
        tools.push("delegate_project".to_string());
    }
    json!(tools)
}

/// Inject delegate-target registry appendix + delegate_project tool for router role at materialize. Author: kejiqing
pub async fn prepare_router_materialize_row(
    db: &GatewaySessionDb,
    proj_id: i64,
    row: ProjectConfigRow,
) -> Result<ProjectConfigRow, SqlxError> {
    let role = db.get_project_role(proj_id).await?;
    if role != PROJECT_ROLE_ROUTER {
        return Ok(row);
    }
    let targets = db.list_delegate_targets(proj_id).await?;
    let appendix = build_delegate_targets_registry_appendix(&targets);
    let mut row = row;
    row.skills_json = merge_specialist_registry_appendix(&row.skills_json, &appendix);
    row.allowed_tools_json = ensure_delegate_project_allowed_tools(&row.allowed_tools_json);
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_appendix_lists_enabled_targets() {
        let appendix = build_delegate_targets_registry_appendix(&[GatewayDelegateTargetRow {
            initiator_proj_id: 1,
            target_proj_id: 271,
            enabled: true,
            label: Some("ops".into()),
            capability_hint: Some("analytics".into()),
            created_at_ms: 0,
            updated_at_ms: 0,
        }]);
        assert!(appendix.contains("| 271 | ops | analytics | yes |"));
    }

    #[test]
    fn merge_registry_appendix_is_idempotent() {
        let skills = json!([{
            "skillName": "specialist-registry",
            "skillContent": "# base\n",
            "enabled": true
        }]);
        let appendix = build_delegate_targets_registry_appendix(&[]);
        let once = merge_specialist_registry_appendix(&skills, &appendix);
        let twice = merge_specialist_registry_appendix(&once, &appendix);
        assert_eq!(once, twice);
        assert!(once[0]["skillContent"]
            .as_str()
            .unwrap()
            .contains(REGISTRY_APPENDIX_MARKER));
    }

    #[test]
    fn ensure_delegate_project_tool_present() {
        let out = ensure_delegate_project_allowed_tools(&json!(["Skill"]));
        assert!(out
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "delegate_project"));
    }
}
