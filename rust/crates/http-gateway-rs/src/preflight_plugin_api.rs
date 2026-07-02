//! Admin API for global preflight plugin registry. Author: kejiqing

use axum::http::StatusCode;
use preflight_spi::{
    normalize_pipeline_steps, parse_pipeline_value, PreflightImpl, PreflightPluginRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertPreflightPluginRequest {
    pub display_name: String,
    #[serde(default = "default_spi_version")]
    pub spi_version: String,
    #[serde(default)]
    pub default_impl: Option<PreflightImpl>,
    #[serde(default)]
    pub config_schema: Value,
}

fn default_spi_version() -> String {
    "1".to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightPluginListResponse {
    pub plugins: Vec<PreflightPluginRecord>,
}

pub type PreflightApiError = (StatusCode, String);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub async fn list_preflight_plugins(
    db: &GatewaySessionDb,
) -> Result<PreflightPluginListResponse, PreflightApiError> {
    let plugins = db
        .list_preflight_plugins()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(PreflightPluginListResponse { plugins })
}

pub async fn upsert_preflight_plugin(
    db: &GatewaySessionDb,
    plugin_id: &str,
    req: UpsertPreflightPluginRequest,
) -> Result<PreflightPluginRecord, PreflightApiError> {
    let plugin_id = plugin_id.trim();
    if plugin_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            String::from("pluginId must be non-empty"),
        ));
    }
    if req.display_name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            String::from("displayName must be non-empty"),
        ));
    }
    if let Some(PreflightImpl::Subprocess { command }) = &req.default_impl {
        if command.is_empty() || command.iter().all(|c| c.trim().is_empty()) {
            return Err((
                StatusCode::BAD_REQUEST,
                String::from("subprocess defaultImpl requires non-empty command"),
            ));
        }
    }
    let record = PreflightPluginRecord {
        plugin_id: plugin_id.to_string(),
        display_name: req.display_name.trim().to_string(),
        spi_version: req.spi_version.trim().to_string(),
        default_impl: req.default_impl,
        config_schema: if req.config_schema.is_null() {
            Value::Object(Map::default())
        } else {
            req.config_schema
        },
    };
    db.upsert_preflight_plugin(&record, now_ms())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(record)
}

/// Ensure each pipeline step references a registered plugin id.
pub async fn validate_solve_preflight_plugin_refs(
    db: &GatewaySessionDb,
    value: &Value,
) -> Result<(), String> {
    gateway_solve_turn::project_preflight::validate_solve_preflight_json(value)?;
    let cfg = parse_pipeline_value(value)?;
    let steps = normalize_pipeline_steps(&cfg);
    if steps.is_empty() {
        return Ok(());
    }
    let registered = db
        .list_preflight_plugin_ids()
        .await
        .map_err(|e| format!("preflight plugin registry: {e}"))?;
    let registered: std::collections::HashSet<String> = registered.into_iter().collect();
    for step in &steps {
        if !registered.contains(&step.plugin_id) {
            return Err(format!(
                "solvePreflightJson references unknown pluginId {:?} (register in preflight plugin library first)",
                step.plugin_id
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upsert_request_deserializes() {
        let req: UpsertPreflightPluginRequest = serde_json::from_value(json!({
            "displayName": "Custom",
            "defaultImpl": {"type": "subprocess", "command": ["python3", "/opt/x.py"]}
        }))
        .expect("json");
        assert_eq!(req.display_name, "Custom");
    }
}
