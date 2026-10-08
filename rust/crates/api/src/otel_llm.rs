//! OTEL generation spans for LLM calls (`gen_ai.*`). Author: kejiqing

use telemetry::{log_prompts_enabled, otel_enabled, OtelSpanGuard};

use crate::types::{MessageRequest, MessageResponse, Usage};

#[derive(Debug)]
pub struct LlmOtelGuard {
    inner: OtelSpanGuard,
    completion: String,
}

impl LlmOtelGuard {
    /// Start `llm.chat` with Anthropic system label (legacy callers).
    #[must_use]
    pub fn start(model: &str, prompt_preview: &str) -> Option<Self> {
        Self::start_with_system("anthropic", model, prompt_preview)
    }

    /// Start `llm.chat` under current context. Author: kejiqing
    #[must_use]
    pub fn start_with_system(system: &str, model: &str, prompt_preview: &str) -> Option<Self> {
        if !otel_enabled() {
            return None;
        }
        let guard = OtelSpanGuard::start("claw-api", "llm.chat", None)?;
        guard.set_attribute("gen_ai.system", system.to_string());
        guard.set_attribute("gen_ai.operation.name", "chat");
        guard.set_attribute("gen_ai.request.model", model.to_string());
        if let Ok(turn_id) = std::env::var("CLAW_TURN_ID") {
            let turn_id = turn_id.trim();
            if !turn_id.is_empty() {
                guard.set_attribute("turn_id", turn_id.to_string());
            }
        }
        if log_prompts_enabled() {
            guard.set_attribute("gen_ai.prompt", prompt_preview.to_string());
        }
        Some(Self {
            inner: guard,
            completion: String::new(),
        })
    }

    /// Optional iteration attr when the caller has agent-loop context. Author: kejiqing
    #[allow(dead_code)]
    pub fn set_iteration(&self, iteration: u64) {
        self.inner.set_attribute("iteration", iteration.to_string());
    }

    pub fn push_completion_delta(&mut self, text: &str) {
        if !log_prompts_enabled() {
            return;
        }
        self.completion.push_str(text);
    }

    pub fn finish_with_response(&mut self, response: &MessageResponse) {
        if log_prompts_enabled() {
            self.completion = message_response_completion_preview(response);
        }
        self.finish_with_usage(&response.usage, Some(response.model.as_str()));
    }

    pub fn finish_with_usage(&self, usage: &Usage, response_model: Option<&str>) {
        if log_prompts_enabled() && !self.completion.is_empty() {
            self.inner
                .set_attribute("gen_ai.completion", self.completion.clone());
        }
        if let Some(model) = response_model {
            self.inner
                .set_attribute("gen_ai.response.model", model.to_string());
        }
        self.record_usage(usage);
        self.inner.set_ok();
    }

    pub fn finish_error(&self, message: impl Into<String>) {
        self.inner.set_error(message);
    }

    fn record_usage(&self, usage: &Usage) {
        self.inner
            .set_attribute("gen_ai.usage.input_tokens", usage.input_tokens.to_string());
        self.inner.set_attribute(
            "gen_ai.usage.output_tokens",
            usage.output_tokens.to_string(),
        );
        if usage.cache_creation_input_tokens > 0 {
            self.inner.set_attribute(
                "gen_ai.usage.cache_creation_input_tokens",
                usage.cache_creation_input_tokens.to_string(),
            );
        }
        if usage.cache_read_input_tokens > 0 {
            self.inner.set_attribute(
                "gen_ai.usage.cache_read_input_tokens",
                usage.cache_read_input_tokens.to_string(),
            );
        }
    }
}

#[must_use]
pub fn message_request_prompt_preview(request: &MessageRequest) -> String {
    let mut parts = Vec::new();
    for message in &request.messages {
        for block in &message.content {
            if let crate::types::InputContentBlock::Text { text } = block {
                let t = text.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
        }
    }
    parts.join("\n")
}

#[must_use]
pub fn message_response_completion_preview(response: &MessageResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|block| {
            if let crate::types::OutputContentBlock::Text { text } = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_otel_guard_start_none_when_otel_off() {
        std::env::remove_var("CLAW_OTEL_ENABLED");
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        assert!(LlmOtelGuard::start("m", "p").is_none());
        assert!(LlmOtelGuard::start_with_system("openai", "m", "p").is_none());
    }

    #[test]
    fn prompt_preview_joins_text_blocks() {
        let request = MessageRequest {
            model: "m".to_string(),
            messages: vec![crate::types::InputMessage {
                role: "user".to_string(),
                content: vec![
                    crate::types::InputContentBlock::Text {
                        text: "hello".to_string(),
                    },
                    crate::types::InputContentBlock::Text {
                        text: "world".to_string(),
                    },
                ],
            }],
            max_tokens: 16,
            ..Default::default()
        };
        assert_eq!(message_request_prompt_preview(&request), "hello\nworld");
    }
}
