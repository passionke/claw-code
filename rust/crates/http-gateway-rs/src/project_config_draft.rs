//! Draft vs formal `project_config` versioning. Author: kejiqing
//!
//! State machine (per `ds_id`):
//!
//! ```text
//! STEADY:  draft_open=false, effective E ∈ formal revisions (project_config_revision)
//! EDITING: draft_open=true,  exactly one temp row content_rev=__draft__, effective E unchanged
//! ```
//!
//! - Solve/materialize uses **effective formal** `stable_content_rev` only (never `__draft__`).
//! - Tab saves → EDITING (open or update the single temp draft).
//! - `POST …/versions/commit` → auto id + optional note → formal F, back to STEADY (effective still E).
//! - `POST …/versions/{rev}/activate` → effective E := rev (must be formal), discard open draft.
//! - `DELETE …/versions/{rev}` → drop formal rev (not current effective).

use axum::http::StatusCode;
use chrono::{Local, TimeZone};
use serde_json::Value;

use crate::session_db::{GatewaySessionDb, ProjectConfigRevisionRow, ProjectConfigRow, ProjectConfigUpsert};

/// The single in-progress temp revision id (never effective, never in formal list).
pub const DRAFT_CONTENT_REV: &str = "__draft__";

pub fn is_draft_content_rev(rev: &str) -> bool {
    rev.trim() == DRAFT_CONTENT_REV
}

/// Formal version id: local `YYYYMMDDHHmmss` (second resolution). Author: kejiqing
pub fn format_formal_content_rev_local_ms(ms: i64) -> String {
    let Some(dt) = Local.timestamp_millis_opt(ms).single() else {
        return ms.to_string();
    };
    dt.format("%Y%m%d%H%M%S").to_string()
}

/// Pick unused formal `content_rev` for `ds_id` (suffix `-2` on collision). Author: kejiqing
pub async fn allocate_formal_content_rev(
    db: &GatewaySessionDb,
    ds_id: i64,
    now_ms: i64,
) -> Result<String, DraftError> {
    let base = format_formal_content_rev_local_ms(now_ms);
    let mut rev = base.clone();
    let mut n = 2u32;
    while db.get_project_config_revision(ds_id, &rev).await?.is_some() {
        rev = format!("{base}-{n}");
        n += 1;
    }
    Ok(rev)
}

const MAX_REVISION_NOTE_LEN: usize = 500;

pub fn normalize_revision_note(note: Option<String>) -> Option<String> {
    note.map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.len() > MAX_REVISION_NOTE_LEN {
                s[..MAX_REVISION_NOTE_LEN].to_string()
            } else {
                s
            }
        })
}

/// Effective revision id: must not be the temp draft marker.
pub fn effective_formal_rev(row: &ProjectConfigRow) -> Result<&str, DraftError> {
    let stable = row
        .stable_content_rev
        .as_deref()
        .filter(|s| !s.is_empty() && !is_draft_content_rev(s))
        .or_else(|| {
            if !row.draft_open && !is_draft_content_rev(&row.content_rev) {
                Some(row.content_rev.as_str())
            } else {
                None
            }
        });
    stable.ok_or_else(|| {
        DraftError::new(
            StatusCode::CONFLICT,
            "no effective formal contentRev (create project or activate a formal version)",
        )
    })
}

pub async fn require_formal_revision(
    db: &GatewaySessionDb,
    ds_id: i64,
    content_rev: &str,
) -> Result<ProjectConfigRevisionRow, DraftError> {
    if is_draft_content_rev(content_rev) {
        return Err(DraftError::new(
            StatusCode::BAD_REQUEST,
            "temporary draft is not a formal version",
        ));
    }
    db.get_project_config_revision(ds_id, content_rev)
        .await?
        .ok_or_else(|| {
            DraftError::new(
                StatusCode::NOT_FOUND,
                format!("no formal revision {content_rev} for ds {ds_id}"),
            )
        })
}

/// Ensure effective formal revision exists in `project_config_revision` (repair legacy rows).
pub async fn ensure_formal_revision_recorded(
    db: &GatewaySessionDb,
    ds_id: i64,
    formal_rev: &str,
    snapshot: &ProjectConfigRow,
) -> Result<(), DraftError> {
    if is_draft_content_rev(formal_rev) {
        return Err(DraftError::new(
            StatusCode::BAD_REQUEST,
            "cannot archive temp draft as formal version",
        ));
    }
    if db
        .get_project_config_revision(ds_id, formal_rev)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let row = ProjectConfigRevisionRow {
        ds_id,
        content_rev: formal_rev.to_string(),
        created_at_ms: snapshot.updated_at_ms,
        note: None,
        rules_json: snapshot.rules_json.clone(),
        mcp_servers_json: snapshot.mcp_servers_json.clone(),
        skills_sources_json: snapshot.skills_sources_json.clone(),
        skills_json: snapshot.skills_json.clone(),
        allowed_tools_json: snapshot.allowed_tools_json.clone(),
        claude_md: snapshot.claude_md.clone(),
    };
    let _ = db.insert_project_config_revision_immutable(&row).await?;
    Ok(())
}

pub fn revision_row_from_config_row(
    row: &ProjectConfigRow,
    content_rev: &str,
    note: Option<String>,
) -> ProjectConfigRevisionRow {
    ProjectConfigRevisionRow {
        ds_id: row.ds_id,
        content_rev: content_rev.to_string(),
        created_at_ms: row.updated_at_ms,
        note,
        rules_json: row.rules_json.clone(),
        mcp_servers_json: row.mcp_servers_json.clone(),
        skills_sources_json: row.skills_sources_json.clone(),
        skills_json: row.skills_json.clone(),
        allowed_tools_json: row.allowed_tools_json.clone(),
        claude_md: row.claude_md.clone(),
    }
}

