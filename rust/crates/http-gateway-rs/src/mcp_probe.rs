//! MCP framework connectivity probe for admin (`POST /v1/mcp/test`). Author: kejiqing
//!
//! Scope: transport reachability + `initialize` + `tools/list` only.
//! Business auth (e.g. SQLBot `mcp_start`, other servers' `init`) belongs to solve/preflight, not here.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use runtime::{ConfigLoader, McpServerManager, RuntimeConfig};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;

const PROBE_HINT_WORKER: &str = "探测在 Gateway 进程环境执行。若 solve 使用 pool worker 容器，URL 须用 worker 能访问宿主机 MCP 的地址（如 http://host.docker.internal:8001/mcp-streamable），勿写 127.0.0.1。";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestResponse {
    pub ok: bool,
    pub status: String,
    pub server_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    pub discover_ok: bool,
    pub tool_count: usize,
    #[serde(default)]
    pub tools_sample: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    pub duration_ms: u64,
    pub hint: &'static str,
}

fn probe_temp_dir(server_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("claw-mcp-probe-{server_name}-{nanos}"))
}

fn url_from_config(config: &Value) -> Option<String> {
    config
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn transport_from_config(config: &Value) -> Option<String> {
    config
        .get("type")
        .or_else(|| config.get("transport"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn collect_warnings(server_name: &str, config: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(url) = url_from_config(config) {
        if url.contains("127.0.0.1") || url.contains("localhost") {
            out.push(format!(
                "URL `{url}` 在 pool worker 容器内通常指向容器自身而非宿主机；solve 请改用 host.docker.internal（或宿主机 LAN IP）。"
            ));
        }
    }
    if config
        .get("headers")
        .and_then(Value::as_object)
        .is_none_or(serde_json::Map::is_empty)
    {
        out.push("未配置 headers：HTTP MCP 通常需要 Authorization 或 x-ak/x-sk。".to_string());
    }
    let _ = server_name;
    out
}

fn discovery_errors(server_name: &str, report: &runtime::McpToolDiscoveryReport) -> Vec<String> {
    let mut out = Vec::new();
    for failure in &report.failed_servers {
        if failure.server_name == server_name {
            out.push(format!(
                "{}: {} ({})",
                failure.phase, failure.error, failure.server_name
            ));
        }
    }
    for unsupported in &report.unsupported_servers {
        if unsupported.server_name == server_name {
            out.push(format!(
                "unsupported transport {:?}: {}",
                unsupported.transport, unsupported.reason
            ));
        }
    }
    out
}

async fn write_probe_settings(
    work_dir: &Path,
    server_name: &str,
    config: &Value,
) -> Result<(), String> {
    let claw = work_dir.join(".claw");
    fs::create_dir_all(&claw)
        .await
        .map_err(|e| format!("mkdir {}: {e}", claw.display()))?;
    let settings = json!({
        "mcpServers": {
            server_name: config
        },
        "auto_hidden_system_prompt": 1
    });
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(claw.join("settings.json"), bytes)
        .await
        .map_err(|e| format!("write settings.json: {e}"))
}

/// Run MCP discover (`initialize` + `tools/list`) for one server config.
pub async fn probe_mcp_server(server_name: &str, config: &Value) -> McpTestResponse {
    let started = Instant::now();
    let warnings = collect_warnings(server_name, config);
    let url = url_from_config(config);
    let transport = transport_from_config(config);

    let work_dir = probe_temp_dir(server_name);
    let mut errors = Vec::new();
    let mut discover_ok = false;
    let mut tool_count = 0usize;
    let mut tools_sample = Vec::new();

    let cleanup = || {
        let _ = std::fs::remove_dir_all(&work_dir);
    };

    if !config.is_object() {
        cleanup();
        return McpTestResponse {
            ok: false,
            status: "error".to_string(),
            server_name: server_name.to_string(),
            url,
            transport,
            discover_ok: false,
            tool_count: 0,
            tools_sample,
            warnings,
            errors: vec!["config must be a JSON object".to_string()],
            duration_ms: started.elapsed().as_millis() as u64,
            hint: PROBE_HINT_WORKER,
        };
    }

    if let Err(e) = write_probe_settings(&work_dir, server_name, config).await {
        errors.push(e);
        cleanup();
        return fail_response(
            server_name,
            url,
            transport,
            warnings,
            errors,
            started,
            discover_ok,
            tool_count,
            tools_sample,
        );
    }

    let runtime_cfg: RuntimeConfig = match ConfigLoader::default_for(&work_dir).load() {
        Ok(cfg) => cfg,
        Err(e) => {
            errors.push(format!("load config: {e}"));
            cleanup();
            return fail_response(
                server_name,
                url,
                transport,
                warnings,
                errors,
                started,
                discover_ok,
                tool_count,
                tools_sample,
            );
        }
    };

    let mut manager = McpServerManager::from_runtime_config(&runtime_cfg);
    if !manager.server_names().contains(&server_name.to_string()) {
        errors.extend(
            manager
                .unsupported_servers()
                .iter()
                .filter(|u| u.server_name == server_name)
                .map(|u| format!("unsupported: {}", u.reason)),
        );
        if errors.is_empty() {
            errors.push(format!(
                "server {server_name:?} not registered (check type/url)"
            ));
        }
        cleanup();
        return fail_response(
            server_name,
            url,
            transport,
            warnings,
            errors,
            started,
            discover_ok,
            tool_count,
            tools_sample,
        );
    }

    let report = manager.discover_tools_best_effort().await;
    errors.extend(discovery_errors(server_name, &report));

    let qualified: Vec<String> = report
        .tools
        .iter()
        .filter(|t| t.server_name == server_name)
        .map(|t| t.qualified_name.clone())
        .collect();
    tool_count = qualified.len();
    discover_ok = tool_count > 0 && errors.is_empty();
    tools_sample = qualified
        .iter()
        .filter_map(|q| q.rsplit("__").next().map(str::to_string))
        .take(12)
        .collect();

    cleanup();

    let ok = discover_ok && errors.is_empty();
    let status = if ok {
        "ok".to_string()
    } else {
        "error".to_string()
    };

    McpTestResponse {
        ok,
        status,
        server_name: server_name.to_string(),
        url,
        transport,
        discover_ok,
        tool_count,
        tools_sample,
        warnings,
        errors,
        duration_ms: started.elapsed().as_millis() as u64,
        hint: PROBE_HINT_WORKER,
    }
}

#[allow(clippy::too_many_arguments)]
fn fail_response(
    server_name: &str,
    url: Option<String>,
    transport: Option<String>,
    warnings: Vec<String>,
    errors: Vec<String>,
    started: Instant,
    discover_ok: bool,
    tool_count: usize,
    tools_sample: Vec<String>,
) -> McpTestResponse {
    McpTestResponse {
        ok: false,
        status: "error".to_string(),
        server_name: server_name.to_string(),
        url,
        transport,
        discover_ok,
        tool_count,
        tools_sample,
        warnings,
        errors,
        duration_ms: started.elapsed().as_millis() as u64,
        hint: PROBE_HINT_WORKER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_on_localhost_url() {
        let cfg = json!({"type":"streamable-http","url":"http://127.0.0.1:8001/mcp-streamable"});
        let w = collect_warnings("sqlbot-streamable", &cfg);
        assert!(w.iter().any(|x| x.contains("127.0.0.1")));
    }
}
