//! L2 per-entity revision history (`project_entity_revision`). Author: kejiqing
//!
//! Append-only history per `(ds_id, domain, entity_key)`. Project-level publish (L1) unchanged.

use axum::http::StatusCode;
use serde::Serialize;
use serde_json::{json, Value};

use crate::project_config_draft::{self, DRAFT_CONTENT_REV};
use crate::session_db::{
    GatewaySessionDb, ProjectConfigRow, ProjectConfigUpsert, ProjectEntityRevisionRow,
};

pub const DOMAIN_RULE: &str = "rule";
pub const DOMAIN_SKILL: &str = "skill";
pub const DOMAIN_MCP: &str = "mcp";
pub const DOMAIN_CLAUDE: &str = "claude";
pub const DOMAIN_TOOLS: &str = "tools";
pub const CLAUDE_ENTITY_KEY: &str = "_";
pub const TOOLS_ENTITY_KEY: &str = "_";

#[derive(Debug)]
pub struct EntityRevisionError {
    pub status: StatusCode,
    pub message: String,
}

impl EntityRevisionError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub fn parse_domain(raw: &str) -> Result<&'static str, EntityRevisionError> {
    match raw.trim().to_lowercase().as_str() {
        "rule" | "rules" => Ok(DOMAIN_RULE),
        "skill" | "skills" => Ok(DOMAIN_SKILL),
        "mcp" => Ok(DOMAIN_MCP),
        "claude" => Ok(DOMAIN_CLAUDE),
        "tool" | "tools" => Ok(DOMAIN_TOOLS),
        other => Err(EntityRevisionError::new(
            StatusCode::BAD_REQUEST,
            format!("unknown entity domain: {other}"),
        )),
    }
}

pub fn normalize_entity_key(domain: &str, raw: &str) -> Result<String, EntityRevisionError> {
    let key = raw.trim().to_string();
    if domain == DOMAIN_CLAUDE {
        return Ok(CLAUDE_ENTITY_KEY.to_string());
    }
    if domain == DOMAIN_TOOLS {
        return Ok(TOOLS_ENTITY_KEY.to_string());
    }
    if key.is_empty() || key.len() > 128 {
        return Err(EntityRevisionError::new(
            StatusCode::BAD_REQUEST,
            "entityKey must be 1..=128 characters",
        ));
    }
    if key.contains('/') || key.contains('\\') {
        return Err(EntityRevisionError::new(
            StatusCode::BAD_REQUEST,
            "entityKey must not contain path separators",
        ));
    }
    Ok(key)
}

async fn allocate_entity_rev(
    db: &GatewaySessionDb,
    ds_id: i64,
    domain: &str,
    entity_key: &str,
    now_ms: i64,
) -> Result<String, sqlx::Error> {
    let base = project_config_draft::format_formal_content_rev_local_ms(now_ms);
    let mut rev = base.clone();
    let mut n = 2u32;
    while db
        .get_project_entity_revision(ds_id, domain, entity_key, &rev)
        .await?
        .is_some()
    {
        rev = format!("{base}-{n}");
        n += 1;
    }
    Ok(rev)
}

/// Append one immutable entity revision. Author: kejiqing
pub async fn append_revision(
    db: &GatewaySessionDb,
    ds_id: i64,
    domain: &str,
    entity_key: &str,
    body: Value,
    note: Option<String>,
    now_ms: i64,
) -> Result<String, EntityRevisionError> {
    parse_domain(domain)?;
    let entity_key = normalize_entity_key(domain, entity_key)?;
    let entity_rev = allocate_entity_rev(db, ds_id, domain, &entity_key, now_ms)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let row = ProjectEntityRevisionRow {
        ds_id,
        domain: domain.to_string(),
        entity_key,
        entity_rev: entity_rev.clone(),
        created_at_ms: now_ms,
        note: project_config_draft::normalize_revision_note(note),
        body,
    };
    db.insert_project_entity_revision_immutable(&row)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(entity_rev)
}

