//! Session file upload → NAS `sessions/{seg}/uploads/` (+ optional OSS dual-write). Author: kejiqing

use std::path::{Component, Path};

use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use claw_e2b_sandbox_client::session_rel;
use gateway_solve_turn::SolveAttachment;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api_error::ApiError;
use crate::app_state::AppState;
use crate::cluster_identity::gateway_cluster_id;
use crate::oss_object_store::OssConfig;
use crate::session_merge;

const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionFilesQuery {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    #[param(rename = "projId")]
    pub proj_id: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionFilesUploadResponse {
    #[schema(value_type = Vec<Object>)]
    pub attachments: Vec<Value>,
}

/// Classify attachment kind from MIME / filename. Unknown → reject. Author: kejiqing
pub fn classify_attachment(mime: &str, name: &str) -> Result<&'static str, String> {
    let mime = mime.trim().to_ascii_lowercase();
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if mime.starts_with("image/") || matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
    {
        return Ok("image");
    }
    if matches!(
        mime.as_str(),
        "application/pdf"
            | "text/plain"
            | "text/csv"
            | "application/csv"
            | "text/markdown"
            | "application/json"
    ) || matches!(ext.as_str(), "pdf" | "txt" | "csv" | "md" | "json")
    {
        return Ok("document");
    }
    Err(format!(
        "unsupported file type mime={mime} name={name}; allow images (png/jpeg/webp/gif) and documents (pdf/csv/txt/md/json)"
    ))
}

fn safe_original_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

fn mime_for_name(name: &str, provided: Option<&str>) -> String {
    if let Some(m) = provided.map(str::trim).filter(|s| !s.is_empty()) {
        return m.to_string();
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png".into(),
        "jpg" | "jpeg" => "image/jpeg".into(),
        "webp" => "image/webp".into(),
        "gif" => "image/gif".into(),
        "pdf" => "application/pdf".into(),
        "csv" => "text/csv".into(),
        "txt" => "text/plain".into(),
        "md" => "text/markdown".into(),
        "json" => "application/json".into(),
        _ => "application/octet-stream".into(),
    }
}

/// `POST /v1/sessions/{session_id}/files?projId=` multipart field `file`. Author: kejiqing
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/files",
    tag = "Sessions",
    operation_id = "session_upload_upload_session_files",
    summary = "Upload session files (multipart)",
    description = "Accepts `multipart/form-data` with field name `file` or `files`. Requires prior POST /v1/start.",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        SessionFilesQuery
    ),
    responses(
        (status = 200, description = "Uploaded attachment metadata", body = SessionFilesUploadResponse),
        (status = 400, description = "Unknown session, empty file, or unsupported type")
    )
)]
pub(crate) async fn upload_session_files(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<SessionFilesQuery>,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    if q.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let session_id = session_merge::trim_session_id(Some(session_id.as_str()))
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "sessionId is required"))?
        .to_string();
    let home_rel = state
        .session_db
        .get_session_home_rel(&session_id, q.proj_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("lookup session failed: {e}"),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown sessionId {session_id}; call POST /v1/start first"),
            )
        })?;
    session_merge::validate_session_home_rel(&home_rel)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid session_home in registry"))?;
    let segment = session_merge::sessions_directory_segment(&session_id);
    let cluster_id = gateway_cluster_id().map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cluster id: {e}"),
        )
    })?;
    let local_home = session_merge::join_session_home_from_rel(&state.cfg.work_root, &home_rel);
    let oss = OssConfig::from_env();
    let mut uploaded: Vec<SolveAttachment> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("multipart parse failed: {e}"),
        )
    })? {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name != "file" && field_name != "files" {
            continue;
        }
        let original = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "file".to_string());
        let provided_mime = field.content_type().map(|m| m.to_string());
        let bytes = field.bytes().await.map_err(|e| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("read upload bytes failed: {e}"),
            )
        })?;
        if bytes.is_empty() {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty file upload"));
        }
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("file too large (max {MAX_UPLOAD_BYTES} bytes)"),
            ));
        }
        let mime = mime_for_name(&original, provided_mime.as_deref());
        let kind = classify_attachment(&mime, &original)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
        let safe = safe_original_name(&original);
        let short = &Uuid::new_v4().simple().to_string()[..8];
        let file_leaf = format!("{short}_{safe}");
        let rel_under = format!("uploads/{file_leaf}");
        if Path::new(&rel_under)
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid upload path",
            ));
        }
        let nas_rel = format!(
            "{}/{rel_under}",
            session_rel(&cluster_id, q.proj_id, &segment)
        );
        // Ensure uploads/ on NAS then put file.
        let uploads_dir = format!("{}/uploads", session_rel(&cluster_id, q.proj_id, &segment));
        let _ = state.nas_api.mkdir(&uploads_dir, true).await;
        state
            .nas_api
            .put_file(&nas_rel, &bytes)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("NAS put_file failed: {e}"),
                )
            })?;
        let local = local_home.join(&rel_under);
        if let Some(parent) = local.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&local, &bytes).await;

        let mut oss_key = None;
        let mut oss_url = None;
        let mut oss_retain_until_ms = None;
        if oss.enabled() {
            let key = oss.build_attachment_key(&cluster_id, q.proj_id, &session_id, &file_leaf);
            oss.put_object(&key, &bytes, &mime).await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("OSS put_object failed: {e}"),
                )
            })?;
            oss_url = Some(oss.object_url(&key));
            oss_retain_until_ms = Some(oss.retain_until_ms(Utc::now()));
            oss_key = Some(key);
        }

        uploaded.push(SolveAttachment {
            path: rel_under,
            mime,
            kind: kind.to_string(),
            name: Some(original),
            size: Some(bytes.len() as u64),
            oss_key,
            oss_url,
            oss_retain_until_ms,
        });
    }

    if uploaded.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "no file parts found; use multipart field name \"file\"",
        ));
    }
    // Enrich response with temporary ossSignedUrl (not stored in entry_params / SolveAttachment).
    let attachments_json: Value = if oss.enabled() {
        let now = Utc::now();
        let ttl = oss.signed_url_ttl_secs;
        Value::Array(
            uploaded
                .into_iter()
                .map(|att| {
                    let signed = att
                        .oss_key
                        .as_deref()
                        .and_then(|k| oss.presign_get(k, ttl, now).ok());
                    let mut v = serde_json::to_value(&att).unwrap_or(json!({}));
                    if let (Some(url), Some(obj)) = (signed, v.as_object_mut()) {
                        obj.insert("ossSignedUrl".into(), Value::String(url));
                    }
                    v
                })
                .collect(),
        )
    } else {
        json!(uploaded)
    };
    Ok(Json(json!({ "attachments": attachments_json })))
}

#[cfg(test)]
mod tests {
    use super::classify_attachment;

    #[test]
    fn classify_image_and_document() {
        assert_eq!(classify_attachment("image/png", "a.png").unwrap(), "image");
        assert_eq!(
            classify_attachment("application/pdf", "r.pdf").unwrap(),
            "document"
        );
        assert_eq!(
            classify_attachment("text/csv", "t.csv").unwrap(),
            "document"
        );
        assert!(classify_attachment("application/zip", "x.zip").is_err());
    }
}
