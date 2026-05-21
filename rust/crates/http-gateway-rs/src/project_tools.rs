//! Gateway tool catalog and per-`ds_id` allowed-tools resolution. Author: kejiqing

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

/// Each selected tool must appear in the gateway catalog and satisfy global policy (if set).
pub fn validate_project_allowed_tools_json(
    value: &Value,
    global_allowed_tools: &[String],
) -> Result<Vec<String>, String> {
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
        if !global_allowed_tools.is_empty() && !is_tool_allowed(name, global_allowed_tools) {
            return Err(format!(
                "allowedToolsJson: tool `{name}` is not allowed by gateway policy (CLAW_ALLOWED_TOOLS)"
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

/// Project baseline: non-empty `project` restricts to that set (within global ceiling).
/// Empty / missing project selection inherits global ceiling only.
#[must_use]
pub fn project_baseline_allowed_tools(
    global_allowed_tools: &[String],
    project_selected: Option<&[String]>,
) -> Vec<String> {
    let Some(project) = project_selected.filter(|p| !p.is_empty()) else {
        return global_allowed_tools.to_vec();
    };
    if global_allowed_tools.is_empty() {
        return project.to_vec();
    }
    project
        .iter()
        .filter(|t| is_tool_allowed(t, global_allowed_tools))
        .cloned()
        .collect()
}

/// Resolve tools for solve: global ceiling → project subset → optional request override.
pub fn resolve_effective_allowed_tools_for_ds(
    global_allowed_tools: &[String],
    project_selected: Option<&[String]>,
    requested_allowed_tools: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let baseline = project_baseline_allowed_tools(global_allowed_tools, project_selected);
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
        let err =
            validate_project_allowed_tools_json(&json!(["not_a_real_tool_xyz"]), &[]).unwrap_err();
        assert!(err.contains("catalog"));
    }

    #[test]
    fn project_baseline_intersects_global() {
        let global = vec!["bash".into(), "read_file".into()];
        let project = vec!["bash".into(), "write_file".into()];
        let base = project_baseline_allowed_tools(&global, Some(&project));
        assert_eq!(base, vec!["bash"]);
    }

    #[test]
    fn resolve_request_must_fit_project() {
        let project = vec!["read_file".into()];
        let err =
            resolve_effective_allowed_tools_for_ds(&[], Some(&project), Some(&["bash".into()]))
                .unwrap_err();
        assert!(err.contains("project"));
    }
}
