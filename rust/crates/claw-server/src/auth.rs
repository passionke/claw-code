use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::db;
use crate::error::ServerError;
use crate::server::AppState;

const SESSION_COOKIE: &str = "claw_session";
const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 7;

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct UserDto {
    pub id: String,
    pub email: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let email = body.email.trim().to_ascii_lowercase();
    if email.is_empty() || body.password.len() < 8 {
        return Err(ServerError::BadRequest(
            "email required and password must be at least 8 characters".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let salt = SaltString::generate(&mut rand::thread_rng());
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| ServerError::Internal(e.to_string()))?
        .to_string();
    let now = db::now_ms();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at_ms) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&email)
    .bind(&hash)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(d) = &e {
            if d.is_unique_violation() {
                return ServerError::BadRequest("email already registered".into());
            }
        }
        ServerError::Db(e)
    })?;
    Ok((StatusCode::CREATED, Json(UserDto { id, email })))
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let email = body.email.trim().to_ascii_lowercase();
    let row = sqlx::query("SELECT id, email, password_hash FROM users WHERE email = ?")
        .bind(&email)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Err(ServerError::Unauthorized);
    };
    let user_id: String = row.try_get("id").map_err(|e| ServerError::Internal(e.to_string()))?;
    let db_email: String = row
        .try_get("email")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let ph: String = row
        .try_get("password_hash")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let parsed = PasswordHash::new(&ph).map_err(|e| ServerError::Internal(e.to_string()))?;
    Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .map_err(|_| ServerError::Unauthorized)?;

    let token = Uuid::new_v4().to_string();
    let exp = db::now_ms() + SESSION_TTL_SECS * 1000;
    sqlx::query("INSERT INTO auth_sessions (token, user_id, expires_at_ms) VALUES (?, ?, ?)")
        .bind(&token)
        .bind(&user_id)
        .bind(exp)
        .execute(&state.pool)
        .await?;

    let cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; Path=/; Max-Age={}; SameSite=Lax",
        SESSION_TTL_SECS
    );
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(UserDto {
            id: user_id,
            email: db_email,
        }),
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(token) = session_token_from_headers(&headers) {
        sqlx::query("DELETE FROM auth_sessions WHERE token = ?")
            .bind(token)
            .execute(&state.pool)
            .await?;
    }
    let cookie = format!("{SESSION_COOKIE}=; HttpOnly; Path=/; Max-Age=0");
    Ok(([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT))
}

pub async fn me(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<UserDto>, ServerError> {
    let user = require_user(&state, &headers).await?;
    Ok(Json(user))
}

pub fn session_token_from_headers(headers: &axum::http::HeaderMap) -> Option<&str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(SESSION_COOKIE) {
            let rest = rest.trim_start_matches('=').trim();
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

pub async fn require_user(state: &AppState, headers: &axum::http::HeaderMap) -> Result<UserDto, ServerError> {
    let Some(token) = session_token_from_headers(headers) else {
        return Err(ServerError::Unauthorized);
    };
    let now = db::now_ms();
    let row = sqlx::query(
        "SELECT u.id as id, u.email as email FROM auth_sessions s \
         JOIN users u ON u.id = s.user_id \
         WHERE s.token = ? AND s.expires_at_ms > ?",
    )
    .bind(token)
    .bind(now)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ServerError::Unauthorized);
    };
    Ok(UserDto {
        id: row.try_get("id").map_err(|e| ServerError::Internal(e.to_string()))?,
        email: row
            .try_get("email")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
    })
}
