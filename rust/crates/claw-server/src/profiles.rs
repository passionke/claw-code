use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::require_user;
use crate::crypto;
use crate::db;
use crate::error::ServerError;
use crate::server::AppState;

#[derive(Serialize)]
pub struct ProviderProfileDto {
    pub id: String,
    pub name: String,
    pub provider_kind: String,
    pub base_url: Option<String>,
    pub model: String,
    pub created_at_ms: i64,
}

#[derive(Deserialize)]
pub struct CreateProfileRequest {
    pub name: String,
    /// `anthropic` | `openai_compat` | `dashscope` | `xai`
    pub provider_kind: String,
    pub base_url: Option<String>,
    pub model: String,
    pub api_key: String,
}

pub async fn list_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProviderProfileDto>>, ServerError> {
    let user = require_user(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, name, provider_kind, base_url, model, created_at_ms FROM provider_profiles WHERE user_id = ? ORDER BY created_at_ms DESC",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(ProviderProfileDto {
            id: row.try_get("id").map_err(|e| ServerError::Internal(e.to_string()))?,
            name: row.try_get("name").map_err(|e| ServerError::Internal(e.to_string()))?,
            provider_kind: row
                .try_get("provider_kind")
                .map_err(|e| ServerError::Internal(e.to_string()))?,
            base_url: row
                .try_get::<Option<String>>("base_url")
                .map_err(|e| ServerError::Internal(e.to_string()))?,
            model: row.try_get("model").map_err(|e| ServerError::Internal(e.to_string()))?,
            created_at_ms: row
                .try_get("created_at_ms")
                .map_err(|e| ServerError::Internal(e.to_string()))?,
        });
    }
    Ok(Json(out))
}

pub async fn create_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateProfileRequest>,
) -> Result<Json<ProviderProfileDto>, ServerError> {
    let user = require_user(&state, &headers).await?;
    let name = body.name.trim();
    if name.is_empty() || body.model.trim().is_empty() || body.api_key.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "name, model, and api_key required".into(),
        ));
    }
    let kind = body.provider_kind.trim().to_ascii_lowercase();
    if !matches!(
        kind.as_str(),
        "anthropic" | "openai_compat" | "dashscope" | "xai"
    ) {
        return Err(ServerError::BadRequest(
            "provider_kind must be anthropic, openai_compat, dashscope, or xai".into(),
        ));
    }
    if kind != "anthropic"
        && body
            .base_url
            .as_ref()
            .map_or(true, |s| s.trim().is_empty())
    {
        return Err(ServerError::BadRequest(
            "base_url required for non-anthropic providers".into(),
        ));
    }
    let ciphertext =
        crypto::encrypt_secret(&state.master_key, body.api_key.trim()).map_err(|e| e)?;
    let id = Uuid::new_v4().to_string();
    let now = db::now_ms();
    sqlx::query(
        "INSERT INTO provider_profiles (id, user_id, name, provider_kind, base_url, model, api_key_ciphertext, created_at_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(name)
    .bind(&kind)
    .bind(body.base_url.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()))
    .bind(body.model.trim())
    .bind(&ciphertext)
    .bind(now)
    .execute(&state.pool)
    .await?;
    Ok(Json(ProviderProfileDto {
        id,
        name: name.to_string(),
        provider_kind: kind,
        base_url: body.base_url,
        model: body.model.trim().to_string(),
        created_at_ms: now,
    }))
}

pub async fn delete_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Result<StatusCode, ServerError> {
    let user = require_user(&state, &headers).await?;
    let res = sqlx::query("DELETE FROM provider_profiles WHERE id = ? AND user_id = ?")
        .bind(&id)
        .bind(&user.id)
        .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ServerError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Row for internal chat wiring (includes ciphertext).
#[derive(Clone)]
pub struct ProviderProfileRow {
    pub id: String,
    pub provider_kind: String,
    pub base_url: Option<String>,
    pub model: String,
    pub api_key_ciphertext: String,
}

pub async fn load_profile(
    state: &AppState,
    user_id: &str,
    profile_id: &str,
) -> Result<ProviderProfileRow, ServerError> {
    let row = sqlx::query(
        "SELECT id, provider_kind, base_url, model, api_key_ciphertext FROM provider_profiles WHERE id = ? AND user_id = ?",
    )
    .bind(profile_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ServerError::NotFound);
    };
    Ok(ProviderProfileRow {
        id: row.try_get("id").map_err(|e| ServerError::Internal(e.to_string()))?,
        provider_kind: row
            .try_get("provider_kind")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        base_url: row.try_get("base_url").ok(),
        model: row.try_get("model").map_err(|e| ServerError::Internal(e.to_string()))?,
        api_key_ciphertext: row
            .try_get("api_key_ciphertext")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
    })
}