pub fn config_row_from_revision(
    ds_id: i64,
    rev: &ProjectConfigRevisionRow,
    git_sync_json: Value,
    stable_content_rev: &str,
) -> ProjectConfigRow {
    ProjectConfigRow {
        ds_id,
        content_rev: stable_content_rev.to_string(),
        stable_content_rev: Some(stable_content_rev.to_string()),
        draft_open: false,
        updated_at_ms: rev.created_at_ms,
        rules_json: rev.rules_json.clone(),
        mcp_servers_json: rev.mcp_servers_json.clone(),
        skills_sources_json: rev.skills_sources_json.clone(),
        skills_json: rev.skills_json.clone(),
        allowed_tools_json: rev.allowed_tools_json.clone(),
        claude_md: rev.claude_md.clone(),
        git_sync_json,
    }
}

pub fn upsert_from_row<'a>(
    row: &'a ProjectConfigRow,
    content_rev: &'a str,
    updated_at_ms: i64,
    claude_md: Option<&'a str>,
    stable_content_rev: Option<&'a str>,
) -> ProjectConfigUpsert<'a> {
    ProjectConfigUpsert {
        ds_id: row.ds_id,
        content_rev,
        stable_content_rev,
        draft_open: row.draft_open,
        updated_at_ms,
        rules_json: &row.rules_json,
        mcp_servers_json: &row.mcp_servers_json,
        skills_sources_json: &row.skills_sources_json,
        skills_json: &row.skills_json,
        allowed_tools_json: &row.allowed_tools_json,
        claude_md,
        git_sync_json: &row.git_sync_json,
    }
}

/// Row used for `apply_if_needed` — always effective **formal** snapshot, never temp draft.
pub async fn row_for_materialize(
    db: &GatewaySessionDb,
    ds_id: i64,
) -> Result<Option<ProjectConfigRow>, sqlx::Error> {
    let Some(row) = db.get_project_config(ds_id).await? else {
        return Ok(None);
    };
    let effective = match effective_formal_rev(&row) {
        Ok(s) => s.to_string(),
        Err(_) => return Ok(None),
    };
    if let Some(rev) = db.get_project_config_revision(ds_id, &effective).await? {
        return Ok(Some(config_row_from_revision(
            ds_id,
            &rev,
            row.git_sync_json.clone(),
            &effective,
        )));
    }
    if !row.draft_open && row.content_rev == effective {
        return Ok(Some(row));
    }
    Ok(None)
}

/// Open or refresh the **only** temp draft from current effective formal revision.
pub async fn ensure_draft(
    db: &GatewaySessionDb,
    ds_id: i64,
) -> Result<ProjectConfigRow, DraftError> {
    let Some(row) = db.get_project_config(ds_id).await? else {
        return Err(DraftError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}"),
        ));
    };
    let effective = effective_formal_rev(&row)?.to_string();
    ensure_formal_revision_recorded(db, ds_id, &effective, &row).await?;
    let formal = require_formal_revision(db, ds_id, &effective).await?;

    if row.draft_open && is_draft_content_rev(&row.content_rev) {
        return Ok(row);
    }

    let now = now_ms();
    let upsert = ProjectConfigUpsert {
        ds_id,
        content_rev: DRAFT_CONTENT_REV,
        stable_content_rev: Some(effective.as_str()),
        draft_open: true,
        updated_at_ms: now,
        rules_json: &formal.rules_json,
        mcp_servers_json: &formal.mcp_servers_json,
        skills_sources_json: &formal.skills_sources_json,
        skills_json: &formal.skills_json,
        allowed_tools_json: &formal.allowed_tools_json,
        claude_md: formal.claude_md.as_deref(),
        git_sync_json: &row.git_sync_json,
    };
    db.upsert_project_config(upsert).await?;
    db.get_project_config(ds_id)
        .await?
        .ok_or_else(|| DraftError::new(StatusCode::INTERNAL_SERVER_ERROR, "draft row missing after upsert"))
}

/// Close temp draft; `project_config` row becomes effective formal content (STEADY).
pub async fn close_draft_to_stable(
    db: &GatewaySessionDb,
    ds_id: i64,
    stable_content_rev: &str,
    git_sync_json: &Value,
) -> Result<ProjectConfigRow, DraftError> {
    if is_draft_content_rev(stable_content_rev) {
        return Err(DraftError::new(
            StatusCode::BAD_REQUEST,
            "effective version cannot be the temp draft id",
        ));
    }
    let formal = require_formal_revision(db, ds_id, stable_content_rev).await?;
    let row = config_row_from_revision(ds_id, &formal, git_sync_json.clone(), stable_content_rev);
    db.upsert_project_config(upsert_from_row(
        &row,
        stable_content_rev,
        now_ms(),
        row.claude_md.as_deref(),
        Some(stable_content_rev),
    ))
    .await?;
    db.get_project_config(ds_id)
        .await?
        .ok_or_else(|| DraftError::new(StatusCode::INTERNAL_SERVER_ERROR, "row missing after close draft"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug)]
pub struct DraftError {
    pub status: StatusCode,
    pub message: String,
}

impl DraftError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<sqlx::Error> for DraftError {
    fn from(e: sqlx::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
    }
}
