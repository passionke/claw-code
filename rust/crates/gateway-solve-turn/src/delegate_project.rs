//! `delegate_project` gateway tool: resolve session, enqueue specialist solve, passthrough live SSE. Author: kejiqing

use std::time::Duration;

use api::ToolDefinition;
use futures_util::StreamExt;
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use runtime::ToolError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::gateway_stdout::{emit_report_delta_passthrough, suppress_further_live_deltas};
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
            "Delegate the user question to a specialist project. Streams the specialist reply to the user. \
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
        // Reverse-tunnel keep-alives go stale; reuse then fails with "error sending request".
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

fn extract_delta_text(data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data).ok()?;
    v.get("text")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
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

#[must_use]
fn task_status_is_terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

/// Apply one SSE event block. `true` means the live stream is finished. Author: kejiqing
fn apply_sse_event_block(block: &str, emitted: &mut bool) -> Result<bool, ToolError> {
    let mut event_name = String::new();
    let mut data_lines = Vec::new();
    for line in block.lines() {
        if let Some(ev) = line.strip_prefix("event:") {
            event_name = ev.trim().to_string();
        } else if let Some(d) = line.strip_prefix("data:") {
            data_lines.push(d.trim());
        }
    }
    if event_name == "biz.report.delta" {
        let data = data_lines.join("\n");
        if let Some(text) = extract_delta_text(&data) {
            emit_report_delta_passthrough(&text)
                .map_err(|e| ToolError::new(format!("emit delta: {e}")))?;
            *emitted = true;
        }
    }
    if event_name == "biz.report.done" {
        // Stop only. Done payload is a full snapshot, not another live paragraph.
        return Ok(true);
    }
    if event_name == "biz.report.error" {
        return Err(ToolError::new("specialist live report error"));
    }
    Ok(false)
}

fn feed_sse_bytes(buf: &mut String, bytes: &[u8], emitted: &mut bool) -> Result<bool, ToolError> {
    buf.push_str(&String::from_utf8_lossy(bytes));
    while let Some(pos) = buf.find("\n\n") {
        let block = buf[..pos].to_string();
        *buf = buf[pos + 2..].to_string();
        if apply_sse_event_block(&block, emitted)? {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn fetch_task_status(client: &reqwest::Client, task_url: &str) -> Option<String> {
    let resp = client.get(task_url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    v.get("status").and_then(Value::as_str).map(str::to_string)
}

async fn wait_task_terminal(client: &reqwest::Client, task_url: &str) {
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    for _ in 0..7200 {
        poll.tick().await;
        if let Some(status) = fetch_task_status(client, task_url).await {
            if task_status_is_terminal(&status) {
                return;
            }
        }
    }
}

async fn read_sse_until_done_or_task(
    client: &reqwest::Client,
    resp: reqwest::Response,
    task_url: &str,
) -> Result<bool, ToolError> {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut emitted = false;
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    None => break,
                    Some(Err(e)) => return Err(ToolError::new(format!("sse chunk: {e}"))),
                    Some(Ok(bytes)) => {
                        if feed_sse_bytes(&mut buf, &bytes, &mut emitted)? {
                            return Ok(emitted);
                        }
                    }
                }
            }
            _ = poll.tick() => {
                if let Some(status) = fetch_task_status(client, task_url).await {
                    if task_status_is_terminal(&status) {
                        break;
                    }
                }
            }
        }
    }
    Ok(emitted)
}

/// Copy specialist live deltas. Stop on `biz.report.done` **or** specialist task
/// terminal — SSE may never see `done` (keep-alive / late hub subscribe). Author: kejiqing
fn passthrough_live_sse(
    _client: &Client,
    base: &str,
    session_id: &str,
    turn_id: &str,
    proj_id: i64,
) -> Result<bool, ToolError> {
    let sse_url = format!(
        "{base}/v1/biz_advice_report?sessionId={session_id}&turnId={turn_id}&projId={proj_id}&stream=true"
    );
    let task_url = format!("{base}/v1/tasks/{session_id}");
    let rt = tokio::runtime::Handle::try_current()
        .map_err(|_| ToolError::new("delegate_project passthrough requires tokio runtime"))?;
    rt.block_on(async {
        let async_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|e| ToolError::new(format!("live sse client: {e}")))?;
        tokio::select! {
            resp = async_client
                .get(&sse_url)
                .header(ACCEPT, "text/event-stream")
                .send() => {
                let resp = resp.map_err(|e| ToolError::new(format!("live SSE: {e}")))?;
                if !resp.status().is_success() {
                    return Err(ToolError::new(format!("live SSE status {}", resp.status())));
                }
                read_sse_until_done_or_task(&async_client, resp, &task_url).await
            }
            () = wait_task_terminal(&async_client, &task_url) => Ok(false)
        }
    })
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

    let mut passthrough_emitted = false;
    if async_resp.status == "running" || async_resp.status == "queued" {
        passthrough_emitted = passthrough_live_sse(
            &client,
            &base,
            &async_resp.task_id,
            &async_resp.turn_id,
            parsed.proj_id,
        )?;
    }
    let terminal = poll_task_terminal(&client, &base, &async_resp.task_id)?;
    let report = fetch_terminal_report(
        &client,
        &base,
        &resolved.delegate_session_id,
        &async_resp.turn_id,
        parsed.proj_id,
    )?;
    if !passthrough_emitted {
        emit_report_delta_passthrough(&report)
            .map_err(|e| ToolError::new(format!("emit delegate report: {e}")))?;
    }
    // Live already has the specialist body. Router restatement must not stream again.
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
    fn extract_delta_text_parses_json() {
        let t = extract_delta_text(r#"{"text":"hello"}"#).unwrap();
        assert_eq!(t, "hello");
    }

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
    fn task_status_is_terminal_covers_end_states() {
        assert!(task_status_is_terminal("succeeded"));
        assert!(task_status_is_terminal("failed"));
        assert!(task_status_is_terminal("cancelled"));
        assert!(!task_status_is_terminal("running"));
        assert!(!task_status_is_terminal("queued"));
    }

    #[test]
    fn apply_sse_done_stops_without_marking_emitted() {
        let mut emitted = false;
        let finished = apply_sse_event_block(
            "event: biz.report.done\ndata: {\"reportText\":\"full snapshot\"}",
            &mut emitted,
        )
        .unwrap();
        assert!(finished);
        assert!(!emitted);
    }

    #[test]
    fn apply_sse_done_after_delta_keeps_emitted_and_stops() {
        let mut emitted = true;
        let finished = apply_sse_event_block(
            "event: biz.report.done\ndata: {\"reportText\":\"full snapshot\"}",
            &mut emitted,
        )
        .unwrap();
        assert!(finished);
        assert!(emitted);
    }

    #[test]
    fn apply_sse_error_is_err() {
        let mut emitted = false;
        let err =
            apply_sse_event_block("event: biz.report.error\ndata: {}", &mut emitted).unwrap_err();
        assert!(err.to_string().contains("live report error"));
    }
}
