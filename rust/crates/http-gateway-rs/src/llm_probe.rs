//! LLM connectivity probe for admin (`POST /v1/gateway/global-settings/llm-models/test`). Author: kejiqing

use std::time::Instant;

use api::{
    max_tokens_for_model, InputMessage, MessageRequest, OpenAiCompatConfig, OutputContentBlock,
    ProviderClient, Usage,
};
use serde::{Deserialize, Serialize};

use crate::claw_tap_cluster_state::active_llm_upstream;
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;

const DEFAULT_PROBE_PROMPT: &str = "Reply with exactly: pong";
const PROBE_HINT: &str =
    "探测在 Gateway 进程直连上游 OpenAI-compat API，不经过 clawTap。thinking 开关与采样参数按请求体传入。";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestRequest {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(default)]
    pub prompt: Option<String>,
    /// `default` = omit field (provider default); `true` / `false` = explicit.
    #[serde(default)]
    pub thinking_enabled: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default, rename = "topP")]
    pub top_p: Option<f64>,
    #[serde(default, rename = "maxTokens")]
    pub max_tokens: Option<u32>,
    #[serde(default, rename = "frequencyPenalty")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, rename = "presencePenalty")]
    pub presence_penalty: Option<f64>,
    #[serde(default, rename = "reasoningEffort")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestUsagePublic {
    #[serde(rename = "inputTokens")]
    pub input_tokens: u32,
    #[serde(rename = "outputTokens")]
    pub output_tokens: u32,
    #[serde(rename = "totalTokens")]
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTestResponse {
    pub ok: bool,
    pub status: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(rename = "upstreamUrl")]
    pub upstream_url: String,
    #[serde(rename = "responseText", skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
    #[serde(rename = "thinkingText", skip_serializing_if = "Option::is_none")]
    pub thinking_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<LlmTestUsagePublic>,
    #[serde(rename = "thinkingEnabled")]
    pub thinking_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    pub hint: &'static str,
}

fn usage_public(usage: &Usage) -> LlmTestUsagePublic {
    LlmTestUsagePublic {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens(),
    }
}

fn extract_response_parts(content: &[OutputContentBlock]) -> (String, String) {
    let mut text = String::new();
    let mut thinking = String::new();
    for block in content {
        match block {
            OutputContentBlock::Text { text: t } => text.push_str(t),
            OutputContentBlock::Thinking { thinking: t, .. } => thinking.push_str(t),
            _ => {}
        }
    }
    (text.trim().to_string(), thinking.trim().to_string())
}

fn validate_sampling(v: Option<f64>, label: &str, min: f64, max: f64) -> Result<(), String> {
    if let Some(x) = v {
        if !x.is_finite() || x < min || x > max {
            return Err(format!("{label} must be between {min} and {max}"));
        }
    }
    Ok(())
}

pub async fn probe_llm_model(
    db: &GatewaySessionDb,
    req: LlmTestRequest,
) -> Result<LlmTestResponse, String> {
    let model_id = req.model_id.trim();
    if model_id.is_empty() {
        return Err("modelId must be non-empty".into());
    }
    validate_sampling(req.temperature, "temperature", 0.0, 2.0)?;
    validate_sampling(req.top_p, "topP", 0.0, 1.0)?;
    validate_sampling(req.frequency_penalty, "frequencyPenalty", -2.0, 2.0)?;
    validate_sampling(req.presence_penalty, "presencePenalty", -2.0, 2.0)?;
    if let Some(max_tokens) = req.max_tokens {
        if max_tokens == 0 || max_tokens > 32_768 {
            return Err("maxTokens must be between 1 and 32768".into());
        }
    }

    let runtime = gateway_global_settings::load_llm_runtime_for_model_id(db, model_id).await?;
    let (upstream, wire_model) = active_llm_upstream(&runtime)?;
    let api_key = runtime.api_key.trim();
    if api_key.is_empty() {
        return Err("apiKey is not configured".into());
    }

    let prompt = req
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_PROBE_PROMPT)
        .to_string();
    let max_tokens = req.max_tokens.unwrap_or_else(|| {
        let cap = max_tokens_for_model(&wire_model);
        cap.min(512).max(64.min(cap))
    });

    let message_req = MessageRequest {
        model: wire_model.clone(),
        max_tokens,
        messages: vec![InputMessage::user_text(prompt)],
        thinking_enabled: req.thinking_enabled,
        temperature: req.temperature,
        top_p: req.top_p,
        frequency_penalty: req.frequency_penalty,
        presence_penalty: req.presence_penalty,
        reasoning_effort: req
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        stream: false,
        ..Default::default()
    };

    let started = Instant::now();
    let provider = ProviderClient::from_explicit_openai_compat(
        api_key,
        upstream.clone(),
        OpenAiCompatConfig::openai(),
    );
    match provider.send_message(&message_req).await {
        Ok(resp) => {
            let (response_text, thinking_text) = extract_response_parts(&resp.content);
            let mut warnings = Vec::new();
            if response_text.is_empty() && thinking_text.is_empty() {
                warnings.push("上游返回空内容（无 text / thinking 块）".to_string());
            }
            let ok = !response_text.is_empty() || !thinking_text.is_empty();
            Ok(LlmTestResponse {
                ok,
                status: if ok {
                    "ok".to_string()
                } else {
                    "empty_response".to_string()
                },
                model_id: runtime.model_id,
                model_name: runtime.model_name,
                upstream_url: upstream,
                response_text: (!response_text.is_empty()).then_some(response_text),
                thinking_text: (!thinking_text.is_empty()).then_some(thinking_text),
                usage: Some(usage_public(&resp.usage)),
                thinking_enabled: req.thinking_enabled,
                temperature: req.temperature,
                top_p: req.top_p,
                warnings,
                errors: vec![],
                duration_ms: started.elapsed().as_millis() as u64,
                hint: PROBE_HINT,
            })
        }
        Err(e) => Ok(LlmTestResponse {
            ok: false,
            status: "upstream_error".to_string(),
            model_id: runtime.model_id,
            model_name: runtime.model_name,
            upstream_url: upstream,
            response_text: None,
            thinking_text: None,
            usage: None,
            thinking_enabled: req.thinking_enabled,
            temperature: req.temperature,
            top_p: req.top_p,
            warnings: vec![],
            errors: vec![e.to_string()],
            duration_ms: started.elapsed().as_millis() as u64,
            hint: PROBE_HINT,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_response_parts_splits_text_and_thinking() {
        let content = vec![
            OutputContentBlock::Thinking {
                thinking: "step".into(),
                signature: None,
            },
            OutputContentBlock::Text {
                text: "pong".into(),
            },
        ];
        let (text, thinking) = extract_response_parts(&content);
        assert_eq!(text, "pong");
        assert_eq!(thinking, "step");
    }

    #[test]
    fn validate_sampling_rejects_out_of_range() {
        assert!(validate_sampling(Some(3.0), "temperature", 0.0, 2.0).is_err());
        assert!(validate_sampling(Some(0.5), "temperature", 0.0, 2.0).is_ok());
    }
}
