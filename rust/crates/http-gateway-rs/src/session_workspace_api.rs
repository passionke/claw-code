//! Read-only workspace tree/file API for interactive `/coding` sidebar. Author: kejiqing

use std::path::{Component, Path, PathBuf};

use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::pool::session_home_under_work_root;
use crate::session_terminal_api::{TerminalApiError, TerminalProjQuery};

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 2_000;

#[derive(Clone)]
pub struct WorkspaceApiContext {
    pub work_root: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTreeEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTreeResponse {
    pub session_id: String,
    pub proj_id: i64,
    pub entries: Vec<WorkspaceTreeEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileQuery {
    pub proj_id: i64,
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMediaQuery {
    pub proj_id: i64,
    pub path: String,
}

const MAX_MEDIA_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileResponse {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

fn safe_rel_path(raw: &str) -> Result<String, TerminalApiError> {
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let p = Path::new(trimmed);
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            Component::CurDir => {}
            _ => {
                return Err(TerminalApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid path",
                ));
            }
        }
    }
    Ok(trimmed.replace('\\', "/"))
}

pub async fn workspace_tree(
    ctx: WorkspaceApiContext,
    session_id: String,
    q: TerminalProjQuery,
) -> Result<Json<WorkspaceTreeResponse>, TerminalApiError> {
    let root = session_home_under_work_root(&ctx.work_root, q.proj_id, &session_id);
    if !root.is_dir() {
        return Ok(Json(WorkspaceTreeResponse {
            session_id,
            proj_id: q.proj_id,
            entries: vec![],
        }));
    }
    let mut entries = Vec::new();
    collect_tree(&root, &root, &mut entries, 0)?;
    Ok(Json(WorkspaceTreeResponse {
        session_id,
        proj_id: q.proj_id,
        entries,
    }))
}

fn collect_tree(
    root: &Path,
    dir: &Path,
    out: &mut Vec<WorkspaceTreeEntry>,
    depth: usize,
) -> Result<(), TerminalApiError> {
    if out.len() >= MAX_TREE_ENTRIES {
        return Ok(());
    }
    if depth > 12 {
        return Ok(());
    }
    let mut items: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .filter_map(Result::ok)
        .collect();
    items.sort_by_key(|e| e.file_name());
    for ent in items {
        if out.len() >= MAX_TREE_ENTRIES {
            break;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == ".git" || name == "target" {
            continue;
        }
        let path = ent.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let is_dir = path.is_dir();
        out.push(WorkspaceTreeEntry {
            name,
            path: rel,
            is_dir,
        });
        if is_dir {
            collect_tree(root, &path, out, depth + 1)?;
        }
    }
    Ok(())
}

pub async fn workspace_file(
    ctx: WorkspaceApiContext,
    session_id: String,
    q: WorkspaceFileQuery,
) -> Result<Json<WorkspaceFileResponse>, TerminalApiError> {
    let rel = safe_rel_path(&q.path)?;
    let root = session_home_under_work_root(&ctx.work_root, q.proj_id, &session_id);
    let full = if rel.is_empty() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "path required",
        ));
    } else {
        root.join(&rel)
    };
    if !full.starts_with(&root) {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "path escapes session root",
        ));
    }
    let meta = std::fs::metadata(&full)
        .map_err(|_| TerminalApiError::new(StatusCode::NOT_FOUND, "file not found"))?;
    if meta.is_dir() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "path is a directory",
        ));
    }
    let bytes = std::fs::read(&full)
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let truncated = bytes.len() > MAX_FILE_BYTES;
    let slice = if truncated {
        &bytes[..MAX_FILE_BYTES]
    } else {
        &bytes[..]
    };
    let content = String::from_utf8_lossy(slice).into_owned();
    Ok(Json(WorkspaceFileResponse {
        path: rel,
        content,
        truncated,
    }))
}

fn workspace_media_content_type(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("png") | Some("PNG") => Some("image/png"),
        Some("jpg") | Some("jpeg") | Some("JPG") | Some("JPEG") => Some("image/jpeg"),
        Some("gif") | Some("GIF") => Some("image/gif"),
        Some("webp") | Some("WEBP") => Some("image/webp"),
        Some("svg") | Some("SVG") => Some("image/svg+xml"),
        _ => None,
    }
}

pub async fn workspace_media(
    ctx: WorkspaceApiContext,
    session_id: String,
    q: WorkspaceMediaQuery,
) -> Result<impl IntoResponse, TerminalApiError> {
    let rel = safe_rel_path(&q.path)?;
    let root = session_home_under_work_root(&ctx.work_root, q.proj_id, &session_id);
    let full = if rel.is_empty() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "path required",
        ));
    } else {
        root.join(&rel)
    };
    if !full.starts_with(&root) {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "path escapes session root",
        ));
    }
    let content_type = workspace_media_content_type(&full).ok_or_else(|| {
        TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported media type (png, jpg, gif, webp, svg only)",
        )
    })?;
    let meta = std::fs::metadata(&full)
        .map_err(|_| TerminalApiError::new(StatusCode::NOT_FOUND, "file not found"))?;
    if meta.is_dir() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "path is a directory",
        ));
    }
    if meta.len() as usize > MAX_MEDIA_BYTES {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "media file too large",
        ));
    }
    let bytes = std::fs::read(&full)
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "private, max-age=60"),
        ],
        bytes,
    ))
}

#[must_use]
pub fn workspace_api_context(work_root: PathBuf) -> WorkspaceApiContext {
    WorkspaceApiContext { work_root }
}
