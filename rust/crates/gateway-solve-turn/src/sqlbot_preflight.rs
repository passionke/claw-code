//! Gateway resolve preflight: before the first LLM turn, run `SQLBot` `mcp_start`,
//! `mcp_datasource_list`, and `mcp_datasource_tables`, then inject into session transcript.
//! Author: kejiqing

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{DirectToolExecutor, GatewaySolveTurnError};
use runtime::{
    ContentBlock, ConversationMessage, Session, ToolExecutor,
    GATEWAY_SQLBOT_MCP_DATASOURCE_LIST_TOOL, GATEWAY_SQLBOT_MCP_DATASOURCE_TABLES_TOOL,
    GATEWAY_SQLBOT_MCP_START_TOOL,
};
use serde_json::{json, Value};

const PREFLIGHT_ENV: &str = "CLAW_GATEWAY_SQLBOT_PREFLIGHT";

#[derive(Debug, Clone)]
struct SqlbotCredentials {
    token: String,
}

fn preflight_enabled() -> bool {
    match std::env::var(PREFLIGHT_ENV) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "off" | "no")
        }
        Err(_) => true,
    }
}

fn next_preflight_tool_use_id(step: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("claw_preflight_{step}_{nanos}")
}

fn session_has_tool_result(messages: &[ConversationMessage], tool_substr: &str) -> bool {
    messages.iter().any(|msg| {
        msg.blocks.iter().any(|block| {
            matches!(
                block,
                ContentBlock::ToolResult {
                    tool_name,
                    is_error: false,
                    ..
                } if tool_name.contains(tool_substr)
            )
        })
    })
}

fn last_successful_tool_output(
    messages: &[ConversationMessage],
    tool_substr: &str,
) -> Option<String> {
    let mut found = None;
    for msg in messages {
        for block in &msg.blocks {
            if let ContentBlock::ToolResult {
                tool_name,
                output,
                is_error: false,
                ..
            } = block
            {
                if tool_name.contains(tool_substr) {
                    found = Some(output.clone());
                }
            }
        }
    }
    found
}

fn inject_tool_exchange(
    session: &mut Session,
    tool_use_id: String,
    tool_name: &str,
    input: &str,
    output: String,
    is_error: bool,
) -> Result<(), GatewaySolveTurnError> {
    session
        .push_message(ConversationMessage::assistant(vec![
            ContentBlock::ToolUse {
                id: tool_use_id.clone(),
                name: tool_name.to_string(),
                input: input.to_string(),
            },
        ]))
        .map_err(|e| {
            crate::err(
                crate::HTTP_INTERNAL,
                format!("preflight persist assistant tool_use ({tool_name}): {e}"),
            )
        })?;
    session
        .push_message(ConversationMessage::tool_result(
            tool_use_id,
            tool_name,
            output,
            is_error,
        ))
        .map_err(|e| {
            crate::err(
                crate::HTTP_INTERNAL,
                format!("preflight persist tool_result ({tool_name}): {e}"),
            )
        })?;
    Ok(())
}

fn run_preflight_mcp(
    session: &mut Session,
    executor: &mut DirectToolExecutor,
    step: &str,
    tool_name: &str,
    input: &str,
) -> Result<String, GatewaySolveTurnError> {
    if !executor.allows_tool(tool_name) {
        return Err(crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight tool not allowed: {tool_name}"),
        ));
    }
    let tool_use_id = next_preflight_tool_use_id(step);
    let output = executor.execute(tool_name, input).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight {tool_name} failed: {e}"),
        )
    })?;
    let is_error = sqlbot_payload_is_error(&output);
    inject_tool_exchange(
        session,
        tool_use_id,
        tool_name,
        input,
        output.clone(),
        is_error,
    )?;
    if is_error {
        return Err(crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight {tool_name} returned error payload"),
        ));
    }
    Ok(output)
}

fn sqlbot_payload_is_error(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    if lower.contains("\"iserror\":true") || lower.contains("\"error\"") {
        return true;
    }
    parse_sqlbot_inner_json(output)
        .ok()
        .and_then(|v| v.get("code").and_then(Value::as_i64))
        .is_some_and(|code| code != 0)
}

/// Unwrap MCP tool wrapper (`content[0].text` JSON) to `SQLBot` `{code,data,msg}`.
fn parse_sqlbot_inner_json(output: &str) -> Result<Value, String> {
    let outer: Value = serde_json::from_str(output).map_err(|e| format!("outer json: {e}"))?;
    if let Some(text) = outer.pointer("/content/0/text").and_then(Value::as_str) {
        return serde_json::from_str(text).map_err(|e| format!("inner json: {e}"));
    }
    Ok(outer)
}

fn credentials_from_start_inner(inner: &Value) -> Option<SqlbotCredentials> {
    let data = inner.get("data")?;
    let token = data.get("access_token")?.as_str()?.to_string();
    data.get("chat_id").and_then(Value::as_i64)?;
    Some(SqlbotCredentials { token })
}

fn credentials_from_session(
    messages: &[ConversationMessage],
) -> Result<SqlbotCredentials, GatewaySolveTurnError> {
    let output = last_successful_tool_output(messages, "mcp_start")
        .ok_or_else(|| crate::err(crate::HTTP_INTERNAL, "preflight: mcp_start result missing"))?;
    let inner = parse_sqlbot_inner_json(&output).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: parse mcp_start output: {e}"),
        )
    })?;
    credentials_from_start_inner(&inner).ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_start missing access_token or chat_id",
        )
    })
}

