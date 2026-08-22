//! Normalize OpenAI Chat Completions / Responses into AgentCompletionRequest.
//! Author: kejiqing
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

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
}

#[derive(Debug, Clone, Deserialize)]
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
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    pub extra_session: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(default)]
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
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
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
) -> Value {
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
        "usage": Value::Null,
        "nerogate": {
            "sessionId": session_id,
            "turnId": turn_id
        }
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
) -> Value {
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
        "usage": Value::Null,
        "nerogate": {
            "sessionId": session_id,
            "turnId": turn_id
        }
    })
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
}
