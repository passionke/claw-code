//! Admin auth / accounts / me-mcp-tokens HTTP routes. Author: kejiqing

use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::admin_auth::{
    create_account, delete_project_member, list_accounts, login, patch_account,
    require_members_manager, require_principal, require_system_admin, require_system_admin_or_open,
    resolve_optional_principal, revoke_session_by_token, upsert_project_member, AccountPublic,
};
use crate::api_error::ApiError;
use crate::app_state::AppState;
use crate::gateway_admin_mcp_token::{
    self, admin_mcp_tokens_public_for_account, extract_bearer_token, IssueAdminMcpTokenInput,
    IssueAdminMcpTokenResponse,
};
use crate::gateway_global_settings::get_gateway_global_settings;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/admin/auth/login", post(login_handler))
        .route("/v1/admin/auth/logout", post(logout_handler))
        .route("/v1/admin/auth/me", get(me_handler))
        .route(
            "/v1/admin/accounts",
            get(list_accounts_handler).post(create_account_handler),
        )
        .route(
            "/v1/admin/accounts/{account_id}",
            patch(patch_account_handler),
        )
        .route(
            "/v1/admin/accounts/{account_id}/projects/{proj_id}",
            put(put_member_handler).delete(delete_member_handler),
        )
        .route(
            "/v1/admin/me/mcp-tokens",
            get(list_my_mcp_tokens_handler).post(issue_my_mcp_token_handler),
        )
        .route(
            "/v1/admin/me/mcp-tokens/{token_id}",
            delete(revoke_my_mcp_token_handler),
        )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub username: String,
    #[serde(rename = "systemRole")]
    pub system_role: String,
    #[serde(rename = "projectIds")]
    pub project_ids: Vec<i64>,
    pub token: String,
    #[serde(rename = "expiresAtMs")]
    pub expires_at_ms: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MeResponse {
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub username: String,
    #[serde(rename = "systemRole")]
    pub system_role: String,
    #[serde(rename = "projectIds")]
    pub project_ids: Vec<i64>,
    #[serde(rename = "systemAdmin")]
    pub system_admin: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateAccountBody {
    pub username: String,
    pub password: String,
    #[serde(default, rename = "systemRole")]
    pub system_role: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PatchAccountBody {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default, rename = "systemRole")]
    pub system_role: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/v1/admin/auth/login",
    tag = "Admin Auth",
    operation_id = "login_handler",
    request_body = LoginBody,
    responses((status = 200, description = "Logged in", body = LoginResponse))
)]
pub(crate) async fn login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, ApiError> {
    let (principal, token, expires_at_ms) =
        login(&state.session_db, &body.username, &body.password)
            .await
            .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e))?;
    Ok(Json(LoginResponse {
        account_id: principal.account_id,
        username: principal.username,
        system_role: if principal.system_admin {
            "system_admin".into()
        } else {
            "none".into()
        },
        project_ids: principal.project_ids,
        token,
        expires_at_ms,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/admin/auth/logout",
    tag = "Admin Auth",
    operation_id = "logout_handler",
    responses((status = 200, description = "Logged out"))
)]
pub(crate) async fn logout_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Some(tok) = extract_bearer_token(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    ) {
        let _ = revoke_session_by_token(&state.session_db, &tok).await;
    }
    Ok(Json(json!({ "ok": true })))
}

#[utoipa::path(
    get,
    path = "/v1/admin/auth/me",
    tag = "Admin Auth",
    operation_id = "me_handler",
    responses((status = 200, description = "Current account", body = MeResponse))
)]
pub(crate) async fn me_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    Ok(Json(MeResponse {
        account_id: p.account_id,
        username: p.username,
        system_role: if p.system_admin {
            "system_admin".into()
        } else {
            "none".into()
        },
        project_ids: p.project_ids,
        system_admin: p.system_admin,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/admin/accounts",
    tag = "Admin Auth",
    operation_id = "list_accounts_handler",
    responses((status = 200, description = "Account list"))
)]
pub(crate) async fn list_accounts_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    require_members_manager(&p)?;
    let accounts = list_accounts(&state.session_db)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "accounts": accounts })))
}

#[utoipa::path(
    post,
    path = "/v1/admin/accounts",
    tag = "Admin Auth",
    operation_id = "create_account_handler",
    request_body = CreateAccountBody,
    responses((status = 201, description = "Account created", body = AccountPublic))
)]
pub(crate) async fn create_account_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateAccountBody>,
) -> Result<(StatusCode, Json<AccountPublic>), ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    require_members_manager(&p)?;
    let role = body.system_role.as_deref().unwrap_or("none");
    let acc = create_account(&state.session_db, &body.username, &body.password, role)
        .await
        .map_err(|e| {
            let status = if e.contains("already exists")
                || e.contains("required")
                || e.contains("must be")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ApiError::new(status, e)
        })?;
    Ok((StatusCode::CREATED, Json(acc)))
}

