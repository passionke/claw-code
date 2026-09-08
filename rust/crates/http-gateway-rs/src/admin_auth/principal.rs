//! Resolve AuthPrincipal from Authorization header. Author: kejiqing

use axum::http::{header, HeaderMap, StatusCode};

use crate::api_error::ApiError;
use crate::gateway_admin_mcp_token::{
    extract_bearer_token, verify_admin_mcp_token, TOKEN_PREFIX as CAMT_PREFIX,
};
use crate::session_db::GatewaySessionDb;

use super::acl::{
    can_access_project, can_create_or_delete_project, can_manage_global, can_manage_members,
    AuthPrincipal,
};
use super::store::{
    get_account_by_id, principal_from_account, resolve_session_principal, SESSION_TOKEN_PREFIX,
};

/// Resolve optional principal from Bearer (cass_ or camt_). Author: kejiqing
pub(crate) async fn resolve_optional_principal(
    db: &GatewaySessionDb,
    headers: &HeaderMap,
) -> Result<Option<AuthPrincipal>, ApiError> {
    let Some(tok) = extract_bearer_token(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    ) else {
        return Ok(None);
    };
    if tok.starts_with(SESSION_TOKEN_PREFIX) {
        let p = resolve_session_principal(db, &tok)
            .await
            .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e))?;
        return Ok(Some(p));
    }
    if tok.starts_with(CAMT_PREFIX) {
        let entry = verify_admin_mcp_token(db, &tok)
            .await
            .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e))?;
        if let Some(account_id) = entry.account_id.as_deref().filter(|s| !s.is_empty()) {
            let Some(acc) = get_account_by_id(db, account_id)
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?
            else {
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "admin MCP token account not found",
                ));
            };
            let p = principal_from_account(db, &acc)
                .await
                .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e))?;
            return Ok(Some(p));
        }
        // Transition: unbound camt_ → system_admin scope.
        return Ok(Some(AuthPrincipal::legacy_camt_system_admin()));
    }
    Ok(None)
}

pub(crate) async fn require_principal(
    db: &GatewaySessionDb,
    headers: &HeaderMap,
) -> Result<AuthPrincipal, ApiError> {
    resolve_optional_principal(db, headers)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "authentication required"))
}

/// When principal present: must be system_admin. When absent: allow (trusted internal). Author: kejiqing
pub(crate) async fn require_system_admin_or_open(
    db: &GatewaySessionDb,
    headers: &HeaderMap,
) -> Result<Option<AuthPrincipal>, ApiError> {
    match resolve_optional_principal(db, headers).await? {
        None => Ok(None),
        Some(p) if can_manage_global(&p) => Ok(Some(p)),
        Some(_) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "system_admin required",
        )),
    }
}

/// When principal present: must access proj. When absent: allow. Author: kejiqing
pub(crate) async fn require_project_access_or_open(
    db: &GatewaySessionDb,
    headers: &HeaderMap,
    proj_id: i64,
) -> Result<Option<AuthPrincipal>, ApiError> {
    match resolve_optional_principal(db, headers).await? {
        None => Ok(None),
        Some(p) if can_access_project(&p, proj_id) => Ok(Some(p)),
        Some(_) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("no access to projId={proj_id}"),
        )),
    }
}

pub(crate) async fn require_create_project_or_open(
    db: &GatewaySessionDb,
    headers: &HeaderMap,
) -> Result<Option<AuthPrincipal>, ApiError> {
    match resolve_optional_principal(db, headers).await? {
        None => Ok(None),
        Some(p) if can_create_or_delete_project(&p) => Ok(Some(p)),
        Some(_) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "system_admin required to create or delete projects",
        )),
    }
}

pub(crate) fn require_members_manager(p: &AuthPrincipal) -> Result<(), ApiError> {
    if can_manage_members(p) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "system_admin required to manage members",
        ))
    }
}

pub(crate) fn require_system_admin(p: &AuthPrincipal) -> Result<(), ApiError> {
    if can_manage_global(p) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "system_admin required",
        ))
    }
}
