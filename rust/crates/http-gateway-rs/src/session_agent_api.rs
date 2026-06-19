//! Agent WebSocket bridge for OVS `@claw` Chat (JSON + CDP over ttyd). Author: kejiqing

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use tracing::warn;

use crate::client_origin::CLIENT_ORIGIN_OVS_CHAT;
use crate::persistence::transcript;
use crate::pool::terminal_ws_connect_url;
use crate::session_db::GatewaySessionDb;
use crate::session_ovs_api::{ovs_agent_session_id, ovs_chat_record_session_id};
use crate::session_terminal_api::{ensure_terminal_active, TerminalApiContext, TerminalApiError};
use crate::turn_id;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProjQuery {
    pub proj_id: i64,
    /// OVS Chat panel id for `gateway_turns` only; worker REPL stays `ovs-{projId}`.
    pub chat_session_id: Option<String>,
}

const OSC_PREFIX: &str = "\x1b]1337;Claw;";
const OSC_SUFFIX: char = '\x07';
const TTYD_SPAWN_COLS: u16 = 120;
const TTYD_SPAWN_ROWS: u16 = 24;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum AgentClientMsg {
    Spawn,
    Prompt { text: String },
}

struct ActiveOvsTurn {
    turn_id: String,
    buffer: String,
}

fn extract_cdp_frames(input: &str) -> (Vec<serde_json::Value>, String) {
    let mut frames = Vec::new();
    let mut clean = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(OSC_PREFIX) {
        clean.push_str(&rest[..start]);
        rest = &rest[start + OSC_PREFIX.len()..];
        if let Some(end) = rest.find(OSC_SUFFIX) {
            let encoded = &rest[..end];
            rest = &rest[end + OSC_SUFFIX.len_utf8()..];
            if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded) {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    frames.push(v);
                }
            }
        } else {
            break;
        }
    }
    clean.push_str(rest);
    (frames, clean)
}

fn ttyd_input_frame(text: &str) -> Vec<u8> {
    let body = text.as_bytes();
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(0x30);
    out.extend_from_slice(body);
    out
}

fn ttyd_resize_frame(cols: u16, rows: u16) -> Vec<u8> {
    let json = format!(r#"{{"columns":{cols},"rows":{rows}}}"#);
    let body = json.as_bytes();
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(0x31);
    out.extend_from_slice(body);
    out
}

async fn finalize_active_turn(
    db: &GatewaySessionDb,
    active: &mut Option<ActiveOvsTurn>,
    status: &str,
) {
    let Some(turn) = active.take() else {
        return;
    };
    let finished_at = transcript::now_ms();
    let report = if turn.buffer.trim().is_empty() {
        None
    } else {
        Some(turn.buffer.as_str())
    };
    if let Err(e) = db
        .finalize_turn_terminal(&turn.turn_id, status, Some(finished_at), report, None, None)
        .await
    {
        warn!(
            target: "claw_gateway_agent",
            turn_id = %turn.turn_id,
            error = %e,
            "finalize ovs-chat turn failed"
        );
    }
}

async fn start_ovs_turn(
    db: &GatewaySessionDb,
    proj_id: i64,
    record_session_id: &str,
    worker_session_id: &str,
    ovs_chat_key: Option<&str>,
    user_prompt: &str,
) -> Result<String, String> {
    let turn_id = turn_id::mint_turn_id();
    let now = transcript::now_ms();
    let entry = serde_json::json!({
        "projId": proj_id,
        "sessionId": record_session_id,
        "workerSessionId": worker_session_id,
        "ovsChatKey": ovs_chat_key,
        "source": "ovs-agent-ws",
    });
    db.insert_turn(
        &turn_id,
        record_session_id,
        proj_id,
        "running",
        now,
        Some(user_prompt),
        Some(CLIENT_ORIGIN_OVS_CHAT),
        Some(&entry),
    )
    .await
    .map_err(|e| format!("insert ovs-chat turn: {e}"))?;
    Ok(turn_id)
}

async fn ensure_ovs_chat_record_session(
    ctx: &TerminalApiContext,
    proj_id: i64,
    record_session_id: &str,
) -> Result<(), String> {
    if ctx
        .session_db
        .session_exists(record_session_id, proj_id)
        .await
        .map_err(|e| format!("session registry lookup: {e}"))?
    {
        let now = transcript::now_ms();
        ctx.session_db
            .touch_updated(record_session_id, proj_id, now)
            .await
            .map_err(|e| format!("session registry touch: {e}"))?;
        return Ok(());
    }
    let seg = crate::session_merge::sessions_directory_segment(record_session_id);
    let session_home = ctx
        .work_root
        .join(format!("proj_{proj_id}"))
        .join("sessions")
        .join(&seg);
    tokio::fs::create_dir_all(session_home.join(".claw"))
        .await
        .map_err(|e| format!("mkdir chat record session: {e}"))?;
    let session_home_rel = format!("proj_{proj_id}/sessions/{seg}");
    let now = transcript::now_ms();
    ctx.session_db
        .insert_session(
            record_session_id,
            proj_id,
            &session_home_rel,
            now,
            Some(CLIENT_ORIGIN_OVS_CHAT),
        )
        .await
        .map_err(|e| format!("session registry insert: {e}"))?;
    Ok(())
}

/// Worker REPL is always `ovs-{projId}`; path `session_id` is legacy/compat only.
fn ovs_worker_session_id(proj_id: i64, path_session_id: &str) -> String {
    if path_session_id.starts_with("ovs-") {
        ovs_agent_session_id(proj_id)
    } else {
        path_session_id.to_string()
    }
}

fn ovs_record_session_id(
    proj_id: i64,
    path_session_id: &str,
    chat_session_id: Option<&str>,
) -> String {
    if let Some(key) = chat_session_id.map(str::trim).filter(|s| !s.is_empty()) {
        if key.starts_with("ovs-chat-") {
            return key.to_string();
        }
        return ovs_chat_record_session_id(proj_id, key);
    }
    if path_session_id.starts_with("ovs-chat-") {
        return path_session_id.to_string();
    }
    if path_session_id.starts_with("ovs-") {
        return ovs_agent_session_id(proj_id);
    }
    path_session_id.to_string()
}

pub async fn agent_ws_upgrade(
    ctx: TerminalApiContext,
    session_id: String,
    q: AgentProjQuery,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return TerminalApiError::new(StatusCode::BAD_REQUEST, "sessionId required")
            .into_response();
    }
    if q.proj_id < 1 {
        return TerminalApiError::new(StatusCode::BAD_REQUEST, "projId must be >= 1")
            .into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = run_agent_ws_bridge(ctx, q, session_id, socket).await {
            warn!(
                target: "claw_gateway_agent",
                error = %e,
                "agent ws bridge ended with error"
            );
        }
    })
}

