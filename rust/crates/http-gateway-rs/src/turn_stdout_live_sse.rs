//! Live `GET /v1/biz_advice_report?stream=true` from worker stdout hub. Author: kejiqing

use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::biz_advice_report::{
    biz_report_sse_event_stream, split_catchup_chunks, sanitize_external_report_text,
    BizAdviceReportPayload, BizReportStreamMsg,
};
use crate::turn_stdout_hub::{HubMsg, TurnStdoutHub};

/// Max chars per catch-up SSE delta when client connects after hub already has text.
const CATCHUP_CHUNK_CHARS: usize = 48;

/// SSE while solve is running: tail stdout `report.delta` events ingested from pool exec.
pub fn live_stdout_report_sse(
    hub: Arc<TurnStdoutHub>,
    turn_id: String,
    task_id: String,
    source_request_id: String,
    source_ds_id: i64,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<BizReportStreamMsg>();
    let turn_id_worker = turn_id.clone();
    tokio::spawn(async move {
        // Atomic (subscribe, snapshot) under one lock — broadcast deliveries strictly
        // after this point are disjoint from the snapshot already in state.text.
        // Without this, every chunk that landed between subscribe and snapshot_text
        // would be emitted twice (catchup + broadcast), causing per-char interleaved
        // duplication on the wire. Author: kejiqing
        let (mut sub, snapshot_raw) = hub.subscribe_with_snapshot(&turn_id_worker);
        let snapshot = sanitize_external_report_text(&snapshot_raw);
        for part in split_catchup_chunks(&snapshot, CATCHUP_CHUNK_CHARS) {
            let _ = tx.send(BizReportStreamMsg::Delta(part));
            tokio::task::yield_now().await;
        }
        // FIFO termination: SolveDone arrives through the same broadcast channel
        // as Delta, guaranteeing all prior deltas were drained before we exit.
        // Polling `hub.solve_done()` alongside the receiver used to break too
        // early, dropping tail chunks queued between the last consumed delta and
        // the solve.done ingest. Author: kejiqing
        loop {
            match sub.recv().await {
                Ok(HubMsg::Delta(chunk)) => {
                    let _ = tx.send(BizReportStreamMsg::Delta(chunk));
                }
                Ok(HubMsg::SolveDone) => break,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
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
    });

    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = HeaderValue::from_static("no");
    (
        AppendHeaders([(no_buffer, no_buffer_val)]),
        Sse::new(biz_report_sse_event_stream(&turn_id, rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
