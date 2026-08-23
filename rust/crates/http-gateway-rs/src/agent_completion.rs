//! Normalize OpenAI Chat Completions / Responses into AgentCompletionRequest.
//! Author: kejiqing
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Internal completion request shared by Chat Completions and Responses adapters.
#[derive(Debug, Clone)]
pub struct AgentCompletionRequest {
    pub proj_id: i64,
    pub model_alias: String,
    pub user_prompt: String,
    pub system_instructions: Option<String>,
    /// Caller-stable conversation key (Chat `user` or Responses `conversation`).
    pub conversation_key: Option<String>,
    /// Existing gateway session when continuing.
    pub session_id: Option<String>,
    /// Responses `previous_response_id` when provided.
    pub previous_response_id: Option<String>,
    pub stream: bool,
    pub timeout_seconds: Option<u64>,
    pub extra_session: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub content: Value,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    /// OpenAI `user` — stable conversation key for session continuity.
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    #[schema(value_type = Option<Vec<Object>>)]
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub extra_session: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub input: Value,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub conversation: Option<String>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<Vec<Object>>)]
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub extra_session: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiErrorBody {
    pub error: OpenAiErrorDetail,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: Option<String>,
    pub param: Option<String>,
}

pub fn openai_error(message: impl Into<String>, code: &str, error_type: &str) -> OpenAiErrorBody {
    OpenAiErrorBody {
        error: OpenAiErrorDetail {
            message: message.into(),
            error_type: error_type.into(),
            code: Some(code.into()),
            param: None,
        },
    }
}

fn content_to_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                } else if let Some(t) = part.as_str() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            out
        }
        other => other
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| other.to_string()),
    }
}

fn reject_nonempty_tools(tools: Option<&Vec<Value>>) -> Result<(), OpenAiErrorBody> {
    if let Some(tools) = tools {
        if !tools.is_empty() {
            return Err(openai_error(
                "function tools are not supported on this Agent endpoint; omit tools or pass []",
                "unsupported_feature",
                "invalid_request_error",
            ));
        }
    }
    Ok(())
}

/// Convert Chat Completions messages into AgentCompletionRequest (without proj/session resolve).
pub fn normalize_chat_completions(
    req: &ChatCompletionsRequest,
) -> Result<(AgentCompletionRequest, String), OpenAiErrorBody> {
    reject_nonempty_tools(req.tools.as_ref())?;
    if req.messages.is_empty() {
        return Err(openai_error(
            "messages must be a non-empty array",
            "invalid_request",
            "invalid_request_error",
        ));
    }
    let mut system_parts: Vec<String> = Vec::new();
    let mut history_parts: Vec<String> = Vec::new();
    let mut last_user: Option<String> = None;
    for msg in &req.messages {
        let role = msg.role.trim().to_ascii_lowercase();
        let text = content_to_text(&msg.content);
        match role.as_str() {
            "system" | "developer" => {
                if !text.trim().is_empty() {
                    system_parts.push(text);
                }
            }
            "assistant" => {
                history_parts.push(format!("Assistant: {text}"));
            }
            "user" => {
                if let Some(prev) = last_user.take() {
                    history_parts.push(format!("User: {prev}"));
                }
                last_user = Some(text);
            }
            other => {
                history_parts.push(format!("{other}: {text}"));
            }
        }
    }
    let user_prompt = last_user.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        openai_error(
            "messages must include a non-empty user message",
            "invalid_request",
            "invalid_request_error",
        )
    })?;
    let mut prompt = String::new();
    if !history_parts.is_empty() {
        prompt.push_str("## Prior conversation\n");
        prompt.push_str(&history_parts.join("\n"));
        prompt.push_str("\n\n");
    }
    prompt.push_str("## Current user request\n");
    prompt.push_str(&user_prompt);
    let system_instructions = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    if let Some(sys) = &system_instructions {
        prompt = format!("## Supplemental instructions\n{sys}\n\n{prompt}");
    }
    Ok((
        AgentCompletionRequest {
            proj_id: 0, // filled by auth
            model_alias: req.model.trim().to_string(),
            user_prompt: prompt,
            system_instructions,
            conversation_key: req
                .user
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            session_id: None,
            previous_response_id: None,
            stream: req.stream,
            timeout_seconds: req.timeout,
            extra_session: req.extra_session.clone(),
        },
        req.model.trim().to_string(),
    ))
}

fn responses_input_to_text(input: &Value) -> Result<String, OpenAiErrorBody> {
    match input {
        Value::String(s) if !s.trim().is_empty() => Ok(s.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(s) = item.as_str() {
                    parts.push(s.to_string());
                    continue;
                }
                if let Some(content) = item.get("content") {
                    let t = content_to_text(content);
                    if !t.trim().is_empty() {
                        parts.push(t);
                    }
                    continue;
                }
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    parts.push(t.to_string());
                }
            }
            let joined = parts.join("\n");
            if joined.trim().is_empty() {
                Err(openai_error(
                    "input must contain non-empty text",
                    "invalid_request",
                    "invalid_request_error",
                ))
            } else {
                Ok(joined)
            }
        }
        _ => Err(openai_error(
            "input must be a string or array",
            "invalid_request",
            "invalid_request_error",
        )),
    }
}