pub async fn append_rule(db: &GatewaySessionDb, ds_id: i64, rule: &Value, now_ms: i64) -> Result<(), EntityRevisionError> {
    let key = rule_entity_key(rule)?;
    let _ = append_revision(db, ds_id, DOMAIN_RULE, &key, rule.clone(), None, now_ms).await?;
    Ok(())
}

pub async fn append_skill(
    db: &GatewaySessionDb,
    ds_id: i64,
    skill_name: &str,
    skill_body: Value,
    now_ms: i64,
) -> Result<(), EntityRevisionError> {
    let _ = append_revision(
        db,
        ds_id,
        DOMAIN_SKILL,
        skill_name,
        skill_body,
        None,
        now_ms,
    )
    .await?;
    Ok(())
}

pub async fn append_mcp_server(
    db: &GatewaySessionDb,
    ds_id: i64,
    server_name: &str,
    config: Value,
    now_ms: i64,
) -> Result<(), EntityRevisionError> {
    let _ = append_revision(
        db,
        ds_id,
        DOMAIN_MCP,
        server_name,
        config,
        None,
        now_ms,
    )
    .await?;
    Ok(())
}

pub async fn append_claude(
    db: &GatewaySessionDb,
    ds_id: i64,
    content: &str,
    now_ms: i64,
) -> Result<(), EntityRevisionError> {
    let body = json!({ "content": content });
    let _ = append_revision(
        db,
        ds_id,
        DOMAIN_CLAUDE,
        CLAUDE_ENTITY_KEY,
        body,
        None,
        now_ms,
    )
    .await?;
    Ok(())
}

pub async fn append_tools(
    db: &GatewaySessionDb,
    ds_id: i64,
    tools: &Value,
    now_ms: i64,
) -> Result<(), EntityRevisionError> {
    let _ = append_revision(
        db,
        ds_id,
        DOMAIN_TOOLS,
        TOOLS_ENTITY_KEY,
        json!({ "allowedTools": tools }),
        None,
        now_ms,
    )
    .await?;
    Ok(())
}

