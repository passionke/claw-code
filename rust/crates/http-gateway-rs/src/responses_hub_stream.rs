//! Responses `stream=true`: open SSE first, enqueue solve, pump LiveReportHub. Author: kejiqing

use std::convert::Infallible;
use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use futures_util::stream;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::agent_completion::responses_api_response;
use crate::pool::{HubMsg, LiveReportHub};
use crate::session_db::{GatewaySessionDb, TurnModelUsageRow};

/// SSE from Hub for OpenAI Responses stream. Author: kejiqing
pub fn responses_hub_sse_response(
    hub: Arc<LiveReportHub>,
    model: String,
    session_id: String,
    turn_id: String,
    session_db: Arc<GatewaySessionDb>,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<(String, String)>();
    let hub_done = Arc::clone(&hub);
    tokio::spawn(async move {
        let created_at = chrono::Utc::now().timestamp();
        let _ = tx.send((
            "response.created".into(),
            json!({
                "type": "response.created",
                "response": {
                    "id": turn_id,
                    "object": "response",
                    "created_at": created_at,
                    "status": "in_progress",
                    "model": model,
                    "output": []
                }
            })
            .to_string(),
        ));

        let (mut sub, snapshot, _, _) = hub.subscribe_with_process_snapshot(&turn_id);
        let mut acc = String::new();
        for chunk in snapshot {
            if chunk.text.is_empty() {
                continue;
            }
            acc.push_str(&chunk.text);
            let _ = tx.send((
                "response.output_text.delta".into(),
                json!({
                    "type": "response.output_text.delta",
                    "delta": chunk.text,
                })
                .to_string(),
            ));
        }

        let mut done = hub.is_solve_done(&turn_id);
        if !done {
            loop {
                match sub.recv().await {
                    Ok(HubMsg::Delta(d)) => {
                        if d.text.is_empty() {
                            continue;
                        }
                        acc.push_str(&d.text);
                        if tx
                            .send((
                                "response.output_text.delta".into(),
                                json!({
                                    "type": "response.output_text.delta",
                                    "delta": d.text,
                                })
                                .to_string(),
                            ))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(HubMsg::Process(pe)) if pe.ev == "tool.start" => {
                        let _ = tx.send((
                            "response.output_item.added".into(),
                            json!({
                                "type": "response.output_item.added",
                                "item": {
                                    "type": "function_call",
                                    "id": pe.payload.get("toolCallId"),
                                    "name": pe.payload.get("name"),
                                    "arguments": pe.payload.get("argsSummary").and_then(|v| v.as_str()).unwrap_or(""),
                                }
                            })
                            .to_string(),
                        ));
                    }
                    Ok(HubMsg::Process(_) | HubMsg::AskUser(_) | HubMsg::AskUserCleared) => {}
                    Ok(HubMsg::SolveDone) | Err(RecvError::Closed) => {
                        done = true;
                        break;
                    }
                    Err(RecvError::Lagged(_)) => {
                        if hub.is_solve_done(&turn_id) {
                            done = true;
                            break;
                        }
                    }
                }
            }
        }

        if acc.is_empty() {
            acc = hub.snapshot_text(&turn_id);
        }
        let usage_rows: Vec<TurnModelUsageRow> = session_db
            .list_model_usage_for_turn(&turn_id)
            .await
            .unwrap_or_default();
        let completed = responses_api_response(
            &model,
            &turn_id,
            &session_id,
            &acc,
            created_at * 1000,
            &usage_rows,
        );
        let _ = tx.send(("response.completed".into(), completed.to_string()));
        let _ = tx.send(("done".into(), "[DONE]".into()));
        if done {
            hub_done.try_remove_turn(&turn_id);
        }
    });

    let event_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some((event, data)) => {
                let ev = Ok::<Event, Infallible>(Event::default().event(event).data(data));
                Some((ev, rx))
            }
            None => None,
        }
    });
    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    (
        AppendHeaders([(no_buffer, HeaderValue::from_static("no"))]),
        Sse::new(event_stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
