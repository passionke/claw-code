//! `delegate_project_tool`: enqueue specialist solve, fan-in live, persist body into **router session**. Author: kejiqing
//!
//! Specialist only speaks. This tool writes the body under the router's own session home and
//! returns `reportPath` (no inlined `message`).

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use api::ToolDefinition;
use reqwest::blocking::Client;
use runtime::ToolError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::gateway_stdout::{emit_delegate_active, emit_delegate_clear};
use crate::mcp_call_context::GatewayMcpCallContext;

pub const DELEGATE_PROJECT_TOOL_NAME: &str = "delegate_project_tool";

/// Router-session relative dir for delegated report bodies. Author: kejiqing
pub const DELEGATE_AGENTS_RESULTS_DIR_REL: &str = ".claw/delegate-agents-results";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DelegateProjectInput {
    #[serde(rename = "projId")]
    proj_id: i64,
    user_prompt: String,
    #[serde(default, rename = "extraSession")]
    extra_session: Option<Value>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveSessionResponse {
    delegate_session_id: String,
    #[allow(dead_code)]
    root_session_id: String,
    created: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SolveAsyncResponse {
    task_id: String,
    turn_id: String,
    #[allow(dead_code)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct TaskGetResponse {
    status: String,
    #[serde(default, rename = "result")]
    result: Option<Value>,
}

#[must_use]
pub fn delegate_project_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: DELEGATE_PROJECT_TOOL_NAME.to_string(),
        description: Some(
            "Delegate the user question to a specialist project. Live report streams via gateway fan-in. \
             On success the body is stored under this router session at reportPath (not inlined). \
             Required: projId, userPrompt. Pass extraSession unchanged. Do not pass sessionId."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "projId": {
                    "type": "integer",
                    "description": "Target specialist project id (must be in delegate-targets allowlist)"
                },
                "userPrompt": {
                    "type": "string",
                    "description": "Question for the specialist (sub-question when mixed intent)"
                },
                "extraSession": {
                    "type": "object",
                    "description": "Business context; pass through from user turn unchanged"
                }
            },
            "required": ["projId", "userPrompt"]
        }),
    }
}

fn gateway_base() -> Result<String, ToolError> {
    std::env::var("CLAW_GATEWAY_BASE")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::new("CLAW_GATEWAY_BASE not set in worker"))
}

fn initiator_proj_id() -> Result<i64, ToolError> {
    std::env::var("CLAW_PROJ_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&id| id >= 1)
        .ok_or_else(|| ToolError::new("CLAW_PROJ_ID not set in worker"))
}

fn http_client() -> Result<Client, ToolError> {
    Client::builder()
        .timeout(Duration::from_secs(3600))
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|e| ToolError::new(format!("http client: {e}")))
}

fn post_json(client: &Client, url: &str, body: &Value) -> Result<Value, ToolError> {
    let resp = client
        .post(url)
        .json(body)
        .send()
        .map_err(|e| ToolError::new(format!("POST {url}: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| ToolError::new(format!("read body: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::new(format!("POST {url} {status}: {text}")));
    }
    serde_json::from_str(&text).map_err(|e| ToolError::new(format!("json: {e}: {text}")))
}

fn get_json(client: &Client, url: &str) -> Result<Value, ToolError> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| ToolError::new(format!("GET {url}: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| ToolError::new(format!("read body: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::new(format!("GET {url} {status}: {text}")));
    }
    serde_json::from_str(&text).map_err(|e| ToolError::new(format!("json: {e}: {text}")))
}

fn resolve_delegate_session(
    client: &Client,
    base: &str,
    initiator: i64,
    parent_session_id: &str,
    delegate_proj_id: i64,
) -> Result<ResolveSessionResponse, ToolError> {
    let url = format!("{base}/v1/projects/{initiator}/delegate/resolve-session");
    let body = json!({
        "parentSessionId": parent_session_id,
        "delegateProjId": delegate_proj_id,
    });
    let v = post_json(client, &url, &body)?;
    serde_json::from_value(v).map_err(|e| ToolError::new(format!("resolve-session: {e}")))
}

fn enqueue_solve_async(
    client: &Client,
    base: &str,
    proj_id: i64,
    session_id: &str,
    user_prompt: &str,
    extra_session: Option<&Value>,
) -> Result<SolveAsyncResponse, ToolError> {
    let url = format!("{base}/v1/solve_async");
    let mut body = json!({
        "projId": proj_id,
        "userPrompt": user_prompt,
        "sessionId": session_id,
    });
    if let Some(es) = extra_session {
        body["extraSession"] = es.clone();
    }
    let v = post_json(client, &url, &body)?;
    serde_json::from_value(v).map_err(|e| ToolError::new(format!("solve_async: {e}")))
}

