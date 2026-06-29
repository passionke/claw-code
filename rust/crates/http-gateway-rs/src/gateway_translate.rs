//! Admin chat translation via active gateway LLM + PG snapshot cache. Author: kejiqing

use std::sync::Arc;

use api::{
    max_tokens_for_model, InputContentBlock, InputMessage, MessageRequest, OpenAiCompatConfig,
    OutputContentBlock, ProviderClient,
};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::claw_tap_cluster_state::active_llm_upstream;
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;

const MAX_TRANSLATE_INPUT_CHARS: usize = 8_000;
/// Per-chunk source budget; mirrors the former frontend orchestration. Author: kejiqing
const TRANSLATE_CHUNK_CHARS: usize = 3_000;
const TARGET_LANGUAGE_ZH: &str = "zh-CN";
/// A `translating` row older than this is treated as abandoned (worker died on
/// restart) and may be re-claimed. Author: kejiqing
const STALE_TRANSLATE_LOCK_MS: i64 = 10 * 60 * 1_000;

fn is_terminal_turn_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

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

// ---------------------------------------------------------------------------
// Whole-conversation translation (backend-orchestrated, async + poll).
// Source collection, chunking, fingerprint and markdown all live server-side so
// the admin UI only triggers a rebuild and polls the snapshot. Author: kejiqing
// ---------------------------------------------------------------------------

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}')
}

/// Treat text as already-Chinese when CJK chars dominate (>= 35%); skips machine
/// translation. Integer math avoids float-cast precision lints. Author: kejiqing
fn mostly_chinese(text: &str) -> bool {
    let chars: Vec<char> = text.trim().chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return false;
    }
    let cjk = chars.iter().filter(|c| is_cjk(**c)).count();
    cjk.saturating_mul(100) >= chars.len().saturating_mul(35)
}

/// Prefer a paragraph/line/space boundary in `[lo, hi)`, scanning backwards.
fn find_last_boundary(chars: &[char], lo: usize, hi: usize) -> Option<usize> {
    let hi = hi.min(chars.len());
    if hi <= lo {
        return None;
    }
    for j in (lo + 1..hi).rev() {
        if chars[j] == '\n' && chars[j - 1] == '\n' {
            return Some(j + 1);
        }
    }
    for j in (lo + 1..hi).rev() {
        if chars[j] == '\n' {
            return Some(j + 1);
        }
    }
    for j in (lo + 1..hi).rev() {
        if chars[j] == ' ' {
            return Some(j + 1);
        }
    }
    None
}

/// Split long text into <= `max_len`-char chunks on natural boundaries.
fn split_for_translation(text: &str, max_len: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max_len {
        return vec![trimmed.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0usize;
    let n = chars.len();
    while n - start > max_len {
        let hard = start + max_len;
        let half = start + max_len / 2;
        let cut = find_last_boundary(&chars, half, hard).unwrap_or(hard);
        let piece: String = chars[start..cut]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if !piece.is_empty() {
            chunks.push(piece);
        }
        start = cut;
        while start < n && chars[start].is_whitespace() {
            start += 1;
        }
    }
    let tail: String = chars[start..].iter().collect::<String>().trim().to_string();
    if !tail.is_empty() {
        chunks.push(tail);
    }
    chunks
}

/// Translate one field to zh; already-Chinese stays untouched.
async fn translate_text_to_zh(
    db: &GatewaySessionDb,
    text: &str,
    target_language: &str,
) -> Result<String, GatewayTranslateApiError> {
    let src = text.trim();
    if src.is_empty() {
        return Ok(String::new());
    }
    if mostly_chinese(src) {
        return Ok(src.to_string());
    }
    let chunks = split_for_translation(src, TRANSLATE_CHUNK_CHARS);
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if mostly_chinese(&chunk) {
            out.push(chunk);
        } else {
            out.push(translate_text_with_active_llm(db, &chunk, target_language).await?);
        }
    }
    Ok(out.join("\n\n"))
}

/// One settled turn's source text (user prompt + assistant body).
struct ConversationSource {
    index: i32,
    turn_id: String,
    user_text: String,
    assistant_text: String,
}

/// Collect only terminal turns (succeeded/failed/cancelled); in-flight turns are
/// skipped so the snapshot is a stable picture of the settled conversation.
async fn collect_terminal_turns(
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
) -> Result<Vec<ConversationSource>, GatewayTranslateApiError> {
    let turns = db
        .list_turns_for_session(session_id, proj_id)
        .await
        .map_err(|e| {
            GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    let mut out = Vec::new();
    let mut index = 0i32;
    for t in turns {
        if !is_terminal_turn_status(&t.status) {
            continue;
        }
        index += 1;
        let user_text = match t.user_prompt.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => "（无用户文案）".to_string(),
        };
        let assistant_text = if let Some(detail) = t
            .failure_detail
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            detail.to_string()
        } else if let Some(body) = t
            .report_body
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            body.to_string()
        } else {
            "（该轮次无已持久化的助手内容）".to_string()
        };
        out.push(ConversationSource {
            index,
            turn_id: t.turn_id,
            user_text,
            assistant_text,
        });
    }
    Ok(out)
}

