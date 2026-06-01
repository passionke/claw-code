//! Admin chat translation via active gateway LLM. Author: kejiqing

use api::{
    max_tokens_for_model, InputContentBlock, InputMessage, MessageRequest, OpenAiCompatConfig,
    OutputContentBlock, ProviderClient,
};
use axum::http::StatusCode;
use axum::Json;

use crate::claw_tap_cluster_state::active_llm_upstream;
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;

const MAX_TRANSLATE_INPUT_CHARS: usize = 8_000;

#[derive(Debug, serde::Deserialize)]
pub struct GatewayTranslateRequest {
    pub text: String,
    #[serde(default = "default_target_language")]
    #[serde(rename = "targetLanguage")]
    pub target_language: String,
}

fn default_target_language() -> String {
    "zh-CN".to_string()
}

#[derive(Debug, serde::Serialize)]
pub struct GatewayTranslateResponse {
    #[serde(rename = "translatedText")]
    pub translated_text: String,
}

pub struct GatewayTranslateApiError {
    pub status: StatusCode,
    pub message: String,
}

impl GatewayTranslateApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

fn build_translate_prompt(text: &str, target_language: &str) -> String {
    format!(
        "Translate the following text into {target_language}. Output only the translation body — no explanation, no title, no markdown code fences.\n\n{text}"
    )
}

fn message_response_text(content: &[OutputContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            OutputContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

pub async fn translate_text_with_active_llm(
    db: &GatewaySessionDb,
    text: &str,
    target_language: &str,
) -> Result<String, GatewayTranslateApiError> {
    let src = text.trim();
    if src.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "text must be non-empty",
        ));
    }
    if src.chars().count() > MAX_TRANSLATE_INPUT_CHARS {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            format!("text too long (max {MAX_TRANSLATE_INPUT_CHARS} chars)"),
        ));
    }

    let active = gateway_global_settings::load_active_llm_runtime(db)
        .await
        .map_err(|e| {
            GatewayTranslateApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load active LLM failed: {e}"),
            )
        })?
        .ok_or_else(|| {
            GatewayTranslateApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no active LLM model configured in Admin",
            )
        })?;

    let (upstream, model) = active_llm_upstream(&active)
        .map_err(|e| GatewayTranslateApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    let api_key = active.api_key.trim();
    if api_key.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "active LLM apiKey missing",
        ));
    }

    let provider = ProviderClient::from_explicit_openai_compat(
        api_key,
        upstream,
        OpenAiCompatConfig::openai(),
    );
    let prompt = build_translate_prompt(src, target_language.trim());
    let req = MessageRequest {
        model: model.clone(),
        max_tokens: max_tokens_for_model(&model).min(8192),
        messages: vec![InputMessage {
            role: "user".to_string(),
            content: vec![InputContentBlock::Text { text: prompt }],
        }],
        system: None,
        tools: None,
        tool_choice: None,
        thinking_enabled: Some(false),
        stream: false,
        ..Default::default()
    };

    let resp = provider.send_message(&req).await.map_err(|e| {
        GatewayTranslateApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("LLM translate failed: {e}"),
        )
    })?;
    let out = message_response_text(&resp.content);
    if out.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_GATEWAY,
            "LLM returned empty translation",
        ));
    }
    Ok(out)
}

pub async fn post_gateway_translate_handler(
    db: &GatewaySessionDb,
    body: GatewayTranslateRequest,
) -> Result<Json<GatewayTranslateResponse>, GatewayTranslateApiError> {
    let translated = translate_text_with_active_llm(db, &body.text, &body.target_language).await?;
    Ok(Json(GatewayTranslateResponse {
        translated_text: translated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_includes_target_and_source() {
        let p = build_translate_prompt("hello", "zh-CN");
        assert!(p.contains("zh-CN"));
        assert!(p.contains("hello"));
    }
}
