//! Router delegate targets + session link persistence. Author: kejiqing

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Error as SqlxError, Row};
use uuid::Uuid;

use crate::master_observer::PROJECT_ROLE_ROUTER;
use crate::project_relation::RELATION_TYPE_ROUTER_DELEGATE;
use crate::session_db::{now_ms_for_registry, GatewaySessionDb, ProjectConfigRow};
use crate::session_merge;

const SPECIALIST_REGISTRY_SKILL: &str = "specialist-registry";
const REGISTRY_APPENDIX_MARKER: &str = "## Active delegate targets (auto-generated on activate)";
/// Cluster-scoped advisory lock for delegate-graph mutation. Author: kejiqing
const DELEGATE_GRAPH_ADVISORY_NS: i64 = 0x4445_4C45_4741; // "DELEGA"

pub const BODY_RELAY_PASSTHROUGH: &str = "passthrough";
pub const BODY_RELAY_PROGRESS: &str = "progress";

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

fn default_body_relay() -> String {
    BODY_RELAY_PASSTHROUGH.to_string()
}

/// Normalize / validate bodyRelay. Author: kejiqing
pub fn parse_body_relay(raw: Option<&str>) -> Result<&'static str, String> {
    match raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(BODY_RELAY_PASSTHROUGH)
    {
        BODY_RELAY_PASSTHROUGH => Ok(BODY_RELAY_PASSTHROUGH),
        BODY_RELAY_PROGRESS => Ok(BODY_RELAY_PROGRESS),
        other => Err(format!(
            "invalid bodyRelay={other:?}; expected passthrough|progress"
        )),
    }
}

/// Read bodyRelay from router_json (default passthrough). Author: kejiqing
#[must_use]
pub fn body_relay_from_router_json(router_json: &Value) -> &'static str {
    parse_body_relay(router_json.get("bodyRelay").and_then(Value::as_str))
        .unwrap_or(BODY_RELAY_PASSTHROUGH)
}

