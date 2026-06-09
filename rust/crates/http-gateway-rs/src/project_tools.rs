//! Gateway tool catalog and per-`proj_id` allowed-tools resolution (DB only). Author: kejiqing

use serde_json::Value;
use tools::mvp_tool_specs;

/// One entry in the gateway-registered tool catalog (`GET /v1/project/tools/catalog`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCatalogEntry {
    pub name: String,
    pub description: String,
    /// `builtin` | `mcp_pattern`
    pub source: String,
}

/// Built-in tools from `mvp_tool_specs` plus the MCP qualified-name pattern.
#[must_use]
pub fn gateway_registered_tool_catalog() -> Vec<ToolCatalogEntry> {
    let mut out: Vec<ToolCatalogEntry> = mvp_tool_specs()
        .into_iter()
        .map(|spec| ToolCatalogEntry {
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            source: "builtin".to_string(),
        })
        .collect();
    out.push(ToolCatalogEntry {
        name: "mcp__*".to_string(),
        description: "Qualified MCP tools discovered at runtime (mcp__<server>__<tool>)"
            .to_string(),
        source: "mcp_pattern".to_string(),
    });
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[must_use]
pub fn normalize_allowed_tool_name(raw: &str) -> String {
    let name = raw.trim();
    match name {
        "read" | "ReadFile" | "ead_file" => "read_file".to_string(),
        "glob" | "GlobSearch" | "glob_searchr" => "glob_search".to_string(),
        "grep" | "GrepSearch" => "grep_search".to_string(),
        "MCPTool" => "MCP".to_string(),
        "ListMcpResourcesToolMCP" => "ListMcpResources".to_string(),
        other => other.to_string(),
    }
}

pub fn parse_allowed_tools_json(value: &Value) -> Result<Vec<String>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let arr = value
        .as_array()
        .ok_or_else(|| "allowedToolsJson must be a JSON array".to_string())?;
    let mut out = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        let name = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("allowedToolsJson[{i}] must be a non-empty string"))?;
        let norm = normalize_allowed_tool_name(name);
        if !out.contains(&norm) {
            out.push(norm);
        }
    }
    Ok(out)
}

fn catalog_allows_tool_name(name: &str, catalog: &[ToolCatalogEntry]) -> bool {
    for entry in catalog {
        if entry.name == name {
            return true;
        }
        if let Some(prefix) = entry.name.strip_suffix('*') {
            if !prefix.is_empty() && name.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

/// Each selected tool must appear in the gateway catalog (`project_config.allowed_tools_json`).
pub fn validate_project_allowed_tools_json(value: &Value) -> Result<Vec<String>, String> {
    let selected = parse_allowed_tools_json(value)?;
    if selected.is_empty() {
        return Ok(selected);
    }
    let catalog = gateway_registered_tool_catalog();
    for name in &selected {
        if !catalog_allows_tool_name(name, &catalog) {
            return Err(format!(
                "allowedToolsJson: tool `{name}` is not in the gateway tool catalog"
            ));
        }
    }
    Ok(selected)
}

#[must_use]
pub fn is_tool_allowed(tool_name: &str, allowed_tools: &[String]) -> bool {
    if allowed_tools.is_empty() {
        return true;
    }
    for pattern in allowed_tools {
        if pattern == tool_name {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if tool_name.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

/// Project baseline from DB: non-empty list restricts solve; empty / missing → no restriction (all tools).
#[must_use]
pub fn project_baseline_allowed_tools(project_selected: Option<&[String]>) -> Vec<String> {
    project_selected
        .filter(|p| !p.is_empty())
        .map(<[String]>::to_vec)
        .unwrap_or_default()
}

/// Resolve tools for solve: project DB subset → optional request override (no env ceiling).
pub fn resolve_effective_allowed_tools_for_ds(
    project_selected: Option<&[String]>,
    requested_allowed_tools: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let baseline = project_baseline_allowed_tools(project_selected);
    let Some(requested) = requested_allowed_tools else {
        return Ok(baseline);
    };

    let mut normalized = Vec::new();
    for raw in requested {
        let name = normalize_allowed_tool_name(raw);
        if name.is_empty() {
            continue;
        }
        if !normalized.contains(&name) {
            normalized.push(name);
        }
    }
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if baseline.is_empty() {
        return Ok(normalized);
    }

    for requested in &normalized {
        let allowed = if requested.ends_with('*') {
            baseline.contains(requested)
        } else {
            is_tool_allowed(requested, &baseline)
        };
        if !allowed {
            return Err(format!(
                "requested tool pattern is not allowed for this project: {requested}"
            ));
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_includes_bash_and_mcp_pattern() {
        let c = gateway_registered_tool_catalog();
        assert!(c.iter().any(|e| e.name == "bash"));
        assert!(c.iter().any(|e| e.name == "mcp__*"));
    }

    #[test]
    fn validate_rejects_unknown_tool() {
        let err = validate_project_allowed_tools_json(&json!(["not_a_real_tool_xyz"])).unwrap_err();
        assert!(err.contains("catalog"));
    }

    #[test]
    fn project_baseline_uses_db_selection_only() {
        let project = vec!["bash".into(), "read_file".into()];
        let base = project_baseline_allowed_tools(Some(&project));
        assert_eq!(base, project);
        assert!(project_baseline_allowed_tools(None).is_empty());
        assert!(project_baseline_allowed_tools(Some(&[])).is_empty());
    }

    #[test]
    fn resolve_request_must_fit_project() {
        let project = vec!["read_file".into()];
        let err = resolve_effective_allowed_tools_for_ds(Some(&project), Some(&["bash".into()]))
            .unwrap_err();
        assert!(err.contains("project"));
    }
}
