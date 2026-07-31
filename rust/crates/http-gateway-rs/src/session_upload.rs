//! Session file upload → NAS `sessions/{seg}/uploads/` (+ optional OSS dual-write). Author: kejiqing

use std::path::{Component, Path};

use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use claw_e2b_sandbox_client::session_rel;
use gateway_solve_turn::{SolveAttachment, SolveAttachmentKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api_error::ApiError;
use crate::app_state::AppState;
use crate::cluster_identity::gateway_cluster_id;
use crate::oss_object_store::OssConfig;
use crate::session_merge;

const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionFilesQuery {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    #[param(rename = "projId")]
    pub proj_id: i64,
}

/// Upload / turn-list attachment with optional temporary signed URL. Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionUploadedAttachment {
    #[schema(example = "uploads/photo.png")]
    pub path: String,
    #[schema(example = "image/png")]
    pub mime: String,
    pub kind: SolveAttachmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, rename = "ossKey", skip_serializing_if = "Option::is_none")]
    pub oss_key: Option<String>,
    #[serde(default, rename = "ossUrl", skip_serializing_if = "Option::is_none")]
    pub oss_url: Option<String>,
    #[serde(
        default,
        rename = "ossRetainUntilMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub oss_retain_until_ms: Option<i64>,
    /// Temporary GET URL; not stored on solve request attachments. Author: kejiqing
    #[serde(
        default,
        rename = "ossSignedUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub oss_signed_url: Option<String>,
}

impl SessionUploadedAttachment {
    #[must_use]
    pub fn from_solve_attachment(att: SolveAttachment, oss_signed_url: Option<String>) -> Self {
        Self {
            path: att.path,
            mime: att.mime,
            kind: att.kind,
            name: att.name,
            size: att.size,
            oss_key: att.oss_key,
            oss_url: att.oss_url,
            oss_retain_until_ms: att.oss_retain_until_ms,
            oss_signed_url,
        }
    }

    pub fn from_json_value(v: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(v.clone()).ok()
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionFilesUploadResponse {
    pub attachments: Vec<SessionUploadedAttachment>,
}

/// Classify attachment kind from MIME / filename. Unknown → reject. Author: kejiqing
pub fn classify_attachment(mime: &str, name: &str) -> Result<SolveAttachmentKind, String> {
    let mime = mime.trim().to_ascii_lowercase();
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if mime.starts_with("image/") || matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
    {
        return Ok(SolveAttachmentKind::Image);
    }
    if mime.starts_with("video/")
        || matches!(ext.as_str(), "mp4" | "webm" | "mov" | "mkv" | "avi" | "m4v")
    {
        return Ok(SolveAttachmentKind::Video);
    }
    if mime.starts_with("audio/")
        || matches!(ext.as_str(), "wav" | "mp3" | "m4a" | "ogg" | "flac" | "aac" | "opus")
    {
        return Ok(SolveAttachmentKind::Audio);
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
        return Ok(SolveAttachmentKind::Document);
    }
    Err(format!(
        "unsupported file type mime={mime} name={name}; allow images (png/jpeg/webp/gif), video (mp4/webm/mov/mkv), audio (wav/mp3/m4a/ogg/flac), and documents (pdf/csv/txt/md/json)"
    ))
}

/// Fill `url` on video/audio attachments with a fresh OSS presigned GET (model wire). Author: kejiqing
pub fn enrich_media_attachment_urls(attachments: &mut [SolveAttachment], oss: &OssConfig) {
    if !oss.enabled() {
        return;
    }
    let now = Utc::now();
    let ttl = oss.signed_url_ttl_secs;
    for att in attachments.iter_mut() {
        if !matches!(
            att.kind,
            SolveAttachmentKind::Video | SolveAttachmentKind::Audio
        ) {
            continue;
        }
        if att
            .url
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
        {
            continue;
        }
        if let Some(key) = att.oss_key.as_deref().filter(|k| !k.is_empty()) {
            if let Ok(signed) = oss.presign_get(key, ttl, now) {
                att.url = Some(signed);
                continue;
            }
        }
        if let Some(u) = att.oss_url.clone().filter(|s| !s.trim().is_empty()) {
            att.url = Some(u);
        }
    }
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
) -> Result<Json<SessionFilesUploadResponse>, ApiError> {
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
            kind,
            name: Some(original),
            size: Some(bytes.len() as u64),
            oss_key,
            oss_url,
            oss_retain_until_ms,
            url: None,
        });
    }

    if uploaded.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "no file parts found; use multipart field name \"file\"",
        ));
    }
    // Enrich response with temporary ossSignedUrl (not stored in entry_params / SolveAttachment).
    let attachments = if oss.enabled() {
        let now = Utc::now();
        let ttl = oss.signed_url_ttl_secs;
        uploaded
            .into_iter()
            .map(|att| {
                let signed = att
                    .oss_key
                    .as_deref()
                    .and_then(|k| oss.presign_get(k, ttl, now).ok());
                SessionUploadedAttachment::from_solve_attachment(att, signed)
            })
            .collect()
    } else {
        uploaded
            .into_iter()
            .map(|att| SessionUploadedAttachment::from_solve_attachment(att, None))
            .collect()
    };
    Ok(Json(SessionFilesUploadResponse { attachments }))
}

#[cfg(test)]
mod tests {
    use super::classify_attachment;
    use gateway_solve_turn::SolveAttachmentKind;

    #[test]
    fn classify_image_document_video_audio() {
        assert_eq!(
            classify_attachment("image/png", "a.png").unwrap(),
            SolveAttachmentKind::Image
        );
        assert_eq!(
            classify_attachment("application/pdf", "r.pdf").unwrap(),
            SolveAttachmentKind::Document
        );
        assert_eq!(
            classify_attachment("text/csv", "t.csv").unwrap(),
            SolveAttachmentKind::Document
        );
        assert_eq!(
            classify_attachment("video/mp4", "clip.mp4").unwrap(),
            SolveAttachmentKind::Video
        );
        assert_eq!(
            classify_attachment("video/webm", "clip.webm").unwrap(),
            SolveAttachmentKind::Video
        );
        assert_eq!(
            classify_attachment("audio/mpeg", "a.mp3").unwrap(),
            SolveAttachmentKind::Audio
        );
        assert_eq!(
            classify_attachment("audio/wav", "a.wav").unwrap(),
            SolveAttachmentKind::Audio
        );
        assert!(classify_attachment("application/zip", "x.zip").is_err());
    }
}
