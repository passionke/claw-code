//! `GET /v1/sessions/{session_id}/turns/{turn_id}/usage`. Author: kejiqing

use crate::agent_completion::{TurnUsageRequest, TurnUsageSummary};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TurnUsageResponse {
    pub session_id: String,
    pub turn_id: String,
    pub proj_id: i64,
    /// Number of LLM calls recorded for this turn. Author: kejiqing
    pub request_count: u64,
    #[schema(value_type = Object)]
    pub usage: Value,
    #[schema(value_type = Object)]
    pub usage_by_model: Option<Value>,
    pub requests: Vec<TurnUsageRequest>,
}

impl TurnUsageResponse {
    #[must_use]
    pub fn from_summary(
        session_id: String,
        turn_id: String,
        proj_id: i64,
        summary: TurnUsageSummary,
    ) -> Self {
        Self {
            session_id,
            turn_id,
            proj_id,
            request_count: summary.request_count,
            usage: summary.usage,
            usage_by_model: summary.usage_by_model,
            requests: summary.requests,
        }
    }
}
