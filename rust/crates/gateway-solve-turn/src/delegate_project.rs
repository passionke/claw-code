//! `delegate_project` gateway tool: resolve session, enqueue specialist solve, register fan-in. Author: kejiqing

use std::time::Duration;

use api::ToolDefinition;
use reqwest::blocking::Client;
use runtime::ToolError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::gateway_stdout::{
    emit_delegate_active, emit_delegate_clear, suppress_further_live_deltas,
};
use crate::mcp_call_context::GatewayMcpCallContext;

pub const DELEGATE_PROJECT_TOOL_NAME: &str = "delegate_project";

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

fn fetch_terminal_report(
    client: &Client,
    base: &str,
    session_id: &str,
    turn_id: &str,
    proj_id: i64,
) -> Result<String, ToolError> {
    let url = format!(
        "{base}/v1/biz_advice_report?sessionId={session_id}&turnId={turn_id}&projId={proj_id}&stream=false"
    );
    let v = {
        let mut last = None;
        let mut body = None;
        for _ in 0..8 {
            match get_json(client, &url) {
                Ok(v) => {
                    body = Some(v);
                    break;
                }
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        body.ok_or_else(|| last.unwrap_or_else(|| ToolError::new("fetch terminal report failed")))?
    };
    report_text_from_biz_payload(&v)
        .ok_or_else(|| ToolError::new("delegate specialist returned empty report"))
}

fn poll_task_terminal(client: &Client, base: &str, task_id: &str) -> Result<String, ToolError> {
    let url = format!("{base}/v1/tasks/{task_id}");
    for _ in 0..7200 {
        let v = match get_json(client, &url) {
            Ok(v) => v,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(500));
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
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    Err(ToolError::new("delegate task poll timeout"))
}

pub fn run_delegate_project(
    mcp: &GatewayMcpCallContext,
    input: &Value,
) -> Result<String, ToolError> {
    let parsed: DelegateProjectInput = serde_json::from_value(input.clone())
        .map_err(|e| ToolError::new(format!("invalid delegate_project args: {e}")))?;
    if parsed.user_prompt.trim().is_empty() {
        return Err(ToolError::new("userPrompt cannot be empty"));
    }
    if parsed.session_id.is_some() {
        return Err(ToolError::new(
            "sessionId must not be passed to delegate_project",
        ));
    }
    if parsed.proj_id < 1 {
        return Err(ToolError::new("projId must be >= 1"));
    }
    let base = gateway_base()?;
    let initiator = initiator_proj_id()?;
    let parent_session = mcp.clawcode_session_id().to_string();
    let client = http_client()?;

    let resolved =
        resolve_delegate_session(&client, &base, initiator, &parent_session, parsed.proj_id)?;

    let async_resp = enqueue_solve_async(
        &client,
        &base,
        parsed.proj_id,
        &resolved.delegate_session_id,
        parsed.user_prompt.trim(),
        parsed.extra_session.as_ref(),
    )?;

    emit_delegate_active(
        &async_resp.task_id,
        &async_resp.turn_id,
        parsed.proj_id,
        parsed.proj_id,
    )
    .map_err(|e| ToolError::new(format!("delegate.active stdout: {e}")))?;

    let terminal = poll_task_terminal(&client, &base, &async_resp.task_id).inspect_err(|_| {
        let _ = emit_delegate_clear();
    })?;

    emit_delegate_clear().map_err(|e| ToolError::new(format!("delegate.clear stdout: {e}")))?;

    let report = fetch_terminal_report(
        &client,
        &base,
        &resolved.delegate_session_id,
        &async_resp.turn_id,
        parsed.proj_id,
    )?;

    suppress_further_live_deltas();

    serde_json::to_string_pretty(&json!({
        "status": terminal,
        "projId": parsed.proj_id,
        "delegateSessionCreated": resolved.created,
        "delegateSessionId": resolved.delegate_session_id,
        "delegateTurnId": async_resp.turn_id,
        "message": report,
    }))
    .map_err(|e| ToolError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