/// Prefer `recommended_config == 1`, else first `status == Success`, else first row `id`.
fn pick_datasource_id_from_list_inner(inner: &Value) -> Option<i64> {
    let arr = inner.get("data")?.as_array()?;
    for item in arr {
        if item.get("recommended_config").and_then(Value::as_i64) == Some(1) {
            if let Some(id) = item.get("id").and_then(Value::as_i64) {
                return Some(id);
            }
        }
    }
    for item in arr {
        if item.get("status").and_then(Value::as_str) == Some("Success") {
            if let Some(id) = item.get("id").and_then(Value::as_i64) {
                return Some(id);
            }
        }
    }
    arr.first()
        .and_then(|item| item.get("id"))
        .and_then(Value::as_i64)
}

fn datasource_id_from_session(
    messages: &[ConversationMessage],
) -> Result<i64, GatewaySolveTurnError> {
    let output = last_successful_tool_output(messages, "mcp_datasource_list").ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_datasource_list result missing",
        )
    })?;
    let inner = parse_sqlbot_inner_json(&output).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: parse mcp_datasource_list output: {e}"),
        )
    })?;
    pick_datasource_id_from_list_inner(&inner).ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_datasource_list returned no datasource id",
        )
    })
}

fn ensure_mcp_start(
    session: &mut Session,
    executor: &mut DirectToolExecutor,
) -> Result<SqlbotCredentials, GatewaySolveTurnError> {
    if session_has_tool_result(&session.messages, "mcp_start") {
        return credentials_from_session(&session.messages);
    }
    if !executor.allows_tool(GATEWAY_SQLBOT_MCP_START_TOOL) {
        return Err(crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight tool not allowed: {GATEWAY_SQLBOT_MCP_START_TOOL}"),
        ));
    }
    let output = run_preflight_mcp(
        session,
        executor,
        "mcp_start",
        GATEWAY_SQLBOT_MCP_START_TOOL,
        "{}",
    )?;
    let inner = parse_sqlbot_inner_json(&output).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: parse mcp_start output: {e}"),
        )
    })?;
    credentials_from_start_inner(&inner).ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_start missing access_token or chat_id",
        )
    })
}

fn ensure_datasource_list(
    session: &mut Session,
    executor: &mut DirectToolExecutor,
    creds: &SqlbotCredentials,
) -> Result<(), GatewaySolveTurnError> {
    if session_has_tool_result(&session.messages, "mcp_datasource_list") {
        return Ok(());
    }
    let input = serde_json::to_string(&json!({ "token": creds.token })).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: encode mcp_datasource_list input: {e}"),
        )
    })?;
    run_preflight_mcp(
        session,
        executor,
        "datasource_list",
        GATEWAY_SQLBOT_MCP_DATASOURCE_LIST_TOOL,
        &input,
    )?;
    Ok(())
}

/// `SQLBot` `mcp_datasource_tables`: full table models under a datasource (MCP metadata).
/// Requires `token` from `mcp_start.access_token` and `datasource_id` for the current workspace.
/// Does not take `chat_id`. Author: kejiqing
fn ensure_datasource_tables(
    session: &mut Session,
    executor: &mut DirectToolExecutor,
    creds: &SqlbotCredentials,
    datasource_id: i64,
) -> Result<(), GatewaySolveTurnError> {
    if session_has_tool_result(&session.messages, "mcp_datasource_tables") {
        return Ok(());
    }
    let input = serde_json::to_string(&json!({
        "token": creds.token,
        "datasource_id": datasource_id,
    }))
    .map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: encode mcp_datasource_tables input: {e}"),
        )
    })?;
    run_preflight_mcp(
        session,
        executor,
        "datasource_tables",
        GATEWAY_SQLBOT_MCP_DATASOURCE_TABLES_TOOL,
        &input,
    )?;
    Ok(())
}

/// Run gateway resolve preflight on a **new** session turn (call after `push_user_text`, before LLM).
/// Catalog/schema land in session messages as real `SQLBot` MCP `tool_use` + `tool_result` (not static md).
pub(crate) fn run_gateway_resolve_preflight(
    session: &mut Session,
    executor: &mut DirectToolExecutor,
) -> Result<(), GatewaySolveTurnError> {
    if !preflight_enabled() {
        return Ok(());
    }
    let creds = ensure_mcp_start(session, executor)?;
    ensure_datasource_list(session, executor, &creds)?;
    let datasource_id = datasource_id_from_session(&session.messages)?;
    ensure_datasource_tables(session, executor, &creds, datasource_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn preflight_enabled_respects_env() {
        let _guard = env_lock();
        let key = PREFLIGHT_ENV;
        let prev: Option<OsString> = std::env::var_os(key);
        for (value, expected) in [
            (Some("0"), false),
            (Some("false"), false),
            (Some("OFF"), false),
            (Some("no"), false),
            (Some("1"), true),
            (Some("true"), true),
            (None, true),
        ] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
            assert_eq!(
                preflight_enabled(),
                expected,
                "preflight_enabled for {value:?}"
            );
        }
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn pick_datasource_prefers_recommended_config() {
        let inner = json!({
            "code": 0,
            "data": [
                {"id": 10, "status": "Success", "recommended_config": 0},
                {"id": 34, "status": "Success", "recommended_config": 1}
            ]
        });
        assert_eq!(pick_datasource_id_from_list_inner(&inner), Some(34));
    }

    #[test]
    fn parse_sqlbot_wrapped_start_payload() {
        let inner = json!({"code":0,"data":{"access_token":"tok","chat_id":99}});
        let wrapped = json!({"content":[{"type":"text","text": inner.to_string()}]});
        let creds = credentials_from_start_inner(
            &parse_sqlbot_inner_json(&wrapped.to_string()).expect("parse"),
        )
        .expect("creds");
        assert_eq!(creds.token, "tok");
    }
}
