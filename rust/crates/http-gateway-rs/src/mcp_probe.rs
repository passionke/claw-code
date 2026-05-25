//! MCP connectivity + auth probe for admin (`POST /v1/mcp/test`). Author: kejiqing

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use runtime::{
    mcp_tool_name, ConfigLoader, McpServerManager, McpServerManagerError, McpToolCallResult,
    RuntimeConfig,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;

const PROBE_HINT_WORKER: &str = "探测在 Gateway 进程环境执行。若 solve 使用 pool worker 容器，URL 须用 worker 能访问宿主机 SQLBot 的地址（如 http://host.docker.internal:8001/mcp-streamable），勿写 127.0.0.1。";

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
    pub has_mcp_start: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_start_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_start_message: Option<String>,
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
    if config.get("headers").and_then(Value::as_object).is_none_or(|o| o.is_empty()) {
        out.push(
            "未配置 headers：HTTP MCP 通常需要 Authorization 或 x-ak/x-sk。".to_string(),
        );
    }
    let _ = server_name;
    out
}

fn discovery_errors(
    server_name: &str,
    report: &runtime::McpToolDiscoveryReport,
) -> Vec<String> {
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

fn mcp_tool_result_text(result: &McpToolCallResult) -> String {
    if let Ok(v) = serde_json::to_value(result) {
        if let Some(text) = v.pointer("/content/0/text").and_then(Value::as_str) {
            return text.to_string();
        }
    }
    String::new()
}

fn mcp_start_result_snippet(result: &McpToolCallResult) -> (bool, String) {
    if result.is_error == Some(true) {
        let text = mcp_tool_result_text(result);
        return (
            false,
            if text.is_empty() {
                "mcp_start isError".to_string()
            } else {
                truncate(&text, 240)
            },
        );
    }
    let text = mcp_tool_result_text(result);
    if text.is_empty() {
        return (true, "mcp_start returned empty text".to_string());
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("\"code\":0") || lower.contains("\"code\": 0") {
        return (true, truncate(&text, 240));
    }
    if lower.contains("403") || lower.contains("invalid authorization") {
        return (false, truncate(&text, 240));
    }
    (true, truncate(&text, 240))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}…", &s[..max])
}

async fn write_probe_settings(work_dir: &Path, server_name: &str, config: &Value) -> Result<(), String> {
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

/// Run MCP discover (+ optional `mcp_start`) for one server config.
pub async fn probe_mcp_server(
    server_name: &str,
    config: &Value,
    probe_mcp_start: bool,
) -> McpTestResponse {
    let started = Instant::now();
    let mut warnings = collect_warnings(server_name, config);
    let url = url_from_config(config);
    let transport = transport_from_config(config);

    let work_dir = probe_temp_dir(server_name);
    let mut errors = Vec::new();
    let mut discover_ok = false;
    let mut tool_count = 0usize;
    let mut tools_sample = Vec::new();
    let mut has_mcp_start = false;
    let mut mcp_start_ok = None;
    let mut mcp_start_message = None;

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
            has_mcp_start: false,
            mcp_start_ok,
            mcp_start_message,
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
            has_mcp_start,
            mcp_start_ok,
            mcp_start_message,
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
                has_mcp_start,
                mcp_start_ok,
                mcp_start_message,
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
            errors.push(format!("server {server_name:?} not registered (check type/url)"));
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
            has_mcp_start,
            mcp_start_ok,
            mcp_start_message,
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

    let start_tool = mcp_tool_name(server_name, "mcp_start");
    has_mcp_start = qualified.iter().any(|q| q == &start_tool);

    if probe_mcp_start {
        if !has_mcp_start {
            mcp_start_ok = Some(false);
            mcp_start_message = Some("tools/list 中无 mcp_start".to_string());
            warnings.push("未探测 mcp_start：工具列表里没有 mcp_start。".to_string());
        } else {
            match manager
                .call_tool(&start_tool, Some(json!({})), None)
                .await
            {
                Ok(resp) => {
                    if let Some(err) = resp.error {
                        mcp_start_ok = Some(false);
                        mcp_start_message =
                            Some(format!("{} ({})", err.message, err.code));
                        errors.push(format!("mcp_start RPC error: {}", err.message));
                    } else if let Some(result) = resp.result {
                        let (ok, msg) = mcp_start_result_snippet(&result);
                        mcp_start_ok = Some(ok);
                        mcp_start_message = Some(msg);
                        if !ok {
                            errors.push("mcp_start 返回失败或鉴权错误".to_string());
                        }
                    } else {
                        mcp_start_ok = Some(false);
                        mcp_start_message = Some("empty mcp_start result".to_string());
                    }
                }
                Err(McpServerManagerError::UnknownTool { qualified_name }) => {
                    mcp_start_ok = Some(false);
                    mcp_start_message = Some(format!("unknown tool {qualified_name}"));
                    errors.push(mcp_start_message.clone().unwrap());
                }
                Err(e) => {
                    mcp_start_ok = Some(false);
                    mcp_start_message = Some(e.to_string());
                    errors.push(e.to_string());
                }
            }
        }
    }

    cleanup();

    let auth_ok = mcp_start_ok.unwrap_or(discover_ok);
    let ok = discover_ok && errors.is_empty() && mcp_start_ok.unwrap_or(true);
    let status = if ok {
        "ok".to_string()
    } else if discover_ok {
        "degraded".to_string()
    } else {
        "error".to_string()
    };

    let _ = auth_ok;
    McpTestResponse {
        ok,
        status,
        server_name: server_name.to_string(),
        url,
        transport,
        discover_ok,
        tool_count,
        tools_sample,
        has_mcp_start,
        mcp_start_ok,
        mcp_start_message,
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
    has_mcp_start: bool,
    mcp_start_ok: Option<bool>,
    mcp_start_message: Option<String>,
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
        has_mcp_start,
        mcp_start_ok,
        mcp_start_message,
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