#[must_use]
pub fn role_allows_delegate_target(role: &str) -> bool {
    matches!(
        role.trim(),
        crate::master_observer::PROJECT_ROLE_NORMAL
            | crate::master_observer::PROJECT_ROLE_KNOWLEDGE_BASE
            | crate::master_observer::PROJECT_ROLE_ROUTER
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PutDelegateTargetsRequest {
    pub targets: Vec<DelegateTargetSpec>,
    #[serde(default = "default_body_relay")]
    pub body_relay: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DelegateTargetsResponse {
    pub initiator_proj_id: i64,
    pub targets: Vec<GatewayDelegateTargetRow>,
    pub body_relay: String,
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
    pub body_relay: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Directed edge for cycle detection. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegateEdge {
    pub from: i64,
    pub to: i64,
}

/// True if adding `from → to` would create a cycle given existing enabled edges
/// (self-loop, or path `to ↝ from` already exists). Author: kejiqing
#[must_use]
pub fn would_introduce_cycle(existing: &[DelegateEdge], from: i64, to: i64) -> bool {
    find_cycle_after_add(existing, from, to).is_some()
}

/// If `from → to` introduces a cycle, return a path like `[from, to, …, from]`. Author: kejiqing
#[must_use]
pub fn find_cycle_after_add(existing: &[DelegateEdge], from: i64, to: i64) -> Option<Vec<i64>> {
    if from == to {
        return Some(vec![from, to]);
    }
    let mut adj: HashMap<i64, Vec<i64>> = HashMap::new();
    for e in existing {
        adj.entry(e.from).or_default().push(e.to);
    }
    // Path to → … → from in the graph *before* adding from→to.
    let mut parent: HashMap<i64, i64> = HashMap::new();
    let mut q = VecDeque::new();
    q.push_back(to);
    let mut seen = HashSet::from([to]);
    while let Some(n) = q.pop_front() {
        if n == from {
            // Reconstruct to → … → from, then prepend `from` for from → to → … → from.
            let mut mid = vec![from];
            let mut walk = from;
            while walk != to {
                walk = *parent.get(&walk)?;
                mid.push(walk);
            }
            mid.reverse();
            let mut path = vec![from];
            path.extend(mid);
            return Some(path);
        }
        if let Some(nexts) = adj.get(&n) {
            for &nxt in nexts {
                if seen.insert(nxt) {
                    parent.insert(nxt, n);
                    q.push_back(nxt);
                }
            }
        }
    }
    None
}

/// Any cycle in the full edge set; returns one cycle path if found. Author: kejiqing
#[must_use]
pub fn find_any_cycle(edges: &[DelegateEdge]) -> Option<Vec<i64>> {
    let mut adj: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut nodes = HashSet::new();
    for e in edges {
        if e.from == e.to {
            return Some(vec![e.from, e.to]);
        }
        adj.entry(e.from).or_default().push(e.to);
        nodes.insert(e.from);
        nodes.insert(e.to);
    }
    // 0=unseen, 1=in stack, 2=done
    let mut color: HashMap<i64, u8> = HashMap::new();
    let mut stack: Vec<i64> = Vec::new();
    fn dfs(
        n: i64,
        adj: &HashMap<i64, Vec<i64>>,
        color: &mut HashMap<i64, u8>,
        stack: &mut Vec<i64>,
    ) -> Option<Vec<i64>> {
        color.insert(n, 1);
        stack.push(n);
        if let Some(nexts) = adj.get(&n) {
            for &nxt in nexts {
                match color.get(&nxt).copied().unwrap_or(0) {
                    1 => {
                        let idx = stack.iter().position(|&x| x == nxt).unwrap_or(0);
                        let mut path: Vec<i64> = stack[idx..].to_vec();
                        path.push(nxt);
                        return Some(path);
                    }
                    0 => {
                        if let Some(p) = dfs(nxt, adj, color, stack) {
                            return Some(p);
                        }
                    }
                    _ => {}
                }
            }
        }
        stack.pop();
        color.insert(n, 2);
        None
    }
    for n in nodes {
        if color.get(&n).copied().unwrap_or(0) == 0 {
            if let Some(p) = dfs(n, &adj, &mut color, &mut stack) {
                return Some(p);
            }
        }
    }
    None
}

fn format_cycle_path(path: &[i64]) -> String {
    path.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("→")
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
            r"SELECT relation_type, from_proj_id, to_proj_id, relation_label, relation_meta_json,
                     created_at_ms, updated_at_ms
              FROM project_relation
              WHERE cluster_id = $1 AND relation_type = $2 AND from_proj_id = $3
              ORDER BY to_proj_id",
        )
        .bind(self.cluster_id())
        .bind(RELATION_TYPE_ROUTER_DELEGATE)
        .bind(initiator_proj_id)
        .fetch_all(self.pg_pool())
        .await?;
        Ok(rows.iter().map(delegate_target_row_from_relation).collect())
    }

    /// Replace initiator targets with cycle check under advisory xact lock. Author: kejiqing
    pub async fn replace_delegate_targets(
        &self,
        initiator_proj_id: i64,
        targets: &[DelegateTargetSpec],
    ) -> Result<(), String> {
        let now = now_ms_for_registry();
        let mut tx = self.pg_pool().begin().await.map_err(|e| e.to_string())?;
        // Serialize graph mutations per cluster. Author: kejiqing
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(DELEGATE_GRAPH_ADVISORY_NS)
            .bind(delegate_graph_lock_key(self.cluster_id()))
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        let existing_rows = sqlx::query(
            r"SELECT from_proj_id, to_proj_id, relation_meta_json
              FROM project_relation
              WHERE cluster_id = $1 AND relation_type = $2",
        )
        .bind(self.cluster_id())
        .bind(RELATION_TYPE_ROUTER_DELEGATE)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let mut other_edges = Vec::new();
        for r in &existing_rows {
            let from: i64 = r.get("from_proj_id");
            if from == initiator_proj_id {
                continue;
            }
            let meta: Option<sqlx::types::Json<Value>> = r.get("relation_meta_json");
            let meta = meta
                .map(|sqlx::types::Json(v)| v)
                .unwrap_or_else(|| json!({}));
            let enabled = meta.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if enabled {
                other_edges.push(DelegateEdge {
                    from,
                    to: r.get("to_proj_id"),
                });
            }
        }

        let mut proposed = Vec::new();
        for t in targets {
            if t.target_proj_id == initiator_proj_id {
                return Err("cannot delegate to self".into());
            }
            if t.enabled {
                proposed.push(DelegateEdge {
                    from: initiator_proj_id,
                    to: t.target_proj_id,
                });
            }
        }

        let mut combined = other_edges;
        combined.extend(proposed.iter().copied());
        if let Some(path) = find_any_cycle(&combined) {
            return Err(format!(
                "delegate graph cycle: {}",
                format_cycle_path(&path)
            ));
        }

        sqlx::query(
            r"DELETE FROM project_relation
              WHERE cluster_id = $1 AND relation_type = $2 AND from_proj_id = $3",
        )
        .bind(self.cluster_id())
        .bind(RELATION_TYPE_ROUTER_DELEGATE)
        .bind(initiator_proj_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        for t in targets {
            let meta = json!({
                "enabled": t.enabled,
                "capabilityHint": t.capability_hint
            });
            sqlx::query(
                r"INSERT INTO project_relation (
                    cluster_id, relation_type, from_proj_id, to_proj_id, relation_label,
                    relation_meta_json, created_at_ms, updated_at_ms
                  ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(self.cluster_id())
            .bind(RELATION_TYPE_ROUTER_DELEGATE)
            .bind(initiator_proj_id)
            .bind(t.target_proj_id)
            .bind(t.label.as_deref())
            .bind(sqlx::types::Json(&meta))
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_delegate_target(
        &self,
        initiator_proj_id: i64,
        target_proj_id: i64,
    ) -> Result<Option<GatewayDelegateTargetRow>, SqlxError> {
        let row = sqlx::query(
            r"SELECT relation_type, from_proj_id, to_proj_id, relation_label, relation_meta_json,
                     created_at_ms, updated_at_ms
              FROM project_relation
              WHERE cluster_id = $1 AND relation_type = $2 AND from_proj_id = $3 AND to_proj_id = $4",
        )
        .bind(self.cluster_id())
        .bind(RELATION_TYPE_ROUTER_DELEGATE)
        .bind(initiator_proj_id)
        .bind(target_proj_id)
        .fetch_optional(self.pg_pool())
        .await?;
        Ok(row.as_ref().map(delegate_target_row_from_relation))
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
        if !role_allows_delegate_target(&role) {
            return Err(format!(
                "target projId={target_proj_id} must have project_role=normal|knowledge_base|router (got {role})"
            ));
        }
        Ok(())
    }
}

fn delegate_graph_lock_key(cluster_id: &str) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cluster_id.hash(&mut hasher);
    i64::from_ne_bytes(hasher.finish().to_ne_bytes())
}

fn delegate_target_row_from_relation(r: &sqlx::postgres::PgRow) -> GatewayDelegateTargetRow {
    let meta: Option<sqlx::types::Json<Value>> = r.get("relation_meta_json");
    let meta = meta
        .map(|sqlx::types::Json(v)| v)
        .unwrap_or_else(|| json!({}));
    GatewayDelegateTargetRow {
        initiator_proj_id: r.get("from_proj_id"),
        target_proj_id: r.get("to_proj_id"),
        enabled: meta.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        label: r.get("relation_label"),
        capability_hint: meta
            .get("capabilityHint")
            .and_then(Value::as_str)
            .map(str::to_string),
        created_at_ms: r.get("created_at_ms"),
        updated_at_ms: r.get("updated_at_ms"),
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

/// Ensure router materialize exposes delegate + finish tools (migrate legacy name). Author: kejiqing
#[must_use]
pub fn ensure_delegate_project_allowed_tools(allowed_tools_json: &Value) -> Value {
    let mut tools: Vec<String> =
        serde_json::from_value(allowed_tools_json.clone()).unwrap_or_default();
    tools.retain(|t| t != "delegate_project");
    if !tools.iter().any(|t| t == "delegate_project_tool") {
        tools.push("delegate_project_tool".to_string());
    }
    if !tools.iter().any(|t| t == "complete_router_turn") {
        tools.push("complete_router_turn".to_string());
    }
    json!(tools)
}

/// Inject delegate-target registry appendix + delegate_project_tool for router role at materialize. Author: kejiqing
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

    fn e(from: i64, to: i64) -> DelegateEdge {
        DelegateEdge { from, to }
    }

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
        let arr = out.as_array().unwrap();
        assert!(arr.iter().any(|v| v == "delegate_project_tool"));
        assert!(arr.iter().any(|v| v == "complete_router_turn"));
    }

    #[test]
    fn role_allows_router_as_target() {
        assert!(role_allows_delegate_target("router"));
        assert!(role_allows_delegate_target("normal"));
        assert!(role_allows_delegate_target("knowledge_base"));
        assert!(!role_allows_delegate_target("master"));
    }

    #[test]
    fn cycle_self_loop() {
        assert!(would_introduce_cycle(&[], 1, 1));
        assert_eq!(find_cycle_after_add(&[], 1, 1), Some(vec![1, 1]));
    }

    #[test]
    fn cycle_two_node() {
        let existing = [e(1, 2)];
        assert!(would_introduce_cycle(&existing, 2, 1));
        assert!(!would_introduce_cycle(&existing, 2, 3));
    }

    #[test]
    fn cycle_legal_chain() {
        let edges = [e(1, 2), e(2, 3)];
        assert!(find_any_cycle(&edges).is_none());
    }

    #[test]
    fn cycle_unrelated_edge_ok() {
        let existing = [e(1, 2), e(2, 3)];
        assert!(!would_introduce_cycle(&existing, 4, 5));
        let all = [e(1, 2), e(2, 3), e(4, 5)];
        assert!(find_any_cycle(&all).is_none());
    }

    #[test]
    fn cycle_three_node() {
        let existing = [e(1, 2), e(2, 3)];
        assert!(would_introduce_cycle(&existing, 3, 1));
    }

    #[test]
    fn cycle_four_node() {
        let existing = [e(1, 2), e(2, 3), e(3, 4)];
        assert!(would_introduce_cycle(&existing, 4, 1));
    }

    #[test]
    fn cycle_long_chain_mid_backedge() {
        let existing = [e(1, 2), e(2, 3), e(3, 4), e(4, 5)];
        assert!(would_introduce_cycle(&existing, 5, 3));
    }

    #[test]
    fn cycle_diamond_no_cycle() {
        let edges = [e(1, 2), e(1, 3), e(2, 4), e(3, 4)];
        assert!(find_any_cycle(&edges).is_none());
    }

    #[test]
    fn cycle_diamond_plus_backedge() {
        let existing = [e(1, 2), e(1, 3), e(2, 4), e(3, 4)];
        assert!(would_introduce_cycle(&existing, 4, 1));
    }

    #[test]
    fn cycle_multi_parent_backedge() {
        let existing = [e(1, 3), e(2, 3)];
        assert!(would_introduce_cycle(&existing, 3, 1));
    }

    #[test]
    fn cycle_disabled_not_in_graph() {
        // Only enabled edges are passed in; B→A disabled means graph is empty of that edge.
        assert!(!would_introduce_cycle(&[], 1, 2));
        let after_enable = [e(1, 2)];
        assert!(would_introduce_cycle(&after_enable, 2, 1));
    }

    #[test]
    fn cycle_replace_drops_old_edge() {
        // After replace A→B with A→C, combined other=[] + proposed=[A→C]
        let combined = [e(1, 3)];
        assert!(find_any_cycle(&combined).is_none());
    }

    #[test]
    fn parse_body_relay_defaults_and_rejects() {
        assert_eq!(parse_body_relay(None).unwrap(), BODY_RELAY_PASSTHROUGH);
        assert_eq!(
            parse_body_relay(Some("progress")).unwrap(),
            BODY_RELAY_PROGRESS
        );
        assert!(parse_body_relay(Some("nope")).is_err());
        assert_eq!(
            body_relay_from_router_json(&json!({})),
            BODY_RELAY_PASSTHROUGH
        );
        assert_eq!(
            body_relay_from_router_json(&json!({"bodyRelay": "progress"})),
            BODY_RELAY_PROGRESS
        );
    }
}
