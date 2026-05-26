//! Live SSE from pool-local stdout hub (`GET /v1/biz_advice_report/live`). Author: kejiqing

use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::biz_advice_report::{
    biz_report_sse_event_stream, sanitize_external_report_text, BizAdviceReportPayload,
    BizReportStreamMsg,
};
use crate::pool::live_report_hub::{HubMsg, LiveReportHub};

pub fn live_report_sse_response(
    hub: Arc<LiveReportHub>,
    turn_id: &str,
    task_id: String,
    source_request_id: String,
    source_ds_id: i64,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<BizReportStreamMsg>();
    let turn_id_worker = turn_id.to_string();
    let hub_done = Arc::clone(&hub);
    tokio::spawn(async move {
        // Live tail only: no catch-up replay. Admin opens SSE when turn is running; `done` carries full text.
        let (mut sub, _) = hub.subscribe_with_snapshot(&turn_id_worker);
        loop {
            match sub.recv().await {
                Ok(HubMsg::Delta(chunk)) => {
                    let _ = tx.send(BizReportStreamMsg::Delta(chunk));
                }
                Ok(HubMsg::SolveDone) | Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => {}
            }
        }
        let final_text = sanitize_external_report_text(&hub.snapshot_text(&turn_id_worker));
        let done = BizAdviceReportPayload {
            task_id,
            source_request_id,
            source_ds_id,
            source_status: "running".into(),
            report_text: Some(final_text.clone()),
            report_json: Some(json!({ "message": final_text })),
        };
        let _ = tx.send(BizReportStreamMsg::Done(done));
        hub_done.try_remove_turn(&turn_id_worker);
    });

    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = HeaderValue::from_static("no");
    (
        AppendHeaders([(no_buffer, no_buffer_val)]),
        Sse::new(biz_report_sse_event_stream(turn_id, rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
