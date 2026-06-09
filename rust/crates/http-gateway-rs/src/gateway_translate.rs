//! Admin chat translation via active gateway LLM + PG snapshot cache. Author: kejiqing

use api::{
    max_tokens_for_model, InputContentBlock, InputMessage, MessageRequest, OpenAiCompatConfig,
    OutputContentBlock, ProviderClient,
};
use axum::http::StatusCode;
use axum::Json;
use serde_json::Value;

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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ConversationTranslateTurnJson {
    pub index: i32,
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(rename = "userText")]
    pub user_text: String,
    #[serde(rename = "assistantText")]
    pub assistant_text: String,
    #[serde(rename = "userTextZh")]
    pub user_text_zh: String,
    #[serde(rename = "assistantTextZh")]
    pub assistant_text_zh: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ConversationTranslateSnapshotJson {
    #[serde(rename = "sourceFingerprint")]
    pub source_fingerprint: String,
    pub turns: Vec<ConversationTranslateTurnJson>,
    pub markdown: String,
    #[serde(rename = "targetLanguage")]
    pub target_language: String,
    #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct GetConversationTranslateResponse {
    pub snapshot: Option<ConversationTranslateSnapshotJson>,
}

#[derive(Debug, serde::Deserialize)]
pub struct PutConversationTranslateRequest {
    #[serde(rename = "sourceFingerprint")]
    pub source_fingerprint: String,
    pub turns: Vec<ConversationTranslateTurnJson>,
    pub markdown: String,
    #[serde(default = "default_target_language")]
    #[serde(rename = "targetLanguage")]
    pub target_language: String,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct PutConversationTranslateResponse {
    pub ok: bool,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

fn snapshot_turns_from_json(value: &Value) -> Result<Vec<ConversationTranslateTurnJson>, String> {
    let turns: Vec<ConversationTranslateTurnJson> =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid turns_json: {e}"))?;
    Ok(turns)
}

pub async fn get_conversation_translate_handler(
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
) -> Result<Json<GetConversationTranslateResponse>, GatewayTranslateApiError> {
    if proj_id < 1 {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let exists = db.session_exists(session_id, proj_id).await.map_err(|e| {
        GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    if !exists {
        return Err(GatewayTranslateApiError::new(
            StatusCode::NOT_FOUND,
            format!("session not found: {session_id} projId={proj_id}"),
        ));
    }
    let row = db
        .get_conversation_translate_snapshot(session_id, proj_id)
        .await
        .map_err(|e| {
            GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    let Some(row) = row else {
        return Ok(Json(GetConversationTranslateResponse { snapshot: None }));
    };
    let turns = snapshot_turns_from_json(&row.turns_json)
        .map_err(|e| GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(GetConversationTranslateResponse {
        snapshot: Some(ConversationTranslateSnapshotJson {
            source_fingerprint: row.source_fingerprint,
            turns,
            markdown: row.markdown,
            target_language: row.target_language,
            model_id: row.model_id,
            updated_at_ms: row.updated_at_ms,
        }),
    }))
}

pub async fn put_conversation_translate_handler(
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
    body: PutConversationTranslateRequest,
) -> Result<Json<PutConversationTranslateResponse>, GatewayTranslateApiError> {
    if proj_id < 1 {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let fingerprint = body.source_fingerprint.trim();
    if fingerprint.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "sourceFingerprint must be non-empty",
        ));
    }
    if body.turns.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "turns must be non-empty",
        ));
    }
    let markdown = body.markdown.trim();
    if markdown.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "markdown must be non-empty",
        ));
    }
    let exists = db.session_exists(session_id, proj_id).await.map_err(|e| {
        GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    if !exists {
        return Err(GatewayTranslateApiError::new(
            StatusCode::NOT_FOUND,
            format!("session not found: {session_id} projId={proj_id}"),
        ));
    }
    let turns_json = serde_json::to_value(&body.turns).map_err(|e| {
        GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            format!("turns serialization failed: {e}"),
        )
    })?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    db.upsert_conversation_translate_snapshot(
        session_id,
        proj_id,
        fingerprint,
        &turns_json,
        markdown,
        body.target_language.trim(),
        body.model_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        now_ms,
    )
    .await
    .map_err(|e| GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(PutConversationTranslateResponse {
        ok: true,
        updated_at_ms: now_ms,
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
