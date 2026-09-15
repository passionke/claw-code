// Fragment of routes::app (include!). Session inbox receive/list/drain. Author: kejiqing

use crate::session_inbox::{InboxDrainResult, InboxEnqueueResult, InboxError, InboxMessageRow, InboxSummary};

fn inbox_api_err(e: InboxError) -> ApiError {
    match e {
        InboxError::BadRequest(m) => ApiError::new(StatusCode::BAD_REQUEST, m),
        InboxError::Forbidden(m) => ApiError::new(StatusCode::FORBIDDEN, m),
        InboxError::NotFound(m) => ApiError::new(StatusCode::NOT_FOUND, m),
        InboxError::Db(e) => session_db_err(&e),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxEnqueueBody {
    #[serde(rename = "projId", alias = "proj_id")]
    pub proj_id: i64,
    pub source: String,
    pub body: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Sender mailbox `sessionId@projId.clusterId` (required for source=mailbox). Author: kejiqing
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxListQuery {
    #[serde(rename = "projId", alias = "proj_id")]
    pub proj_id: i64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxDrainBody {
    #[serde(rename = "projId", alias = "proj_id")]
    pub proj_id: i64,
    #[serde(rename = "turnId", alias = "turn_id")]
    pub turn_id: String,
    #[serde(default)]
    pub iteration: Option<i32>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxListResponse {
    pub messages: Vec<InboxMessageRow>,
    pub summary: InboxSummary,
}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/inbox",
    tag = "Sessions",
    operation_id = "post_session_inbox",
    params(("session_id" = String, Path, description = "Session id")),
    request_body = InboxEnqueueBody,
    responses(
        (status = 200, description = "Message queued", body = InboxEnqueueResult),
        (status = 400, description = "Bad request"),
        (status = 403, description = "Not steerable"),
        (status = 404, description = "Session not found")
    )
)]
pub(crate) async fn post_session_inbox(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<InboxEnqueueBody>,
) -> Result<Json<InboxEnqueueResult>, ApiError> {
    if body.proj_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "projId must be >= 1"));
    }
    let result = state
        .session_db
        .inbox_enqueue(
            &session_id,
            body.proj_id,
            &body.source,
            &body.body,
            body.idempotency_key.as_deref(),
            *state.inbox_capacity,
            body.from_address.as_deref(),
            body.in_reply_to.as_deref(),
            body.references.as_deref(),
        )
        .await
        .map_err(inbox_api_err)?;
    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/inbox",
    tag = "Sessions",
    operation_id = "get_session_inbox",
    params(
        ("session_id" = String, Path, description = "Session id"),
        InboxListQuery
    ),
    responses(
        (status = 200, description = "Inbox list + summary", body = InboxListResponse),
        (status = 403, description = "Not steerable")
    )
)]
pub(crate) async fn get_session_inbox(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<InboxListQuery>,
) -> Result<Json<InboxListResponse>, ApiError> {
    if q.proj_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "projId must be >= 1"));
    }
    let (messages, summary) = state
        .session_db
        .inbox_list(
            &session_id,
            q.proj_id,
            q.source.as_deref(),
            q.status.as_deref(),
            *state.inbox_capacity,
            q.limit.unwrap_or(50),
        )
        .await
        .map_err(inbox_api_err)?;
    Ok(Json(InboxListResponse { messages, summary }))
}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/inbox/drain",
    tag = "Sessions",
    operation_id = "post_session_inbox_drain",
    params(("session_id" = String, Path, description = "Session id")),
    request_body = InboxDrainBody,
    responses(
        (status = 200, description = "Drained messages", body = InboxDrainResult),
        (status = 403, description = "Not allowed"),
        (status = 400, description = "Bad request")
    )
)]
pub(crate) async fn post_session_inbox_drain(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<InboxDrainBody>,
) -> Result<Json<InboxDrainResult>, ApiError> {
    if body.proj_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "projId must be >= 1"));
    }
    let result = state
        .session_db
        .inbox_drain(
            &session_id,
            body.proj_id,
            &body.turn_id,
            body.iteration.unwrap_or(0),
            *state.inbox_capacity,
        )
        .await
        .map_err(inbox_api_err)?;
    Ok(Json(result))
}