pub fn normalize_responses(
    req: &ResponsesRequest,
) -> Result<(AgentCompletionRequest, String), OpenAiErrorBody> {
    reject_nonempty_tools(req.tools.as_ref())?;
    let mut prompt = responses_input_to_text(&req.input)?;
    if let Some(instr) = req
        .instructions
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        prompt =
            format!("## Supplemental instructions\n{instr}\n\n## Current user request\n{prompt}");
    }
    Ok((
        AgentCompletionRequest {
            proj_id: 0,
            model_alias: req.model.trim().to_string(),
            user_prompt: prompt,
            system_instructions: req.instructions.clone(),
            conversation_key: req
                .conversation
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            session_id: None,
            previous_response_id: req
                .previous_response_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            stream: req.stream,
            timeout_seconds: req.timeout,
            extra_session: req.extra_session.clone(),
        },
        req.model.trim().to_string(),
    ))
}

pub fn chat_completion_response(
    model: &str,
    turn_id: &str,
    session_id: &str,
    content: &str,
    created: i64,
    usage_rows: &[crate::session_db::TurnModelUsageRow],
) -> Value {
    let (usage, usage_by_model) = openai_usage_from_rows(usage_rows);
    let mut nerogate = json!({
        "sessionId": session_id,
        "turnId": turn_id
    });
    if let Some(by_model) = usage_by_model {
        nerogate["usageByModel"] = by_model;
    }
    json!({
        "id": turn_id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": usage,
        "nerogate": nerogate
    })
}

pub fn chat_completion_stream_chunks(
    model: &str,
    turn_id: &str,
    session_id: &str,
    content: &str,
    created: i64,
) -> Vec<String> {
    let delta = json!({
        "id": turn_id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": content },
            "finish_reason": null
        }],
        "nerogate": { "sessionId": session_id, "turnId": turn_id }
    });
    let done = json!({
        "id": turn_id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }],
        "nerogate": { "sessionId": session_id, "turnId": turn_id }
    });
    vec![
        format!("data: {}\n\n", delta),
        format!("data: {}\n\n", done),
        "data: [DONE]\n\n".to_string(),
    ]
}

pub fn responses_api_response(
    model: &str,
    turn_id: &str,
    session_id: &str,
    content: &str,
    created_ms: i64,
    usage_rows: &[crate::session_db::TurnModelUsageRow],
) -> Value {
    let (usage, usage_by_model) = responses_usage_from_rows(usage_rows);
    let mut nerogate = json!({
        "sessionId": session_id,
        "turnId": turn_id
    });
    if let Some(by_model) = usage_by_model {
        nerogate["usageByModel"] = by_model;
    }
    json!({
        "id": turn_id,
        "object": "response",
        "created_at": created_ms / 1000,
        "status": "completed",
        "model": model,
        "output": [{
            "type": "message",
            "id": format!("{turn_id}_msg"),
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": content
            }]
        }],
        "usage": usage,
        "nerogate": nerogate
    })
}

/// Map tap `gateway_model_usage` rows → OpenAI Chat Completions `usage` + by-model breakdown.
/// Empty rows → `(Null, None)` — never invent tokens. Author: kejiqing
#[must_use]
pub fn openai_usage_from_rows(
    rows: &[crate::session_db::TurnModelUsageRow],
) -> (Value, Option<Value>) {
    if rows.is_empty() {
        return (Value::Null, None);
    }
    let mut by_model: BTreeMap<String, (u32, u32, u32, u32)> = BTreeMap::new();
    for row in rows {
        let entry = by_model.entry(row.model.clone()).or_default();
        entry.0 = entry.0.saturating_add(row.input_tokens);
        entry.1 = entry.1.saturating_add(row.output_tokens);
        entry.2 = entry.2.saturating_add(row.cache_creation_input_tokens);
        entry.3 = entry.3.saturating_add(row.cache_read_input_tokens);
    }
    let mut prompt_tokens = 0u32;
    let mut completion_tokens = 0u32;
    let mut cached_tokens = 0u32;
    let mut usage_by_model = Vec::new();
    for (model, (input, output, cache_create, cache_read)) in &by_model {
        let model_prompt = input
            .saturating_add(*cache_create)
            .saturating_add(*cache_read);
        prompt_tokens = prompt_tokens.saturating_add(model_prompt);
        completion_tokens = completion_tokens.saturating_add(*output);
        cached_tokens = cached_tokens.saturating_add(*cache_read);
        usage_by_model.push(json!({
            "model": model,
            "prompt_tokens": model_prompt,
            "completion_tokens": output,
            "total_tokens": model_prompt.saturating_add(*output),
            "prompt_tokens_details": { "cached_tokens": cache_read }
        }));
    }
    let usage = json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens.saturating_add(completion_tokens),
        "prompt_tokens_details": { "cached_tokens": cached_tokens }
    });
    (usage, Some(Value::Array(usage_by_model)))
}