fn rule_entity_key(rule: &Value) -> Result<String, EntityRevisionError> {
    let id = rule
        .get("ruleId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let title = rule
        .get("ruleTitle")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let path = rule
        .get("relativePath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let key = id
        .map(str::to_string)
        .or_else(|| title.map(str::to_string))
        .or_else(|| {
            path.map(|p| {
                p.replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or(p)
                    .trim_end_matches(".mdc")
                    .to_string()
            })
        });
    let Some(key) = key.filter(|s| !s.is_empty()) else {
        return Err(EntityRevisionError::new(
            StatusCode::BAD_REQUEST,
            "rule entry needs ruleId or ruleTitle",
        ));
    };
    normalize_entity_key(DOMAIN_RULE, &key)
}

/// Record L2 revisions after draft PUT when slices changed. Author: kejiqing
pub async fn record_draft_put_sidecars(
    db: &GatewaySessionDb,
    ds_id: i64,
    prev: &ProjectConfigRow,
    rules_json: &Value,
    skills_json: &Value,
    mcp_servers_json: &Value,
    claude_md: Option<&str>,
    allowed_tools_json: &Value,
    now_ms: i64,
) -> Result<(), EntityRevisionError> {
    if rules_json != &prev.rules_json {
        if let Some(arr) = rules_json.as_array() {
            for rule in arr {
                append_rule(db, ds_id, rule, now_ms).await?;
            }
        }
    }
    if skills_json != &prev.skills_json {
        if let Some(arr) = skills_json.as_array() {
            for item in arr {
                let name = item
                    .get("skillName")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let Some(name) = name else {
                    continue;
                };
                append_skill(db, ds_id, name, item.clone(), now_ms).await?;
            }
        }
    }
    if mcp_servers_json != &prev.mcp_servers_json {
        if let Some(obj) = mcp_servers_json.as_object() {
            for (name, cfg) in obj {
                append_mcp_server(db, ds_id, name, cfg.clone(), now_ms).await?;
            }
        }
    }
    let prev_claude = prev.claude_md.as_deref().unwrap_or("");
    let new_claude = claude_md.unwrap_or("");
    if new_claude != prev_claude {
        append_claude(db, ds_id, new_claude, now_ms).await?;
    }
    if allowed_tools_json != &prev.allowed_tools_json {
        append_tools(db, ds_id, allowed_tools_json, now_ms).await?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct EntityVersionsListResponse {
    #[serde(rename = "dsId")]
    pub ds_id: i64,
    pub domain: String,
    #[serde(rename = "entityKey")]
    pub entity_key: String,
    pub versions: Vec<EntityVersionEntry>,
}

#[derive(Debug, Serialize)]
pub struct EntityVersionEntry {
    #[serde(rename = "entityRev")]
    pub entity_rev: String,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EntityCompareResponse {
    #[serde(rename = "dsId")]
    pub ds_id: i64,
    pub domain: String,
    #[serde(rename = "entityKey")]
    pub entity_key: String,
    pub from: String,
    pub to: String,
    pub same: bool,
    #[serde(rename = "fromBody")]
    pub from_body: Value,
    #[serde(rename = "toBody")]
    pub to_body: Value,
}

pub async fn list_entity_versions(
    db: &GatewaySessionDb,
    ds_id: i64,
    domain_raw: &str,
    entity_key_raw: &str,
) -> Result<EntityVersionsListResponse, EntityRevisionError> {
    let domain = parse_domain(domain_raw)?;
    let entity_key = normalize_entity_key(domain, entity_key_raw)?;
    let rows = db
        .list_project_entity_revisions(ds_id, domain, &entity_key)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(EntityVersionsListResponse {
        ds_id,
        domain: domain.to_string(),
        entity_key,
        versions: rows
            .into_iter()
            .map(|r| EntityVersionEntry {
                entity_rev: r.entity_rev,
                created_at_ms: r.created_at_ms,
                note: r.note,
            })
            .collect(),
    })
}

pub async fn compare_entity_versions(
    db: &GatewaySessionDb,
    ds_id: i64,
    domain_raw: &str,
    entity_key_raw: &str,
    from_rev: &str,
    to_rev: &str,
) -> Result<EntityCompareResponse, EntityRevisionError> {
    let domain = parse_domain(domain_raw)?;
    let entity_key = normalize_entity_key(domain, entity_key_raw)?;
    let from_row = db
        .get_project_entity_revision(ds_id, domain, &entity_key, from_rev)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            EntityRevisionError::new(
                StatusCode::NOT_FOUND,
                format!("entity revision not found: {from_rev}"),
            )
        })?;
    let to_row = db
        .get_project_entity_revision(ds_id, domain, &entity_key, to_rev)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            EntityRevisionError::new(
                StatusCode::NOT_FOUND,
                format!("entity revision not found: {to_rev}"),
            )
        })?;
    let same = from_row.body == to_row.body;
    Ok(EntityCompareResponse {
        ds_id,
        domain: domain.to_string(),
        entity_key,
        from: from_rev.to_string(),
        to: to_rev.to_string(),
        same,
        from_body: from_row.body,
        to_body: to_row.body,
    })
}

#[derive(Debug, serde::Deserialize)]
pub struct RestoreEntityRevisionRequest {
    #[serde(rename = "entityRev")]
    pub entity_rev: String,
}

#[derive(Debug, Serialize)]
pub struct RestoreEntityRevisionResponse {
    #[serde(rename = "dsId")]
    pub ds_id: i64,
    pub domain: String,
    #[serde(rename = "entityKey")]
    pub entity_key: String,
    #[serde(rename = "entityRev")]
    pub entity_rev: String,
    #[serde(rename = "draftOpen")]
    pub draft_open: bool,
}

pub async fn restore_entity_revision_to_draft(
    db: &GatewaySessionDb,
    ds_id: i64,
    domain_raw: &str,
    entity_key_raw: &str,
    entity_rev: &str,
) -> Result<RestoreEntityRevisionResponse, EntityRevisionError> {
    let domain = parse_domain(domain_raw)?;
    let entity_key = normalize_entity_key(domain, entity_key_raw)?;
    let rev_row = db
        .get_project_entity_revision(ds_id, domain, &entity_key, entity_rev)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            EntityRevisionError::new(
                StatusCode::NOT_FOUND,
                format!("entity revision not found: {entity_rev}"),
            )
        })?;
    project_config_draft::ensure_draft(db, ds_id)
        .await
        .map_err(draft_err)?;
    let mut row = db
        .get_project_config(ds_id)
        .await
        .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            EntityRevisionError::new(
                StatusCode::NOT_FOUND,
                format!("no project_config for ds {ds_id}"),
            )
        })?;
    apply_entity_body_to_draft_row(&mut row, domain, &entity_key, &rev_row.body)?;
    let now = now_ms();
    row.draft_open = true;
    row.content_rev = DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now;
    let stable = project_config_draft::effective_formal_rev(&row).map_err(draft_err)?;
    db.upsert_project_config(ProjectConfigUpsert {
        ds_id,
        content_rev: DRAFT_CONTENT_REV,
        stable_content_rev: Some(stable),
        draft_open: true,
        updated_at_ms: now,
        rules_json: &row.rules_json,
        mcp_servers_json: &row.mcp_servers_json,
        skills_sources_json: &row.skills_sources_json,
        skills_json: &row.skills_json,
        allowed_tools_json: &row.allowed_tools_json,
        claude_md: row.claude_md.as_deref(),
        git_sync_json: &row.git_sync_json,
    })
    .await
    .map_err(|e| EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(RestoreEntityRevisionResponse {
        ds_id,
        domain: domain.to_string(),
        entity_key,
        entity_rev: entity_rev.to_string(),
        draft_open: true,
    })
}

