//! Live AG-UI SSE from LiveReportHub (`GET /v1/ag-ui/runs`). Author: kejiqing

use std::convert::Infallible;
use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use futures_util::stream;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::ag_ui_projector::{
    ag_ui_from_ask_user, ag_ui_run_finished, ag_ui_run_started, project_hub_msg, ProcessStep,
};
use crate::pool::{HubMsg, LiveReportHub};

pub fn live_ag_ui_sse_response(
    hub: Arc<LiveReportHub>,
    session_id: &str,
    turn_id: &str,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let turn = turn_id.to_string();
    let thread = session_id.to_string();
    let hub_done = Arc::clone(&hub);
    tokio::spawn(async move {
        let _ = tx.send(ag_ui_run_started(&thread, &turn).to_string());
        let (mut sub, _deltas, process_snap, pending_ask) =
            hub.subscribe_with_process_snapshot(&turn);
        let mut steps: Vec<ProcessStep> = Vec::new();
        for pe in &process_snap {
            for ev in project_hub_msg(&turn, &HubMsg::Process(pe.clone()), &mut steps) {
                if tx.send(ev.to_string()).is_err() {
                    return;
                }
            }
        }
        if let Some(ask) = pending_ask {
            let _ = tx.send(ag_ui_from_ask_user(&ask).to_string());
        }
        if hub.is_solve_done(&turn) {
            let _ = tx.send(ag_ui_run_finished(&thread, &turn).to_string());
            hub_done.try_remove_turn(&turn);
            return;
        }
        loop {
            match sub.recv().await {
                Ok(HubMsg::SolveDone) | Err(RecvError::Closed) => {
                    let _ = tx.send(ag_ui_run_finished(&thread, &turn).to_string());
                    break;
                }
                Ok(msg) => {
                    for ev in project_hub_msg(&turn, &msg, &mut steps) {
                        if tx.send(ev.to_string()).is_err() {
                            return;
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    if hub.is_solve_done(&turn) {
                        let _ = tx.send(ag_ui_run_finished(&thread, &turn).to_string());
                        break;
                    }
                }
            }
        }
        hub_done.try_remove_turn(&turn);
    });

    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let event_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(data) => {
                let ev = Ok::<Event, Infallible>(Event::default().data(data));
                Some((ev, rx))
            }
            None => None,
        }
    });
    (
        AppendHeaders([(no_buffer, HeaderValue::from_static("no"))]),
        Sse::new(event_stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