/// Responses API usage shape (`input_tokens` / `output_tokens`). Author: kejiqing
#[must_use]
pub fn responses_usage_from_rows(
    rows: &[crate::session_db::TurnModelUsageRow],
) -> (Value, Option<Value>) {
    let (chat_usage, by_model) = openai_usage_from_rows(rows);
    if chat_usage.is_null() {
        return (Value::Null, None);
    }
    let prompt = chat_usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion = chat_usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = chat_usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let usage = json!({
        "input_tokens": prompt,
        "output_tokens": completion,
        "total_tokens": prompt.saturating_add(completion),
        "input_tokens_details": { "cached_tokens": cached }
    });
    let by_model = by_model.map(|arr| {
        Value::Array(
            arr.as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|item| {
                    let model = item.get("model").cloned().unwrap_or(Value::Null);
                    let input = item
                        .get("prompt_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let output = item
                        .get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cached = item
                        .pointer("/prompt_tokens_details/cached_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    json!({
                        "model": model,
                        "input_tokens": input,
                        "output_tokens": output,
                        "total_tokens": input.saturating_add(output),
                        "input_tokens_details": { "cached_tokens": cached }
                    })
                })
                .collect(),
        )
    });
    (usage, by_model)
}

pub fn extract_solve_message(output_json: Option<&Value>, output_text: &str) -> String {
    if let Some(v) = output_json {
        if let Some(msg) = v.get("message").and_then(Value::as_str) {
            if !msg.is_empty() {
                return msg.to_string();
            }
        }
        if let Some(msg) = v.pointer("/outputJson/message").and_then(Value::as_str) {
            if !msg.is_empty() {
                return msg.to_string();
            }
        }
    }
    output_text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_normalize_uses_last_user() {
        let req = ChatCompletionsRequest {
            model: "agent".into(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: json!("be concise"),
                },
                ChatMessage {
                    role: "user".into(),
                    content: json!("hi"),
                },
                ChatMessage {
                    role: "assistant".into(),
                    content: json!("hello"),
                },
                ChatMessage {
                    role: "user".into(),
                    content: json!("write sql"),
                },
            ],
            stream: false,
            user: Some("chat-1".into()),
            timeout: None,
            tools: None,
            extra_session: None,
        };
        let (norm, model) = normalize_chat_completions(&req).unwrap();
        assert_eq!(model, "agent");
        assert!(norm.user_prompt.contains("write sql"));
        assert!(norm.user_prompt.contains("Prior conversation"));
        assert_eq!(norm.conversation_key.as_deref(), Some("chat-1"));
    }

    #[test]
    fn rejects_tools() {
        let req = ChatCompletionsRequest {
            model: "agent".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: json!("x"),
            }],
            stream: false,
            user: None,
            timeout: None,
            tools: Some(vec![json!({"type":"function"})]),
            extra_session: None,
        };
        assert!(normalize_chat_completions(&req).is_err());
    }

    #[test]
    fn openai_usage_empty_is_null() {
        let (usage, by_model) = openai_usage_from_rows(&[]);
        assert!(usage.is_null());
        assert!(by_model.is_none());
    }

    #[test]
    fn openai_usage_sums_turn_and_by_model() {
        use crate::session_db::TurnModelUsageRow;
        let rows = vec![
            TurnModelUsageRow {
                provider: Some("openai".into()),
                model: "m1".into(),
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 2,
                cache_read_input_tokens: 3,
                source: "tap".into(),
            },
            TurnModelUsageRow {
                provider: Some("openai".into()),
                model: "m1".into(),
                input_tokens: 1,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                source: "tap".into(),
            },
            TurnModelUsageRow {
                provider: Some("anthropic".into()),
                model: "m2".into(),
                input_tokens: 100,
                output_tokens: 20,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 7,
                source: "tap".into(),
            },
        ];
        let (usage, by_model) = openai_usage_from_rows(&rows);
        // m1 prompt = (10+2+3)+(1+0+0)=16; m2 prompt = 100+0+7=107 → 123
        assert_eq!(usage["prompt_tokens"], 123);
        assert_eq!(usage["completion_tokens"], 26);
        assert_eq!(usage["total_tokens"], 149);
        assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 10);
        let arr = by_model.expect("by model").as_array().cloned().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["model"], "m1");
        assert_eq!(arr[0]["prompt_tokens"], 16);
        assert_eq!(arr[1]["model"], "m2");
        assert_eq!(arr[1]["prompt_tokens"], 107);
    }
}