#[utoipa::path(
    patch,
    path = "/v1/admin/accounts/{account_id}",
    tag = "Admin Auth",
    operation_id = "patch_account_handler",
    params(("account_id" = String, Path, description = "Account id")),
    request_body = PatchAccountBody,
    responses((status = 200, description = "Account updated", body = AccountPublic))
)]
pub(crate) async fn patch_account_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(account_id): AxumPath<String>,
    Json(body): Json<PatchAccountBody>,
) -> Result<Json<AccountPublic>, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    require_members_manager(&p)?;
    let acc = patch_account(
        &state.session_db,
        &account_id,
        body.password.as_deref(),
        body.system_role.as_deref(),
        body.disabled,
    )
    .await
    .map_err(|e| {
        let status = if e.contains("not found") {
            StatusCode::NOT_FOUND
        } else if e.contains("must be") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ApiError::new(status, e)
    })?;
    Ok(Json(acc))
}

#[utoipa::path(
    put,
    path = "/v1/admin/accounts/{account_id}/projects/{proj_id}",
    tag = "Admin Auth",
    operation_id = "put_member_handler",
    params(
        ("account_id" = String, Path, description = "Account id"),
        ("proj_id" = i64, Path, description = "Project id")
    ),
    responses((status = 200, description = "Membership upserted"))
)]
pub(crate) async fn put_member_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((account_id, proj_id)): AxumPath<(String, i64)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    require_members_manager(&p)?;
    upsert_project_member(&state.session_db, &account_id, proj_id)
        .await
        .map_err(|e| {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            ApiError::new(status, e)
        })?;
    Ok(Json(
        json!({ "ok": true, "accountId": account_id, "projId": proj_id }),
    ))
}

#[utoipa::path(
    delete,
    path = "/v1/admin/accounts/{account_id}/projects/{proj_id}",
    tag = "Admin Auth",
    operation_id = "delete_member_handler",
    params(
        ("account_id" = String, Path, description = "Account id"),
        ("proj_id" = i64, Path, description = "Project id")
    ),
    responses((status = 204, description = "Membership removed"))
)]
pub(crate) async fn delete_member_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((account_id, proj_id)): AxumPath<(String, i64)>,
) -> Result<StatusCode, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    require_members_manager(&p)?;
    let deleted = delete_project_member(&state.session_db, &account_id, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "membership not found"))
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/me/mcp-tokens",
    tag = "Admin Auth",
    operation_id = "list_my_mcp_tokens_handler",
    responses((status = 200, description = "Own admin MCP tokens"))
)]
pub(crate) async fn list_my_mcp_tokens_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    if p.account_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "legacy token cannot list me-tokens",
        ));
    }
    let (settings, _, _) = get_gateway_global_settings(&state.session_db)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let tokens = admin_mcp_tokens_public_for_account(&settings, &p.account_id);
    Ok(Json(json!({ "tokens": tokens })))
}

#[utoipa::path(
    post,
    path = "/v1/admin/me/mcp-tokens",
    tag = "Admin Auth",
    operation_id = "issue_my_mcp_token_handler",
    request_body = IssueAdminMcpTokenInput,
    responses((status = 200, description = "Token issued", body = IssueAdminMcpTokenResponse))
)]
pub(crate) async fn issue_my_mcp_token_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut req): Json<IssueAdminMcpTokenInput>,
) -> Result<Json<IssueAdminMcpTokenResponse>, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    if p.account_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "legacy token cannot issue me-tokens",
        ));
    }
    req.account_id = Some(p.account_id.clone());
    let body = gateway_admin_mcp_token::issue_admin_mcp_token(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    delete,
    path = "/v1/admin/me/mcp-tokens/{token_id}",
    tag = "Admin Auth",
    operation_id = "revoke_my_mcp_token_handler",
    params(("token_id" = String, Path, description = "Admin MCP token id")),
    responses((status = 204, description = "Token revoked"))
)]
pub(crate) async fn revoke_my_mcp_token_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let p = require_principal(&state.session_db, &headers).await?;
    if p.account_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "legacy token cannot revoke me-tokens",
        ));
    }
    let revoked = gateway_admin_mcp_token::revoke_admin_mcp_token_for_account(
        &state.session_db,
        &token_id,
        &p.account_id,
    )
    .await
    .map_err(|e| {
        if e.starts_with("forbidden") {
            ApiError::new(StatusCode::FORBIDDEN, e)
        } else {
            ApiError::new(StatusCode::BAD_REQUEST, e)
        }
    })?;
    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "admin MCP token not found",
        ))
    }
}

/// Used by global admin-mcp-tokens issue: require system_admin when auth present;
/// bind accountId to caller when available. Author: kejiqing
pub(crate) async fn guard_global_issue_admin_mcp(
    state: &AppState,
    headers: &HeaderMap,
    req: &mut IssueAdminMcpTokenInput,
) -> Result<(), ApiError> {
    let p = require_system_admin_or_open(&state.session_db, headers).await?;
    if let Some(p) = p {
        if req.account_id.as_deref().unwrap_or("").is_empty() && !p.account_id.is_empty() {
            req.account_id = Some(p.account_id.clone());
        }
    }
    Ok(())
}

pub(crate) async fn guard_global_revoke_admin_mcp(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let _ = require_system_admin_or_open(&state.session_db, headers).await?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn optional_principal_for_list(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<crate::admin_auth::AuthPrincipal>, ApiError> {
    resolve_optional_principal(&state.session_db, headers).await
}

#[allow(dead_code)]
pub(crate) fn ensure_system_admin(p: &crate::admin_auth::AuthPrincipal) -> Result<(), ApiError> {
    require_system_admin(p)
}
