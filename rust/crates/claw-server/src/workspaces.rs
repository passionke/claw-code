use std::path::{Path, PathBuf};

use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::{require_user, UserDto};
use crate::db;
use crate::error::ServerError;
use crate::server::AppState;

#[derive(Serialize)]
pub struct WorkspaceDto {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub created_at_ms: i64,
}

#[derive(Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

fn user_workspace_cap(user_id: &str) -> PathBuf {
    PathBuf::from("users").join(user_id).join("ws")
}

/// Ensures `path` is under `state.data_dir/users/<user_id>/`.
pub fn validate_user_path(state: &AppState, user_id: &str, path: &Path) -> Result<PathBuf, ServerError> {
    let base = state.data_dir.join(user_workspace_cap(user_id));
    let base = std::fs::canonicalize(&base).unwrap_or(base);
    let resolved = path.canonicalize().map_err(|_| {
        ServerError::BadRequest("workspace path does not exist or is inaccessible".into())
    })?;
    if !resolved.starts_with(&base) {
        return Err(ServerError::Forbidden);
    }
    Ok(resolved)
}

pub async fn list_workspaces(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<WorkspaceDto>>, ServerError> {
    let user = require_user(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, name, root_path, created_at_ms FROM workspaces WHERE user_id = ? ORDER BY created_at_ms DESC",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(WorkspaceDto {
            id: row.try_get("id").map_err(|e| ServerError::Internal(e.to_string()))?,
            name: row.try_get("name").map_err(|e| ServerError::Internal(e.to_string()))?,
            root_path: row
                .try_get("root_path")
                .map_err(|e| ServerError::Internal(e.to_string()))?,
            created_at_ms: row
                .try_get("created_at_ms")
                .map_err(|e| ServerError::Internal(e.to_string()))?,
        });
    }
    Ok(Json(out))
}

pub async fn create_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateWorkspaceRequest>,
) -> Result<Json<WorkspaceDto>, ServerError> {
    let user = require_user(&state, &headers).await?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ServerError::BadRequest("name required".into()));
    }
    let id = Uuid::new_v4().to_string();
    let rel = user_workspace_cap(&user.id).join(&id);
    let root = state.data_dir.join(&rel);
    std::fs::create_dir_all(&root).map_err(|e| ServerError::Internal(e.to_string()))?;
    let root_path = root
        .canonicalize()
        .map_err(|e| ServerError::Internal(e.to_string()))?
        .to_string_lossy()
        .into_owned();
    let now = db::now_ms();
    sqlx::query(
        "INSERT INTO workspaces (id, user_id, name, root_path, created_at_ms) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(name)
    .bind(&root_path)
    .bind(now)
    .execute(&state.pool)
    .await?;
    Ok(Json(WorkspaceDto {
        id,
        name: name.to_string(),
        root_path,
        created_at_ms: now,
    }))
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Result<StatusCode, ServerError> {
    let user = require_user(&state, &headers).await?;
    let res = sqlx::query("DELETE FROM workspaces WHERE id = ? AND user_id = ?")
        .bind(&id)
        .bind(&user.id)
        .execute(&state.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ServerError::NotFound);
    }
    let _ = std::fs::remove_dir_all(
        state
            .data_dir
            .join(user_workspace_cap(&user.id))
            .join(&id),
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Load workspace for `user_id`; returns row data and validated root path.
pub async fn load_workspace_for_user(
    state: &AppState,
    user: &UserDto,
    workspace_id: &str,
) -> Result<(WorkspaceDto, PathBuf), ServerError> {
    let row = sqlx::query(
        "SELECT id, name, root_path, created_at_ms FROM workspaces WHERE id = ? AND user_id = ?",
    )
    .bind(workspace_id)
    .bind(&user.id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ServerError::NotFound);
    };
    let dto = WorkspaceDto {
        id: row.try_get("id").map_err(|e| ServerError::Internal(e.to_string()))?,
        name: row.try_get("name").map_err(|e| ServerError::Internal(e.to_string()))?,
        root_path: row
            .try_get("root_path")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
    };
    let path = validate_user_path(state, &user.id, Path::new(&dto.root_path))?;
    Ok((dto, path))
}