fn report_text_from_biz_payload(v: &Value) -> Option<String> {
    v.get("reportText")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            v.get("reportJson")
                .and_then(|j| j.get("message"))
                .and_then(|m| m.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn session_home_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Path under **router session home**, keyed by router turnId. Author: kejiqing
#[must_use]
pub fn router_delegate_report_rel(router_turn_id: &str) -> String {
    format!("{DELEGATE_AGENTS_RESULTS_DIR_REL}/{router_turn_id}.md")
}

/// nas-api `.claw/` relative name for finalize fallback. Author: kejiqing
#[must_use]
pub fn router_delegate_report_claw_file(router_turn_id: &str) -> String {
    format!("delegate-agents-results/{router_turn_id}.md")
}

fn ensure_router_report_file(
    session_home: &Path,
    router_turn_id: &str,
) -> Result<(String, PathBuf), ToolError> {
    if router_turn_id.trim().is_empty() {
        return Err(ToolError::new(
            "router turnId required to persist delegate report",
        ));
    }
    let rel = router_delegate_report_rel(router_turn_id);
    let path = session_home.join(&rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ToolError::new(format!("create {}: {e}", parent.display())))?;
    }
    if !path.exists() {
        fs::write(&path, "")
            .map_err(|e| ToolError::new(format!("create {}: {e}", path.display())))?;
    }
    Ok((rel, path))
}

fn append_router_report(path: &Path, chunk: &str) -> Result<(), ToolError> {
    if chunk.is_empty() {
        return Ok(());
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| ToolError::new(format!("open {}: {e}", path.display())))?;
    f.write_all(chunk.as_bytes())
        .map_err(|e| ToolError::new(format!("append {}: {e}", path.display())))?;
    Ok(())
}

fn append_serial_separator(path: &Path) -> Result<(), ToolError> {
    if path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| ToolError::new(format!("open {}: {e}", path.display())))?;
        writeln!(f).map_err(|e| ToolError::new(format!("append sep: {e}")))?;
        writeln!(f, "---").map_err(|e| ToolError::new(format!("append sep: {e}")))?;
        writeln!(f).map_err(|e| ToolError::new(format!("append sep: {e}")))?;
    }
    Ok(())
}

/// Follow specialist live SSE and write into the router-session file. Author: kejiqing
fn stream_specialist_into_router_file(
    client: &Client,
    base: &str,
    specialist_session_id: &str,
    specialist_turn_id: &str,
    specialist_proj_id: i64,
    out_path: &Path,
    stop: Arc<AtomicBool>,
) {
    let url = format!(
        "{base}/v1/biz_advice_report?sessionId={specialist_session_id}&turnId={specialist_turn_id}&projId={specialist_proj_id}&stream=true"
    );
    let Ok(resp) = client.get(&url).send() else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let mut event_name = String::new();
    let reader = BufReader::new(resp);
    for line in reader.lines().flatten() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if line.starts_with("event:") {
            event_name = line.trim_start_matches("event:").trim().to_string();
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            if event_name == "biz.report.delta" {
                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                        let _ = append_router_report(out_path, text);
                    }
                }
            } else if event_name == "biz.report.done" {
                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    // If stream had no deltas, take done payload once.
                    let empty = fs::metadata(out_path).map(|m| m.len() == 0).unwrap_or(true);
                    if empty {
                        if let Some(text) = report_text_from_biz_payload(&v) {
                            let _ = append_router_report(out_path, &text);
                        } else if let Some(text) = v
                            .get("reportText")
                            .and_then(|t| t.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            let _ = append_router_report(out_path, text);
                        }
                    }
                }
                break;
            }
            event_name.clear();
        }
        if line.is_empty() {
            event_name.clear();
        }
    }
}

