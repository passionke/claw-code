//! API error type. Author: kejiqing

use crate::session_merge;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub(crate) fn detail(&self) -> &str {
        &self.message
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "detail": self.message }))).into_response()
    }
}

pub(crate) fn session_routing_error(e: session_merge::SessionRoutingError) -> ApiError {
    let status = match e {
        session_merge::SessionRoutingError::AbsNotUnderWorkRoot => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    };
    ApiError::new(status, e.detail())
}
