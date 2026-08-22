//! OpenAI Chat Completions + Responses compatibility routes. Author: kejiqing
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream;
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::agent_completion::{
    chat_completion_response, chat_completion_stream_chunks, extract_solve_message,
    normalize_chat_completions, normalize_responses, openai_error, responses_api_response,
    AgentCompletionRequest, ChatCompletionsRequest, OpenAiErrorBody, ResponsesRequest,
};
use crate::api_error::ApiError;
use crate::app_state::{AppState, SolveRequest};
use crate::gateway_admin_mcp_token::extract_bearer_token;
use crate::project_model_api_key::ProjectModelApiKeyRow;
use crate::routes::app::{admin_mcp_run_solve_sync, validate_solve_request};
use crate::session_merge;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route(
            "/v1/projects/{proj_id}/model-api-keys",
            get(list_model_api_keys).post(issue_model_api_key),
        )
        .route(
            "/v1/projects/{proj_id}/model-api-keys/{token_id}",
            axum::routing::delete(revoke_model_api_key),
        )
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn openai_err_response(status: StatusCode, body: OpenAiErrorBody) -> Response {
    (status, Json(body)).into_response()
}

async fn require_project_model_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ProjectModelApiKeyRow, Response> {
    let token = extract_bearer_token(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    )
    .ok_or_else(|| {
        openai_err_response(
            StatusCode::UNAUTHORIZED,
            openai_error(
                "Missing Bearer project model API key",
                "invalid_api_key",
                "invalid_request_error",
            ),
        )
    })?;
    state
        .session_db
        .verify_project_model_api_key(&token)
        .await
        .map_err(|e| {
            openai_err_response(
                StatusCode::UNAUTHORIZED,
                openai_error(e, "invalid_api_key", "invalid_request_error"),
            )
        })
}

/// Resolve session from conversation key / previous_response_id. Author: kejiqing
async fn resolve_session(
    state: &AppState,
    key: &ProjectModelApiKeyRow,
    mut req: AgentCompletionRequest,
) -> Result<AgentCompletionRequest, Response> {
    req.proj_id = key.proj_id;
    let allowed = req.model_alias == key.model_alias
        || req.model_alias == format!("proj-{}", key.proj_id)
        || (key.model_alias == "agent" && req.model_alias == "agent");
    if !allowed {
        return Err(openai_err_response(
            StatusCode::BAD_REQUEST,
            openai_error(
                format!(
                    "model {:?} is not bound to this API key (expected {:?})",
                    req.model_alias, key.model_alias
                ),
                "model_not_found",
                "invalid_request_error",
            ),
        ));
    }

    if let Some(prev) = req.previous_response_id.clone() {
        let mapped = state
            .session_db
            .get_openai_response(&prev)
            .await
            .map_err(|e| {
                openai_err_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    openai_error(e, "server_error", "server_error"),
                )
            })?;
        let Some((api_key_id, proj_id, session_id, _)) = mapped else {
            return Err(openai_err_response(
                StatusCode::BAD_REQUEST,
                openai_error(
                    "previous_response_id not found",
                    "invalid_request",
                    "invalid_request_error",
                ),
            ));
        };
        if api_key_id != key.id || proj_id != key.proj_id {
            return Err(openai_err_response(
                StatusCode::FORBIDDEN,
                openai_error(
                    "previous_response_id does not belong to this API key",
                    "permission_denied",
                    "invalid_request_error",
                ),
            ));
        }
        req.session_id = Some(session_id);
        return Ok(req);
    }

    if let Some(conv) = req.conversation_key.clone() {
        if let Some((proj_id, session_id)) = state
            .session_db
            .get_openai_conversation_session(&key.id, &conv)
            .await
            .map_err(|e| {
                openai_err_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    openai_error(e, "server_error", "server_error"),
                )
            })?
        {
            if proj_id != key.proj_id {
                return Err(openai_err_response(
                    StatusCode::FORBIDDEN,
                    openai_error(
                        "conversation belongs to another project",
                        "permission_denied",
                        "invalid_request_error",
                    ),
                ));
            }
            // Only reuse if session still exists for this proj.
            if state
                .session_db
                .get_session_home_rel(&session_id, proj_id)
                .await
                .ok()
                .flatten()
                .is_some()
            {
                req.session_id = Some(session_id);
            }
        }
    }

    Ok(req)
}