fn fill_router_file_from_terminal_if_empty(
    client: &Client,
    base: &str,
    specialist_session_id: &str,
    specialist_turn_id: &str,
    specialist_proj_id: i64,
    out_path: &Path,
) -> Result<(), ToolError> {
    let empty = fs::metadata(out_path).map(|m| m.len() == 0).unwrap_or(true);
    if !empty {
        return Ok(());
    }
    let url = format!(
        "{base}/v1/biz_advice_report?sessionId={specialist_session_id}&turnId={specialist_turn_id}&projId={specialist_proj_id}&stream=false"
    );
    let mut last = None;
    for _ in 0..8 {
        match get_json(client, &url) {
            Ok(v) => {
                let text = report_text_from_biz_payload(&v)
                    .ok_or_else(|| ToolError::new("delegate specialist returned empty report"))?;
                append_router_report(out_path, &text)?;
                return Ok(());
            }
            Err(e) => {
                last = Some(e);
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Err(last.unwrap_or_else(|| ToolError::new("fill router report file failed")))
}

fn poll_task_terminal(client: &Client, base: &str, task_id: &str) -> Result<String, ToolError> {
    let url = format!("{base}/v1/tasks/{task_id}");
    for _ in 0..7200 {
        let v = match get_json(client, &url) {
            Ok(v) => v,
            Err(_) => {
                thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        let parsed: TaskGetResponse =
            serde_json::from_value(v).map_err(|e| ToolError::new(format!("task: {e}")))?;
        match parsed.status.as_str() {
            "succeeded" => return Ok("succeeded".into()),
            "failed" | "cancelled" => {
                let detail = parsed
                    .result
                    .and_then(|r| r.get("detail").and_then(|d| d.as_str().map(str::to_string)))
                    .unwrap_or_else(|| "delegate failed".into());
                return Err(ToolError::new(detail));
            }
            _ => thread::sleep(Duration::from_millis(500)),
        }
    }
    Err(ToolError::new("delegate task poll timeout"))
}

pub fn run_delegate_project(
    mcp: &GatewayMcpCallContext,
    input: &Value,
) -> Result<String, ToolError> {
    let parsed: DelegateProjectInput = serde_json::from_value(input.clone())
        .map_err(|e| ToolError::new(format!("invalid delegate_project_tool args: {e}")))?;
    if parsed.user_prompt.trim().is_empty() {
        return Err(ToolError::new("userPrompt cannot be empty"));
    }
    if parsed.session_id.is_some() {
        return Err(ToolError::new(
            "sessionId must not be passed to delegate_project_tool",
        ));
    }
    if parsed.proj_id < 1 {
        return Err(ToolError::new("projId must be >= 1"));
    }
    let base = gateway_base()?;
    let initiator = initiator_proj_id()?;
    let router_session = mcp.clawcode_session_id().to_string();
    let router_turn = mcp.turn_id.clone();
    let client = http_client()?;

    let resolved =
        resolve_delegate_session(&client, &base, initiator, &router_session, parsed.proj_id)?;

    let async_resp = enqueue_solve_async(
        &client,
        &base,
        parsed.proj_id,
        &resolved.delegate_session_id,
        parsed.user_prompt.trim(),
        parsed.extra_session.as_ref(),
    )?;

    let (report_path, out_path) = ensure_router_report_file(&session_home_dir(), &router_turn)?;
    append_serial_separator(&out_path)?;

    emit_delegate_active(
        &async_resp.task_id,
        &async_resp.turn_id,
        parsed.proj_id,
        parsed.proj_id,
    )
    .map_err(|e| ToolError::new(format!("delegate.active stdout: {e}")))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = Arc::clone(&stop);
    let base_t = base.clone();
    let spec_session = resolved.delegate_session_id.clone();
    let spec_turn = async_resp.turn_id.clone();
    let spec_proj = parsed.proj_id;
    let out_t = out_path.clone();
    let stream_client = http_client()?;
    let stream_join = thread::spawn(move || {
        stream_specialist_into_router_file(
            &stream_client,
            &base_t,
            &spec_session,
            &spec_turn,
            spec_proj,
            &out_t,
            stop_t,
        );
    });

    let terminal = poll_task_terminal(&client, &base, &async_resp.task_id).inspect_err(|_| {
        stop.store(true, Ordering::Relaxed);
        let _ = emit_delegate_clear();
    })?;

    // Give the SSE reader a moment to finish done, then stop.
    thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    let _ = stream_join.join();

    emit_delegate_clear().map_err(|e| ToolError::new(format!("delegate.clear stdout: {e}")))?;

    fill_router_file_from_terminal_if_empty(
        &client,
        &base,
        &resolved.delegate_session_id,
        &async_resp.turn_id,
        parsed.proj_id,
        &out_path,
    )?;

    let written = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    if written == 0 {
        return Err(ToolError::new(
            "delegate_project_tool: router session report file empty",
        ));
    }

    serde_json::to_string_pretty(&json!({
        "status": terminal,
        "projId": parsed.proj_id,
        "delegateSessionCreated": resolved.created,
        "delegateSessionId": resolved.delegate_session_id,
        "delegateTurnId": async_resp.turn_id,
        "routerSessionId": router_session,
        "routerTurnId": router_turn,
        "reportPath": report_path,
    }))
    .map_err(|e| ToolError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn report_text_from_biz_payload_prefers_report_text() {
        let v = json!({
            "reportText": "steps here",
            "reportJson": {"message": "ignored"}
        });
        assert_eq!(
            report_text_from_biz_payload(&v).as_deref(),
            Some("steps here")
        );
    }

    #[test]
    fn tool_result_returns_report_path_not_message() {
        let body = json!({
            "status": "succeeded",
            "projId": 99011,
            "delegateSessionCreated": true,
            "delegateSessionId": "dgt_test",
            "delegateTurnId": "T_child",
            "routerSessionId": "sess_router",
            "routerTurnId": "T_router",
            "reportPath": ".claw/delegate-agents-results/T_router.md",
        });
        assert!(body.get("message").is_none());
        assert_eq!(
            body.get("reportPath").and_then(|v| v.as_str()),
            Some(".claw/delegate-agents-results/T_router.md")
        );
    }

    #[test]
    fn router_report_path_is_under_claw_by_turn() {
        let rel = router_delegate_report_rel("T_b");
        assert_eq!(rel, ".claw/delegate-agents-results/T_b.md");
        let dir = tempdir().expect("temp");
        let (got, path) = ensure_router_report_file(dir.path(), "T_b").expect("file");
        assert_eq!(got, rel);
        assert!(path.exists());
        append_router_report(&path, "hello").expect("append");
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }
}
