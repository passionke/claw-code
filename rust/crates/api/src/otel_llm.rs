//! OTEL generation spans for LLM calls (Langfuse `gen_ai.*`). Author: kejiqing

use telemetry::{log_prompts_enabled, otel_enabled, OtelSpanGuard};

use crate::types::{MessageRequest, MessageResponse, Usage};

#[derive(Debug)]
pub struct LlmOtelGuard {
    inner: OtelSpanGuard,
    completion: String,
}

impl LlmOtelGuard {
    #[must_use]
    pub fn start(model: &str, prompt_preview: &str) -> Option<Self> {
        if !otel_enabled() {
            return None;
        }
        let guard = OtelSpanGuard::start("claw-api", "llm.chat", None)?;
        guard.set_attribute("gen_ai.system", "anthropic");
        guard.set_attribute("gen_ai.operation.name", "chat");
        guard.set_attribute("gen_ai.request.model", model.to_string());
        guard.set_attribute("langfuse.observation.model.name", model.to_string());
        if log_prompts_enabled() {
            guard.set_attribute("gen_ai.prompt", truncate(prompt_preview, 8000));
        }
        Some(Self {
            inner: guard,
            completion: String::new(),
        })
    }

    pub fn push_completion_delta(&mut self, text: &str) {
        if !log_prompts_enabled() {
            return;
        }
        if self.completion.len() < 8000 {
            let remaining = 8000usize.saturating_sub(self.completion.len());
            self.completion
                .push_str(&text.chars().take(remaining).collect::<String>());
        }
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
        self.inner.set_attribute(
            "langfuse.observation.usage_details",
            serde_json::json!({
                "input": usage.input_tokens,
                "output": usage.output_tokens,
                "cache_creation_input_tokens": usage.cache_creation_input_tokens,
                "cache_read_input_tokens": usage.cache_read_input_tokens,
            })
            .to_string(),
        );
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

fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}