async fn run_agent_completion(
    state: &AppState,
    key: &ProjectModelApiKeyRow,
    req: AgentCompletionRequest,
) -> Result<(String, String, String), Response> {
    let session_hint = req.session_id.clone();
    let solve_req = SolveRequest {
        proj_id: key.proj_id,
        user_prompt: req.user_prompt,
        session_id: session_hint.clone(),
        model: None,
        timeout_seconds: req.timeout_seconds,
        extra_session: req.extra_session,
        allowed_tools: None,
        max_iterations: None,
        attachments: None,
    };
    validate_solve_request(&state.session_db, &solve_req)
        .await
        .map_err(|e: ApiError| {
            openai_err_response(
                e.status,
                openai_error(e.message, "invalid_request", "invalid_request_error"),
            )
        })?;

    // For first turn without prior session, omit sessionId so gateway mints one.
    let mut solve_req = solve_req;
    if session_hint.is_none() {
        solve_req.session_id = None;
    }

    let result = admin_mcp_run_solve_sync(state, solve_req)
        .await
        .map_err(|e: ApiError| {
            openai_err_response(
                e.status,
                openai_error(e.message, "server_error", "server_error"),
            )
        })?;

    let content = extract_solve_message(result.output_json.as_ref(), &result.output_text);
    let session_id = result.session_id.clone();
    let turn_id = result.turn_id.clone();

    if let Some(conv) = req.conversation_key.as_ref() {
        let _ = state
            .session_db
            .upsert_openai_conversation(&key.id, key.proj_id, conv, &session_id)
            .await;
    }
    let _ = state
        .session_db
        .insert_openai_response(&turn_id, &key.id, key.proj_id, &session_id, &turn_id)
        .await;

    Ok((session_id, turn_id, content))
}

fn with_session_header(mut resp: Response, session_id: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(session_id) {
        resp.headers_mut()
            .insert(header::HeaderName::from_static("x-nerogate-session-id"), v);
    }
    resp
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChatCompletionsRequest>,
) -> Response {
    let key = match require_project_model_key(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let (norm, model) = match normalize_chat_completions(&body) {
        Ok(v) => v,
        Err(e) => return openai_err_response(StatusCode::BAD_REQUEST, e),
    };
    let norm = match resolve_session(&state, &key, norm).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let stream = norm.stream;
    let (session_id, turn_id, content) = match run_agent_completion(&state, &key, norm).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if stream {
        let chunks =
            chat_completion_stream_chunks(&model, &turn_id, &session_id, &content, now_secs());
        let events = chunks.into_iter().map(|c| {
            // strip "data: " prefix for Event::data
            let data = c
                .strip_prefix("data: ")
                .unwrap_or(&c)
                .trim_end_matches('\n')
                .to_string();
            Ok::<Event, Infallible>(Event::default().data(data))
        });
        let mut resp = Sse::new(stream::iter(events))
            .keep_alive(KeepAlive::default())
            .into_response();
        resp.headers_mut().insert(
            header::HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no"),
        );
        return with_session_header(resp, &session_id);
    }
    let body = chat_completion_response(&model, &turn_id, &session_id, &content, now_secs());
    with_session_header(Json(body).into_response(), &session_id)
}

async fn responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ResponsesRequest>,
) -> Response {
    let key = match require_project_model_key(&state, &headers).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let (norm, model) = match normalize_responses(&body) {
        Ok(v) => v,
        Err(e) => return openai_err_response(StatusCode::BAD_REQUEST, e),
    };
    let norm = match resolve_session(&state, &key, norm).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let stream = norm.stream;
    let (session_id, turn_id, content) = match run_agent_completion(&state, &key, norm).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if stream {
        let created = responses_api_response(&model, &turn_id, &session_id, &content, now_ms());
        let events = vec![
            Ok::<Event, Infallible>(
                Event::default()
                    .event("response.completed")
                    .data(created.to_string()),
            ),
            Ok(Event::default().event("done").data("[DONE]")),
        ];
        let resp = Sse::new(stream::iter(events))
            .keep_alive(KeepAlive::default())
            .into_response();
        return with_session_header(resp, &session_id);
    }
    let body = responses_api_response(&model, &turn_id, &session_id, &content, now_ms());
    with_session_header(Json(body).into_response(), &session_id)
}

#[derive(Debug, Deserialize)]
struct IssueKeyBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default, rename = "modelAlias")]
    model_alias: Option<String>,
}

async fn require_admin_or_open(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    // Prefer admin MCP token when provided; otherwise allow trusted internal network (existing solve pattern).
    if let Some(tok) = extract_bearer_token(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    ) {
        if tok.starts_with("camt_") {
            crate::gateway_admin_mcp_token::verify_admin_mcp_token(&state.session_db, &tok)
                .await
                .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e))?;
        }
    }
    Ok(())
}

async fn list_model_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(proj_id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_or_open(&state, &headers).await?;
    let list = state
        .session_db
        .list_project_model_api_keys(proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "keys": list })))
}

async fn issue_model_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(proj_id): axum::extract::Path<i64>,
    Json(body): Json<IssueKeyBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_or_open(&state, &headers).await?;
    if state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("project_config not found for projId={proj_id}"),
        ));
    }
    let issued = state
        .session_db
        .issue_project_model_api_key(
            proj_id,
            body.model_alias.as_deref().unwrap_or("agent"),
            body.name.as_deref().unwrap_or("default"),
            body.note.as_deref().unwrap_or(""),
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(issued))
}

async fn revoke_model_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path((_proj_id, token_id)): axum::extract::Path<(i64, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_or_open(&state, &headers).await?;
    let ok = state
        .session_db
        .revoke_project_model_api_key(&token_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !ok {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "token not found"));
    }
    Ok(Json(json!({ "revoked": true, "id": token_id })))
}

// Silence unused import warnings in some build configs.
#[allow(dead_code)]
fn _unused() {
    let _ = Uuid::new_v4();
    let _ = session_merge::trim_session_id(None);
}