/// Stable fingerprint over included turns; changes when settled content changes,
/// which is how the UI learns a snapshot is stale.
fn compute_source_fingerprint(sources: &[ConversationSource]) -> String {
    let canonical: Vec<Value> = sources
        .iter()
        .map(|s| {
            json!({
                "turnId": s.turn_id,
                "userText": s.user_text.trim(),
                "assistantText": s.assistant_text.trim(),
            })
        })
        .collect();
    let serialized = serde_json::to_string(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn format_translated_conversation(turns: &[ConversationTranslateTurnJson]) -> String {
    turns
        .iter()
        .map(|t| {
            format!(
                "## 轮次 {}\n\n**用户**\n\n{}\n\n**助手**\n\n{}",
                t.index, t.user_text_zh, t.assistant_text_zh
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

async fn translate_conversation(
    db: &GatewaySessionDb,
    sources: &[ConversationSource],
    target_language: &str,
) -> Result<Vec<ConversationTranslateTurnJson>, GatewayTranslateApiError> {
    let mut out = Vec::with_capacity(sources.len());
    for s in sources {
        let user_text_zh = translate_text_to_zh(db, &s.user_text, target_language).await?;
        let assistant_text_zh = if s.assistant_text.trim().is_empty() {
            "（无助手回复）".to_string()
        } else {
            translate_text_to_zh(db, &s.assistant_text, target_language).await?
        };
        out.push(ConversationTranslateTurnJson {
            index: s.index,
            turn_id: s.turn_id.clone(),
            user_text: s.user_text.clone(),
            assistant_text: s.assistant_text.clone(),
            user_text_zh,
            assistant_text_zh,
        });
    }
    Ok(out)
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
    /// `translating` | `ready` | `error`. Author: kejiqing
    pub status: String,
    /// Settled source changed since this snapshot was built.
    pub stale: bool,
    #[serde(rename = "error", skip_serializing_if = "Option::is_none")]
    pub error_text: Option<String>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct GetConversationTranslateResponse {
    pub snapshot: Option<ConversationTranslateSnapshotJson>,
}

#[derive(Debug, serde::Serialize)]
pub struct RebuildConversationTranslateResponse {
    pub ok: bool,
    /// Always `translating` on a fresh claim; poll GET for completion.
    pub status: String,
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
    // Recompute the current settled fingerprint only for a finished snapshot, so the
    // UI can surface a "stale" hint once more turns complete. Author: kejiqing
    let stale = if row.status == "ready" {
        match collect_terminal_turns(db, session_id, proj_id).await {
            Ok(sources) => compute_source_fingerprint(&sources) != row.source_fingerprint,
            Err(_) => false,
        }
    } else {
        false
    };
    Ok(Json(GetConversationTranslateResponse {
        snapshot: Some(ConversationTranslateSnapshotJson {
            source_fingerprint: row.source_fingerprint,
            turns,
            markdown: row.markdown,
            target_language: row.target_language,
            model_id: row.model_id,
            status: row.status,
            stale,
            error_text: row.error_text,
            updated_at_ms: row.updated_at_ms,
        }),
    }))
}

/// Trigger a backend-orchestrated rebuild of the whole-conversation translation.
/// Single-flight via the snapshot `status`; returns immediately and the heavy
/// LLM work runs in a spawned task. Author: kejiqing
pub async fn rebuild_conversation_translate_handler(
    db: Arc<GatewaySessionDb>,
    session_id: String,
    proj_id: i64,
) -> Result<Json<RebuildConversationTranslateResponse>, GatewayTranslateApiError> {
    if proj_id < 1 {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let exists = db.session_exists(&session_id, proj_id).await.map_err(|e| {
        GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    if !exists {
        return Err(GatewayTranslateApiError::new(
            StatusCode::NOT_FOUND,
            format!("session not found: {session_id} projId={proj_id}"),
        ));
    }

    let sources = collect_terminal_turns(db.as_ref(), &session_id, proj_id).await?;
    if sources.is_empty() {
        return Err(GatewayTranslateApiError::new(
            StatusCode::BAD_REQUEST,
            "no completed turns to translate yet",
        ));
    }
    let fingerprint = compute_source_fingerprint(&sources);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let claimed = db
        .begin_conversation_translate(
            &session_id,
            proj_id,
            &fingerprint,
            TARGET_LANGUAGE_ZH,
            now_ms,
            now_ms - STALE_TRANSLATE_LOCK_MS,
        )
        .await
        .map_err(|e| {
            GatewayTranslateApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    if !claimed {
        return Err(GatewayTranslateApiError::new(
            StatusCode::CONFLICT,
            "translation already in progress",
        ));
    }

    let db_bg = db.clone();
    tokio::spawn(async move {
        let result = translate_conversation(db_bg.as_ref(), &sources, TARGET_LANGUAGE_ZH).await;
        let done_ms = chrono::Utc::now().timestamp_millis();
        match result {
            Ok(turns) => {
                let markdown = format_translated_conversation(&turns);
                let turns_json =
                    serde_json::to_value(&turns).unwrap_or_else(|_| Value::Array(Vec::new()));
                if let Err(e) = db_bg
                    .complete_conversation_translate(
                        &session_id,
                        proj_id,
                        &fingerprint,
                        &turns_json,
                        &markdown,
                        TARGET_LANGUAGE_ZH,
                        None,
                        done_ms,
                    )
                    .await
                {
                    let _ = db_bg
                        .fail_conversation_translate(
                            &session_id,
                            proj_id,
                            &format!("persist failed: {e}"),
                            done_ms,
                        )
                        .await;
                }
            }
            Err(e) => {
                let _ = db_bg
                    .fail_conversation_translate(&session_id, proj_id, &e.message, done_ms)
                    .await;
            }
        }
    });

    Ok(Json(RebuildConversationTranslateResponse {
        ok: true,
        status: "translating".to_string(),
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

    #[test]
    fn mostly_chinese_detects_cjk_dominance() {
        assert!(mostly_chinese("这是一段中文说明文字"));
        assert!(!mostly_chinese("this is plain english text"));
        assert!(!mostly_chinese("   \n  "));
    }

    #[test]
    fn split_for_translation_bounds_each_chunk() {
        let long = "word ".repeat(2000); // ~10k chars
        let chunks = split_for_translation(&long, TRANSLATE_CHUNK_CHARS);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(
                c.chars().count() <= TRANSLATE_CHUNK_CHARS,
                "chunk over budget"
            );
        }
        let short = "just one chunk";
        assert_eq!(
            split_for_translation(short, TRANSLATE_CHUNK_CHARS),
            vec![short.to_string()]
        );
        assert!(split_for_translation("   ", TRANSLATE_CHUNK_CHARS).is_empty());
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive() {
        let a = vec![ConversationSource {
            index: 1,
            turn_id: "T_1".to_string(),
            user_text: "hi".to_string(),
            assistant_text: "ok".to_string(),
        }];
        let b = vec![ConversationSource {
            index: 1,
            turn_id: "T_1".to_string(),
            user_text: "hi".to_string(),
            assistant_text: "changed".to_string(),
        }];
        assert_eq!(
            compute_source_fingerprint(&a),
            compute_source_fingerprint(&a)
        );
        assert_ne!(
            compute_source_fingerprint(&a),
            compute_source_fingerprint(&b)
        );
    }
}
