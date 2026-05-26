//! Per-`ds_id` configuration revision history and diff (active rev may lag newest). Author: kejiqing

use serde::Serialize;
use serde_json::Value;

use crate::session_db::ProjectConfigRevisionRow;

#[derive(Debug, Clone, Serialize)]
pub struct ConfigFieldChange {
    pub field: String,
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectConfigCompareResponse {
    #[serde(rename = "dsId")]
    pub ds_id: i64,
    pub from: String,
    pub to: String,
    #[serde(rename = "activeContentRev")]
    pub active_content_rev: String,
    pub same: bool,
    /// Top-level field summary (quick scan).
    pub changes: Vec<ConfigFieldChange>,
    /// Expanded project config JSON for `from` (materialization fields only).
    #[serde(rename = "fromDocument")]
    pub from_document: Value,
    #[serde(rename = "toDocument")]
    pub to_document: Value,
}

/// Canonical compare/merge document (one project revision as flat JSON). Author: kejiqing
#[must_use]
pub fn revision_row_to_document(row: &ProjectConfigRevisionRow) -> Value {
    serde_json::json!({
        "contentRev": row.content_rev,
        "note": row.note,
        "claudeMd": row.claude_md,
        "rulesJson": row.rules_json,
        "skillsJson": row.skills_json,
        "mcpServersJson": row.mcp_servers_json,
        "allowedToolsJson": row.allowed_tools_json,
    })
}

fn claude_summary(md: Option<&str>) -> String {
    match md.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => format!("{} chars", s.len()),
        None => "empty".into(),
    }
}

fn skill_names(skills: &Value) -> Vec<String> {
    skills
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("skillName")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn rule_ids(rules: &Value) -> Vec<String> {
    rules
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("relativePath")
                        .or_else(|| item.get("ruleId"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn mcp_names(mcp: &Value) -> Vec<String> {
    mcp.as_object()
        .map(|o| {
            let mut keys: Vec<_> = o.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default()
}

fn tool_names(tools: &Value) -> Vec<String> {
    tools
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn set_diff(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let old_set: std::collections::BTreeSet<_> = old.iter().collect();
    let new_set: std::collections::BTreeSet<_> = new.iter().collect();
    let added: Vec<String> = new
        .iter()
        .filter(|s| !old_set.contains(s))
        .cloned()
        .collect();
    let removed: Vec<String> = old
        .iter()
        .filter(|s| !new_set.contains(s))
        .cloned()
        .collect();
    (added, removed)
}

fn push_set_change(
    changes: &mut Vec<ConfigFieldChange>,
    field: &str,
    old: &[String],
    new: &[String],
) {
    if old == new {
        return;
    }
    let (added, removed) = set_diff(old, new);
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("+{}", added.join(", ")));
    }
    if !removed.is_empty() {
        parts.push(format!("-{}", removed.join(", ")));
    }
    changes.push(ConfigFieldChange {
        field: field.to_string(),
        kind: "modified".into(),
        detail: parts.join("; "),
    });
}

/// Compare two stored revisions (`from` → `to`). Author: kejiqing
#[must_use]
pub fn compare_revision_rows(
    ds_id: i64,
    active_content_rev: &str,
    from: &ProjectConfigRevisionRow,
    to: &ProjectConfigRevisionRow,
) -> ProjectConfigCompareResponse {
    let mut changes = Vec::new();

    if from.claude_md != to.claude_md {
        changes.push(ConfigFieldChange {
            field: "claudeMd".into(),
            kind: "modified".into(),
            detail: format!(
                "{} → {}",
                claude_summary(from.claude_md.as_deref()),
                claude_summary(to.claude_md.as_deref())
            ),
        });
    }

    push_set_change(
        &mut changes,
        "skillsJson",
        &skill_names(&from.skills_json),
        &skill_names(&to.skills_json),
    );
    push_set_change(
        &mut changes,
        "rulesJson",
        &rule_ids(&from.rules_json),
        &rule_ids(&to.rules_json),
    );
    push_set_change(
        &mut changes,
        "mcpServersJson",
        &mcp_names(&from.mcp_servers_json),
        &mcp_names(&to.mcp_servers_json),
    );
    push_set_change(
        &mut changes,
        "allowedToolsJson",
        &tool_names(&from.allowed_tools_json),
        &tool_names(&to.allowed_tools_json),
    );

    let from_document = revision_row_to_document(from);
    let to_document = revision_row_to_document(to);
    let same = from_document == to_document;
    ProjectConfigCompareResponse {
        ds_id,
        from: from.content_rev.clone(),
        to: to.content_rev.clone(),
        active_content_rev: active_content_rev.to_string(),
        same,
        changes,
        from_document,
        to_document,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(rev: &str, claude: Option<&str>, skills: Value) -> ProjectConfigRevisionRow {
        ProjectConfigRevisionRow {
            ds_id: 1,
            content_rev: rev.into(),
            created_at_ms: 0,
            note: None,
            rules_json: json!([]),
            mcp_servers_json: json!({}),
            skills_sources_json: json!([]),
            skills_json: skills,
            allowed_tools_json: json!([]),
            claude_md: claude.map(str::to_string),
        }
    }

    #[test]
    fn compare_detects_claude_and_skill_changes() {
        let a = row("a", Some("# A"), json!([]));
        let b = row(
            "b",
            Some("# B longer"),
            json!([{"skillName": "x", "skillContent": "c"}]),
        );
        let r = compare_revision_rows(1, "a", &a, &b);
        assert!(!r.same);
        assert!(r.changes.iter().any(|c| c.field == "claudeMd"));
        assert!(r.changes.iter().any(|c| c.field == "skillsJson"));
        assert_eq!(
            r.from_document.get("contentRev").and_then(Value::as_str),
            Some("a")
        );
        assert_ne!(r.from_document, r.to_document);
    }

    #[test]
    fn revision_row_to_document_includes_materialization_fields() {
        let row = row("v1", Some("# x"), json!([]));
        let doc = revision_row_to_document(&row);
        assert!(doc.get("rulesJson").is_some());
        assert!(doc.get("mcpServersJson").is_some());
        assert!(doc.get("allowedToolsJson").is_some());
    }
}