fn apply_entity_body_to_draft_row(
    row: &mut ProjectConfigRow,
    domain: &str,
    entity_key: &str,
    body: &Value,
) -> Result<(), EntityRevisionError> {
    match domain {
        DOMAIN_RULE => {
            let arr = row.rules_json.as_array_mut().ok_or_else(|| {
                EntityRevisionError::new(StatusCode::INTERNAL_SERVER_ERROR, "rulesJson not array")
            })?;
            let mut found = false;
            for item in arr.iter_mut() {
                if rule_entity_key(item).ok().as_deref() == Some(entity_key) {
                    *item = body.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                arr.push(body.clone());
            }
        }
        DOMAIN_SKILL => {
            let arr = row.skills_json.as_array_mut().ok_or_else(|| {
                EntityRevisionError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "skillsJson not array",
                )
            })?;
            let mut found = false;
            for item in arr.iter_mut() {
                if item.get("skillName").and_then(Value::as_str) == Some(entity_key) {
                    *item = body.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                arr.push(body.clone());
            }
        }
        DOMAIN_MCP => {
            let obj = row
                .mcp_servers_json
                .as_object_mut()
                .ok_or_else(|| {
                    EntityRevisionError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "mcpServersJson not object",
                    )
                })?;
            obj.insert(entity_key.to_string(), body.clone());
        }
        DOMAIN_CLAUDE => {
            let content = body
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("");
            row.claude_md = if content.trim().is_empty() {
                None
            } else {
                Some(content.to_string())
            };
        }
        DOMAIN_TOOLS => {
            row.allowed_tools_json = body
                .get("allowedTools")
                .cloned()
                .unwrap_or_else(|| body.clone());
        }
        _ => {
            return Err(EntityRevisionError::new(
                StatusCode::BAD_REQUEST,
                "unsupported domain",
            ));
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn draft_err(e: project_config_draft::DraftError) -> EntityRevisionError {
    EntityRevisionError::new(e.status, e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_domain_accepts_aliases() {
        assert_eq!(parse_domain("rules").unwrap(), DOMAIN_RULE);
        assert_eq!(parse_domain("skill").unwrap(), DOMAIN_SKILL);
    }

    #[test]
    fn normalize_claude_key_is_fixed() {
        assert_eq!(
            normalize_entity_key(DOMAIN_CLAUDE, "anything").unwrap(),
            CLAUDE_ENTITY_KEY
        );
    }
}
