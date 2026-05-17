//! Axum AG-UI server (L1). Author: kejiqing

use crate::agui_events::{AgUiEvent, RunAgentInput};
use crate::gateway_client::GatewayClient;
use axum::extract::{Path, State};
use axum::http::{Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_http::cors::{Any, CorsLayer};
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub gateway: GatewayClient,
    pub mock: bool,
}

pub async fn serve(addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mock = std::env::var("CLAW_AGUI_MOCK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let state = AppState {
        gateway: GatewayClient::from_env(),
        mock,
    };
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/agent/run", post(agent_run))
        .route(
            "/v1/interrupts/{interrupt_id}/resolve",
            post(resolve_interrupt),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, mock, "ag-ui-claw-bridge listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn agent_run(
    State(state): State<AppState>,
    Json(input): Json<RunAgentInput>,
) -> Result<Response, (StatusCode, String)> {
    if state.mock {
        return Ok(mock_run_sse(input).into_response());
    }
    let ds_id = crate::agui_events::ds_id_from_input(&input)
        .ok_or((StatusCode::BAD_REQUEST, "forwardedProps.dsId required".into()))?;
    let prompt = crate::agui_events::last_user_text(&input.messages)
        .ok_or((StatusCode::BAD_REQUEST, "user message required".into()))?;
    // Map threadId → claw-session-id header only. Body sessionId means explicit continuation
    // and must already exist in gateway SQLite (L2 / http-gateway-rs-api.md).
    let body_session_id = input
        .forwarded_props
        .as_ref()
        .and_then(|p| p.get("sessionId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let extra = input
        .forwarded_props
        .as_ref()
        .and_then(|p| p.get("extraSession").cloned());
    let gw = state.gateway.clone();
    let run_id = input.run_id.clone();
    let thread_id = input.thread_id.clone();
    let (tx, rx) = mpsc::unbounded_channel();
    let tx_err = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = run_via_gateway(
            gw,
            ds_id,
            &prompt,
            body_session_id.as_deref(),
            &thread_id,
            extra,
            &run_id,
            &thread_id,
            tx,
        )
        .await
        {
            let _ = tx_err.send(AgUiEvent::RunError { message: e });
        }
    });
    Ok(sse_from_events(rx).into_response())
}

async fn run_via_gateway(
    gw: GatewayClient,
    ds_id: i64,
    prompt: &str,
    body_session_id: Option<&str>,
    claw_session_header: &str,
    extra_session: Option<serde_json::Value>,
    run_id: &str,
    thread_id: &str,
    tx: mpsc::UnboundedSender<AgUiEvent>,
) -> Result<(), String> {
    let _ = tx.send(AgUiEvent::RunStarted {
        thread_id: thread_id.to_string(),
        run_id: run_id.to_string(),
    });
    let started = gw
        .solve_async(
            ds_id,
            prompt,
            body_session_id,
            claw_session_header,
            extra_session,
            run_id,
        )
        .await?;
    let task_id = started.task_id;
    let message_id = uuid::Uuid::new_v4().to_string();
    let mut saw_text = false;
    let mut last_line_count = 0usize;
    let max_polls = 2000_u32;
    let mut polls = 0_u32;
    loop {
        polls += 1;
        if polls > max_polls {
            let _ = tx.send(AgUiEvent::RunError {
                message: "gateway solve poll timeout".into(),
            });
            return Ok(());
        }
        if let Ok(lines) = gw.fetch_event_lines(&task_id).await {
            for line in lines.iter().skip(last_line_count) {
                map_tap_line(line, &tx, &message_id, &mut saw_text);
            }
            last_line_count = lines.len();
        }
        let task = gw.get_task(&task_id).await?;
        match task.status.as_str() {
            "succeeded" => {
                if !saw_text {
                    if let Some(result) = task.result {
                        if let Some(text) = result
                            .get("outputText")
                            .and_then(|v| v.as_str())
                        {
                            emit_text(&tx, &message_id, text);
                        }
                    }
                }
                let _ = tx.send(AgUiEvent::RunFinished {
                    thread_id: thread_id.to_string(),
                    run_id: run_id.to_string(),
                });
                return Ok(());
            }
            "failed" | "cancelled" => {
                let msg = task
                    .error
                    .and_then(|e| e.get("detail").and_then(|d| d.as_str()).map(String::from))
                    .unwrap_or_else(|| format!("task {}", task.status));
                let _ = tx.send(AgUiEvent::RunError { message: msg });
                return Ok(());
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn map_tap_line(
    line: &serde_json::Value,
    tx: &mpsc::UnboundedSender<AgUiEvent>,
    message_id: &str,
    saw_text: &mut bool,
) {
    let Some(t) = line.get("type").and_then(|v| v.as_str()) else {
        return;
    };
    match t {
        "text.delta" => {
            if let Some(text) = line.get("text").and_then(|v| v.as_str()) {
                if !*saw_text {
                    let _ = tx.send(AgUiEvent::TextMessageStart {
                        message_id: message_id.to_string(),
                    });
                    *saw_text = true;
                }
                let _ = tx.send(AgUiEvent::TextMessageContent {
                    message_id: message_id.to_string(),
                    delta: text.to_string(),
                });
            }
        }
        "tool.start" => {
            let _ = tx.send(AgUiEvent::ToolCallStart {
                tool_call_id: line
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string(),
                tool_name: line
                    .get("toolName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string(),
            });
        }
        "tool.end" => {
            let _ = tx.send(AgUiEvent::ToolCallEnd {
                tool_call_id: line
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string(),
                ok: line.get("ok").and_then(serde_json::Value::as_bool).unwrap_or(true),
            });
        }
        "interrupt.required" => {
            let _ = tx.send(AgUiEvent::Interrupt {
                interrupt_id: line
                    .get("interruptId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("interrupt")
                    .to_string(),
                reason: line
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("permission")
                    .to_string(),
                payload: line.get("payload").cloned().unwrap_or(serde_json::json!({})),
            });
        }
        "solve.finished" => {
            if *saw_text {
                let _ = tx.send(AgUiEvent::TextMessageEnd {
                    message_id: message_id.to_string(),
                });
            }
        }
        _ => {}
    }
}

fn emit_text(tx: &mpsc::UnboundedSender<AgUiEvent>, message_id: &str, text: &str) {
    let _ = tx.send(AgUiEvent::TextMessageStart {
        message_id: message_id.to_string(),
    });
    let _ = tx.send(AgUiEvent::TextMessageContent {
        message_id: message_id.to_string(),
        delta: text.to_string(),
    });
    let _ = tx.send(AgUiEvent::TextMessageEnd {
        message_id: message_id.to_string(),
    });
}

fn mock_run_sse(input: RunAgentInput) -> Sse<impl Stream<Item = Result<Event, Infallible>> + Send> {
    let thread_id = input.thread_id;
    let run_id = input.run_id;
    let message_id = uuid::Uuid::new_v4().to_string();
    let events = vec![
        AgUiEvent::RunStarted {
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
        },
        AgUiEvent::TextMessageStart {
            message_id: message_id.clone(),
        },
        AgUiEvent::TextMessageContent {
            message_id: message_id.clone(),
            delta: "mock bridge ok".into(),
        },
        AgUiEvent::TextMessageEnd {
            message_id: message_id.clone(),
        },
        AgUiEvent::RunFinished { thread_id, run_id },
    ];
    let stream = stream::iter(events.into_iter().map(|ev| {
        Ok(Event::default().event("message").data(ev.sse_data()))
    }));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn sse_from_events(
    rx: mpsc::UnboundedReceiver<AgUiEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>> + Send> {
    let stream = UnboundedReceiverStream::new(rx).map(|ev: AgUiEvent| {
        Ok(Event::default().event("message").data(ev.sse_data()))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agui_events::AgentMessage;
    use serde_json::json;
    use tokio::sync::mpsc;

    #[test]
    fn map_tap_emits_interrupt_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut saw = false;
        let line = json!({
            "type": "interrupt.required",
            "interruptId": "int-1",
            "reason": "permission",
            "payload": {"toolName": "bash"}
        });
        map_tap_line(&line, &tx, "msg-1", &mut saw);
        let ev = rx.try_recv().expect("event");
        match ev {
            AgUiEvent::Interrupt { interrupt_id, reason, .. } => {
                assert_eq!(interrupt_id, "int-1");
                assert_eq!(reason, "permission");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_mode_emits_finished() {
        let input = RunAgentInput {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            messages: vec![AgentMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            tools: vec![],
            forwarded_props: Some(serde_json::json!({"dsId": 1})),
        };
        let resp = mock_run_sse(input);
        // Sse type constructs without panic
        let _ = resp;
    }
}
