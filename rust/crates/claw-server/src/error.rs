use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ServerError::Db(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            ServerError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ServerError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            ServerError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ServerError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            ServerError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}