fn agent_error_json(message: &str) -> String {
    serde_json::json!({ "type": "error", "message": message }).to_string()
}

async fn send_agent_error(
    client: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &str,
) {
    let _ = client
        .send(Message::Text(agent_error_json(message).into()))
        .await;
}

async fn run_agent_ws_bridge(
    ctx: TerminalApiContext,
    q: AgentProjQuery,
    path_session_id: String,
    client: WebSocket,
) -> Result<(), String> {
    let proj_id = q.proj_id;
    let worker_session_id = ovs_worker_session_id(proj_id, &path_session_id);
    let record_session_id =
        ovs_record_session_id(proj_id, &path_session_id, q.chat_session_id.as_deref());
    let ovs_chat_key = q
        .chat_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("ovs-chat-"))
        .map(str::to_string);

    let (mut cli_tx, mut cli_rx) = client.split();
    let active_turn: Arc<Mutex<Option<ActiveOvsTurn>>> = Arc::new(Mutex::new(None));
    let session_db = Arc::clone(&ctx.session_db);

    let active = match ensure_terminal_active(&ctx, proj_id, &worker_session_id).await {
        Ok(a) => a,
        Err(e) => {
            send_agent_error(&mut cli_tx, &e.message).await;
            return Err(e.message);
        }
    };

    let ttyd = active.ttyd.clone();
    let url = terminal_ws_connect_url(&ttyd);
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("ws request {url}: {e}"))?;
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static("tty"));
    let (upstream, _) = match connect_async(req).await {
        Ok(pair) => pair,
        Err(e) => {
            let msg = format!("connect ttyd {url}: {e}");
            send_agent_error(&mut cli_tx, &msg).await;
            return Err(msg);
        }
    };
    let (mut up_tx, mut up_rx) = upstream.split();

    let spawn_json = format!(r#"{{"columns":{TTYD_SPAWN_COLS},"rows":{TTYD_SPAWN_ROWS}}}"#);
    if let Err(e) = up_tx.send(WsMessage::Text(spawn_json.into())).await {
        let msg = format!("ttyd spawn: {e}");
        send_agent_error(&mut cli_tx, &msg).await;
        return Err(msg);
    }
    if let Err(e) = up_tx
        .send(WsMessage::Binary(
            ttyd_resize_frame(TTYD_SPAWN_COLS, TTYD_SPAWN_ROWS).into(),
        ))
        .await
    {
        let msg = format!("ttyd resize: {e}");
        send_agent_error(&mut cli_tx, &msg).await;
        return Err(msg);
    }

    let cli_tx = Arc::new(tokio::sync::Mutex::new(cli_tx));

    let cli_tx_up = Arc::clone(&cli_tx);
    let active_turn_up = Arc::clone(&active_turn);
    let session_db_up = Arc::clone(&session_db);
    let record_session_id_up = record_session_id.clone();
    let worker_session_id_up = worker_session_id.clone();
    let ovs_chat_key_up = ovs_chat_key.clone();
    let ctx_up = ctx.clone();
    let client_to_up = async move {
        while let Some(msg) = cli_rx.next().await {
            let msg = msg.map_err(|e| format!("client ws: {e}"))?;
            match msg {
                Message::Text(t) => {
                    let parsed: AgentClientMsg =
                        serde_json::from_str(&t).map_err(|e| format!("invalid agent json: {e}"))?;
                    match parsed {
                        AgentClientMsg::Spawn => {
                            up_tx
                                .send(WsMessage::Text(
                                    format!(
                                        r#"{{"columns":{TTYD_SPAWN_COLS},"rows":{TTYD_SPAWN_ROWS}}}"#
                                    )
                                    .into(),
                                ))
                                .await
                                .map_err(|e| format!("ttyd respawn: {e}"))?;
                        }
                        AgentClientMsg::Prompt { text } => {
                            if text.is_empty() {
                                continue;
                            }
                            {
                                let mut guard = active_turn_up.lock().await;
                                finalize_active_turn(&session_db_up, &mut guard, "failed").await;
                            }
                            ensure_ovs_chat_record_session(&ctx_up, proj_id, &record_session_id_up)
                                .await?;
                            let turn_id = start_ovs_turn(
                                &session_db_up,
                                proj_id,
                                &record_session_id_up,
                                &worker_session_id_up,
                                ovs_chat_key_up.as_deref(),
                                &text,
                            )
                            .await?;
                            *active_turn_up.lock().await = Some(ActiveOvsTurn {
                                turn_id,
                                buffer: String::new(),
                            });
                            up_tx
                                .send(WsMessage::Binary(ttyd_input_frame(&text).into()))
                                .await
                                .map_err(|e| format!("ttyd prompt: {e}"))?;
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        Ok::<(), String>(())
    };

    let active_turn_down = Arc::clone(&active_turn);
    let session_db_down = Arc::clone(&session_db);
    let up_to_client = async move {
        let mut carry = String::new();
        while let Some(msg) = up_rx.next().await {
            let msg = msg.map_err(|e| format!("upstream ws: {e}"))?;
            let payload = match msg {
                WsMessage::Binary(b) => {
                    if b.is_empty() {
                        continue;
                    }
                    let kind = b[0];
                    if kind != 0x30 {
                        continue;
                    }
                    String::from_utf8_lossy(&b[1..]).into_owned()
                }
                WsMessage::Text(t) => t.to_string(),
                WsMessage::Close(_) => break,
                _ => continue,
            };
            carry.push_str(&payload);
            let (frames, clean) = extract_cdp_frames(&carry);
            carry = clean;
            let mut guard = cli_tx_up.lock().await;
            for ev in frames {
                if let Some(text) = ev.get("text").and_then(|v| v.as_str()) {
                    if ev.get("ev").and_then(|v| v.as_str()) == Some("content.delta")
                        && !text.is_empty()
                    {
                        let mut turn_guard = active_turn_down.lock().await;
                        if let Some(active) = turn_guard.as_mut() {
                            active.buffer.push_str(text);
                        }
                    }
                }
                if let (Some("status"), Some(phase)) = (
                    ev.get("ev").and_then(|v| v.as_str()),
                    ev.get("phase").and_then(|v| v.as_str()),
                ) {
                    if phase == "done" {
                        let mut turn_guard = active_turn_down.lock().await;
                        finalize_active_turn(&session_db_down, &mut turn_guard, "succeeded").await;
                    } else if phase == "failed" {
                        let mut turn_guard = active_turn_down.lock().await;
                        finalize_active_turn(&session_db_down, &mut turn_guard, "failed").await;
                    }
                }
                let body = serde_json::json!({ "type": "cdp", "event": ev });
                guard
                    .send(Message::Text(
                        serde_json::to_string(&body)
                            .map_err(|e| format!("serialize cdp: {e}"))?
                            .into(),
                    ))
                    .await
                    .map_err(|e| format!("client send: {e}"))?;
            }
        }
        Ok::<(), String>(())
    };

    let result = tokio::select! {
        r = client_to_up => r,
        r = up_to_client => r,
    };
    {
        let mut guard = active_turn.lock().await;
        let status = if result.is_ok() {
            "cancelled"
        } else {
            "failed"
        };
        finalize_active_turn(&session_db, &mut guard, status).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ovs_worker_session_id_normalizes_ovs_paths() {
        assert_eq!(ovs_worker_session_id(2, "ovs-2"), "ovs-2");
        assert_eq!(ovs_worker_session_id(2, "ovs-2-chat-foo"), "ovs-2");
        assert_eq!(ovs_worker_session_id(2, "coding-abc"), "coding-abc");
    }

    #[test]
    fn ovs_record_session_id_prefers_chat_query() {
        assert_eq!(
            ovs_record_session_id(1, "ovs-1", Some("panel-a")),
            "ovs-chat-1-panel-a"
        );
        assert_eq!(
            ovs_record_session_id(1, "ovs-1", Some("ovs-chat-1-custom")),
            "ovs-chat-1-custom"
        );
        assert_eq!(ovs_record_session_id(1, "ovs-1", None), "ovs-1");
    }
}
