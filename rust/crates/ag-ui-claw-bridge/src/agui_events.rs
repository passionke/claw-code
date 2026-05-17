//! AG-UI event types (v1 subset). Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAgentInput {
    #[serde(rename = "threadId")]
    pub thread_id: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    pub messages: Vec<AgentMessage>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default, rename = "forwardedProps")]
    pub forwarded_props: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[allow(clippy::enum_variant_names)]
pub enum AgUiEvent {
    RunStarted {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "runId")]
        run_id: String,
    },
    TextMessageStart {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    TextMessageContent {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
    },
    TextMessageEnd {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    ToolCallStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
    },
    ToolCallEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        ok: bool,
    },
    Interrupt {
        #[serde(rename = "interruptId")]
        interrupt_id: String,
        reason: String,
        payload: Value,
    },
    InterruptResolved {
        #[serde(rename = "interruptId")]
        interrupt_id: String,
    },
    RunFinished {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "runId")]
        run_id: String,
    },
    RunError {
        message: String,
    },
}

impl AgUiEvent {
    pub fn sse_data(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

pub fn last_user_text(messages: &[AgentMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .filter(|s| !s.trim().is_empty())
}

pub fn ds_id_from_input(input: &RunAgentInput) -> Option<i64> {
    let props = input.forwarded_props.as_ref()?;
    props.get("dsId").and_then(serde_json::Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ds_id_from_forwarded_props() {
        let input = RunAgentInput {
            thread_id: "t".into(),
            run_id: "r".into(),
            messages: vec![],
            tools: vec![],
            forwarded_props: Some(serde_json::json!({"dsId": 42})),
        };
        assert_eq!(ds_id_from_input(&input), Some(42));
    }
}
