//! `ApiClient` backed by non-streaming `send_message` so we can drive
//! [`runtime::ConversationRuntime`] without duplicating the CLI streaming stack.

use std::collections::BTreeSet;
use std::io;

use api::{
    InputContentBlock, InputMessage, MessageRequest, OutputContentBlock, ProviderClient,
    ToolChoice, ToolDefinition, ToolResultContentBlock,
};
use runtime::{
    ApiClient, ApiRequest, AssistantEvent, ContentBlock, ConversationMessage, MessageRole,
    PromptCacheEvent, RuntimeError,
};
use serde_json::json;

use crate::render_minimal::{push_output_block, response_to_events};

/// Minimal render module (no terminal colors) to reuse response parsing.
mod render_minimal {
    use std::io::Write;

    use api::{MessageResponse, OutputContentBlock, ToolResultContentBlock};
    use runtime::AssistantEvent;

    pub fn push_output_block(
        block: OutputContentBlock,
        out: &mut (dyn Write + Send),
        events: &mut Vec<AssistantEvent>,
        pending_tool: &mut Option<(String, String, String)>,
        streaming_tool_input: bool,
        block_has_thinking_summary: &mut bool,
    ) -> Result<(), RuntimeError> {
        match block {
            OutputContentBlock::Text { text } => {
                if !text.is_empty() {
                    let _ = write!(out, "{text}");
                    let _ = out.flush();
                    events.push(AssistantEvent::TextDelta(text));
                }
            }
            OutputContentBlock::ToolUse { id, name, input } => {
                let initial_input = if streaming_tool_input
                    && input.is_object()
                    && input.as_object().is_some_and(serde_json::Map::is_empty)
                {
                    String::new()
                } else {
                    input.to_string()
                };
                *pending_tool = Some((id, name, initial_input));
            }
            OutputContentBlock::Thinking { .. } | OutputContentBlock::RedactedThinking { .. } => {
                *block_has_thinking_summary = true;
            }
        }
        Ok(())
    }

    pub fn response_to_events(
        response: MessageResponse,
        out: &mut (dyn Write + Send),
    ) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let mut events = Vec::new();
        let mut pending_tool = None;

        for block in response.content {
            let mut block_has_thinking_summary = false;
            push_output_block(
                block,
                out,
                &mut events,
                &mut pending_tool,
                false,
                &mut block_has_thinking_summary,
            )?;
            if let Some((id, name, input)) = pending_tool.take() {
                events.push(AssistantEvent::ToolUse { id, name, input });
            }
        }

        events.push(AssistantEvent::Usage(response.usage.token_usage()));
        events.push(AssistantEvent::MessageStop);
        Ok(events)
    }
}

fn convert_messages(messages: &[ConversationMessage]) -> Vec<InputMessage> {
    messages
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                MessageRole::System | MessageRole::User | MessageRole::Tool => "user",
                MessageRole::Assistant => "assistant",
            };
            let content = message
                .blocks
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => InputContentBlock::Text { text: text.clone() },
                    ContentBlock::ToolUse { id, name, input } => InputContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: serde_json::from_str(input)
                            .unwrap_or_else(|_| json!({ "raw": input })),
                    },
                    ContentBlock::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                        ..
                    } => InputContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: vec![ToolResultContentBlock::Text {
                            text: output.clone(),
                        }],
                        is_error: *is_error,
                    },
                })
                .collect::<Vec<_>>();
            (!content.is_empty()).then(|| InputMessage {
                role: role.to_string(),
                content,
            })
        })
        .collect()
}

/// Drives the model via `send_message` (non-streaming) per `stream()` call.
pub struct BlockingRoundTripClient {
    runtime: tokio::runtime::Runtime,
    client: ProviderClient,
    session_id: String,
    model: String,
    max_tokens: u32,
    enable_tools: bool,
    allowed_tools: Option<BTreeSet<String>>,
    tool_definitions: Vec<ToolDefinition>,
}

impl BlockingRoundTripClient {
    /// `tool_registry` supplies definitions; `allowed_tools` filters like the CLI.
    pub fn new(
        client: ProviderClient,
        session_id: impl Into<String>,
        model: impl Into<String>,
        max_tokens: u32,
        enable_tools: bool,
        allowed_tools: Option<BTreeSet<String>>,
        tool_definitions: Vec<ToolDefinition>,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: tokio::runtime::Runtime::new().map_err(|e| e.to_string())?,
            client,
            session_id: session_id.into(),
            model: model.into(),
            max_tokens,
            enable_tools,
            allowed_tools,
            tool_definitions,
        })
    }

    fn tools_for_request(&self) -> Option<Vec<ToolDefinition>> {
        if !self.enable_tools {
            return None;
        }
        let defs = match &self.allowed_tools {
            Some(allowed) => self
                .tool_definitions
                .iter()
                .filter(|d| allowed.contains(&d.name))
                .cloned()
                .collect::<Vec<_>>(),
            None => self.tool_definitions.clone(),
        };
        (!defs.is_empty()).then_some(defs)
    }
}

impl ApiClient for BlockingRoundTripClient {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let message_request = MessageRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            messages: convert_messages(&request.messages),
            system: (!request.system_prompt.is_empty()).then(|| request.system_prompt.join("\n\n")),
            tools: self.tools_for_request(),
            tool_choice: self.enable_tools.then_some(ToolChoice::Auto),
            stream: false,
            reasoning_effort: None,
            ..Default::default()
        };

        let response = self
            .runtime
            .block_on(self.client.send_message(&message_request))
            .map_err(|e| RuntimeError::new(format!("{}: {e}", self.session_id)))?;

        let mut sink = io::sink();
        let mut events = response_to_events(response, &mut sink)?;

        if let Some(record) = self.client.take_last_prompt_cache_record() {
            if let Some(cache_break) = record.cache_break {
                events.push(AssistantEvent::PromptCache(PromptCacheEvent {
                    unexpected: cache_break.unexpected,
                    reason: cache_break.reason,
                    previous_cache_read_input_tokens: cache_break.previous_cache_read_input_tokens,
                    current_cache_read_input_tokens: cache_break.current_cache_read_input_tokens,
                    token_drop: cache_break.token_drop,
                }));
            }
        }

        Ok(events)
    }
}
