//! AG-UI run stream routes. Author: kejiqing

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::api_error::ApiError;
use crate::app_state::AppState;
use crate::gateway_owner_proxy;
use crate::pool;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[allow(clippy::struct_field_names)] // query wire names: sessionId / turnId / projId
#[into_params(parameter_in = Query)]
pub(crate) struct AgUiRunQuery {
    #[serde(rename = "sessionId")]
    #[param(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "turnId")]
    #[param(rename = "turnId")]
    turn_id: String,
    #[serde(rename = "projId")]
    #[param(rename = "projId")]
    proj_id: i64,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/v1/ag-ui/runs", get(get_ag_ui_run))
}

/// Live AG-UI SSE for process disclosure (tools / A2UI). Report body stays on biz.report.
/// Author: kejiqing
#[utoipa::path(
    get,
    path = "/v1/ag-ui/runs",
    tag = "Sessions",
    operation_id = "get_ag_ui_run",
    params(AgUiRunQuery),
    responses(
        (status = 200, description = "AG-UI SSE process disclosure stream", content_type = "text/event-stream"),
        (status = 400, description = "Missing sessionId or turnId"),
        (status = 502, description = "Owner gateway proxy failed"),
        (status = 503, description = "Owner resolution unavailable")
    )
)]
pub(crate) async fn get_ag_ui_run(
    State(state): State<AppState>,
    Query(query): Query<AgUiRunQuery>,
) -> Result<Response, ApiError> {
    let session_id = query.session_id.trim();
    let turn_id = query.turn_id.trim();
    if session_id.is_empty() || turn_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId and turnId are required",
        ));
    }
    // Owner proxy (multi-gateway): same pattern as biz_advice_report.
    match gateway_owner_proxy::resolve_turn_owner_proxy_base(
        &state.session_db,
        state.gateway_identity.as_ref(),
        turn_id,
        session_id,
        query.proj_id,
    )
    .await
    {
        Ok(Some(owner_base)) => {
            let path = format!(
                "/v1/ag-ui/runs?sessionId={}&turnId={}&projId={}",
                session_id, turn_id, query.proj_id
            );
            let headers = HeaderMap::new();
            return gateway_owner_proxy::proxy_to_owner_gateway(
                &owner_base,
                "GET",
                &path,
                &headers,
            )
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e));
        }
        Ok(None) => {}
        Err(e) => {
            return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e));
        }
    }

    Ok(pool::live_ag_ui_sse_response(
        Arc::clone(&state.live_report_hub),
        session_id,
        turn_id,
    ))
}
