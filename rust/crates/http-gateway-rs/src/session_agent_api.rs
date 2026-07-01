//! Agent WebSocket bridge for OVS `@claw` Chat (JSON + CDP via e2b exec + per-record jsonl). Author: kejiqing

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::warn;

use crate::client_origin::CLIENT_ORIGIN_OVS_CHAT;
use crate::persistence::transcript;
use crate::persistence::transcript::{
    import_turn_messages_to_db, report_body_from_turn_messages,
    turn_message_groups_from_jsonl_contents,
};
use crate::pool::interactive_backend::{
    interactive_backend_is_e2b, InteractiveBackendKind, E2B_INTERACTIVE_POOL_ID,
};
use crate::pool::{gateway_session_home, nas_cluster_id};
use crate::session_db::GatewaySessionDb;
use crate::session_ovs_api::{ovs_agent_session_id, ovs_chat_record_session_id};
use crate::session_terminal_api::{
    ensure_terminal_active, ActiveTerminalSession, TerminalApiContext, TerminalApiError,
};
use crate::turn_id;
use claw_e2b_sandbox_client::E2bSandboxHandle;
use gateway_solve_turn::{
    build_ovs_interactive_prompt_script, build_write_gateway_record_session_script,
    ovs_interactive_jsonl_host,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProjQuery {
    pub proj_id: i64,
    /// OVS Chat panel id for `gateway_turns` only; worker REPL stays `ovs-{projId}`.
    pub chat_session_id: Option<String>,
}

const OSC_PREFIX: &str = "\x1b]1337;Claw;";
const OSC_SUFFIX: char = '\x07';

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

/// Per-`record_session_id` exec lock — concurrent prompts on same record return 409. Author: kejiqing
static RECORD_EXEC_LOCKS: std::sync::OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    std::sync::OnceLock::new();

fn record_exec_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    RECORD_EXEC_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn try_acquire_record_exec(
    record_session_id: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let map = record_exec_locks();
    let slot = {
        let mut guard = map.lock().await;
        guard
            .entry(record_session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    slot.try_lock_owned()
        .map_err(|_| "previous turn still running for this chat session".to_string())
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
    let session_home = gateway_session_home(&ctx.work_root, proj_id, record_session_id)?;
    tokio::fs::create_dir_all(session_home.join(".claw"))
        .await
        .map_err(|e| format!("mkdir chat record session: {e}"))?;
    let session_home_rel =
        crate::session_merge::session_home_rel_under_work_root(&ctx.work_root, &session_home)
            .map_err(|e| e.detail().to_string())?;
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

fn ovs_turn_pool_id(active: &ActiveTerminalSession) -> &str {
    if active.backend == InteractiveBackendKind::E2b {
        E2B_INTERACTIVE_POOL_ID
    } else {
        active.pool_id.as_str()
    }
}

fn ovs_turn_exec_user(active: &ActiveTerminalSession) -> &'static str {
    if active.backend == InteractiveBackendKind::E2b {
        "0:0"
    } else {
        "claw"
    }
}

/// Reconstruct [`E2bSandboxHandle`] from an active interactive session. Author: kejiqing
fn fc_exec_handle_from_active(active: &ActiveTerminalSession) -> Result<E2bSandboxHandle, String> {
    let sandbox_id = active
        .e2b_sandbox_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "fc interactive: missing sandbox id".to_string())?;
    let host = active
        .ttyd
        .proxy_host_header
        .as_deref()
        .filter(|h| !h.is_empty())
        .or({
            if active.ttyd.use_tls && !active.ttyd.host.is_empty() {
                Some(active.ttyd.host.as_str())
            } else {
                None
            }
        })
        .ok_or_else(|| "fc interactive: missing ttyd public host".to_string())?;
    let rest = host
        .split_once('-')
        .map(|(_, after)| after)
        .ok_or_else(|| format!("fc interactive: invalid ttyd host {host}"))?;
    let domain = rest
        .strip_prefix(sandbox_id)
        .and_then(|tail| tail.strip_prefix('.'))
        .ok_or_else(|| format!("fc interactive: sandbox {sandbox_id} not in ttyd host {host}"))?;
    Ok(E2bSandboxHandle {
        sandbox_id: sandbox_id.to_string(),
        sandbox_domain: domain.to_string(),
        envd_access_token: None,
        traffic_access_token: active.ttyd.traffic_access_token.clone(),
        ttyd_public_host: host.to_string(),
        ttyd_use_tls: active.ttyd.use_tls,
    })
}

async fn fc_handle_for_active(
    ctx: &TerminalApiContext,
    active: &ActiveTerminalSession,
) -> Result<E2bSandboxHandle, String> {
    if let Some(proj_id) = active.e2b_warm_proj_id {
        let reg = ctx.pool_clients.e2b_worker_registry();
        if let Some(handle) = reg.leased_handle(proj_id).await {
            return Ok(handle);
        }
    }
    fc_exec_handle_from_active(active)
}

/// Stage dialogue `record_session_id` on worker before exec (tap reads `claw-session-id` from LLM). Author: kejiqing
async fn stage_gateway_record_session_id(
    ctx: &TerminalApiContext,
    active: &ActiveTerminalSession,
    record_session_id: &str,
) -> Result<(), String> {
    if !interactive_backend_is_e2b() {
        return Ok(());
    }
    let script = build_write_gateway_record_session_script(record_session_id);
    let handle = fc_handle_for_active(ctx, active).await?;
    let client = ctx
        .pool_clients
        .e2b_sandbox_client()
        .ok_or_else(|| "fc interactive: sandbox client not configured".to_string())?;
    client.exec_shell_script(&handle, &script).await
}

async fn assign_ovs_turn_pool_worker(
    db: &GatewaySessionDb,
    turn_id: &str,
    active: &ActiveTerminalSession,
) {
    let Some(worker_name) = active.worker_name.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    if let Err(e) = db
        .assign_turn_pool_worker(
            turn_id,
            ovs_turn_pool_id(active),
            worker_name,
            Some(ovs_turn_exec_user(active)),
        )
        .await
    {
        warn!(
            target: "claw_gateway_agent",
            turn_id = %turn_id,
            pool_id = ovs_turn_pool_id(active),
            worker_name = %worker_name,
            error = %e,
            "assign ovs-chat turn pool/worker failed"
        );
    }
}

async fn import_interactive_turn_from_jsonl(
    db: &GatewaySessionDb,
    work_root: &std::path::Path,
    proj_id: i64,
    record_session_id: &str,
    turn_id: &str,
) -> Result<(), String> {
    let seg = crate::session_merge::sessions_directory_segment(record_session_id);
    let cluster_id = nas_cluster_id()?;
    let path = ovs_interactive_jsonl_host(work_root, &cluster_id, proj_id, &seg);
    let contents = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read interactive jsonl {}: {e}", path.display()))?;
    let groups = turn_message_groups_from_jsonl_contents(&contents);
    let Some(messages) = groups.last() else {
        return Ok(());
    };
    if messages.is_empty() {
        return Ok(());
    }
    let now = transcript::now_ms();
    import_turn_messages_to_db(db, record_session_id, proj_id, turn_id, messages, now)
        .await
        .map_err(|e| format!("import interactive turn messages: {e}"))?;
    Ok(())
}

async fn send_cdp_event(
    cli_tx: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    ev: &serde_json::Value,
) -> Result<(), String> {
    let body = serde_json::json!({ "type": "cdp", "event": ev });
    let mut guard = cli_tx.lock().await;
    guard
        .send(Message::Text(
            serde_json::to_string(&body)
                .map_err(|e| format!("serialize cdp: {e}"))?
                .into(),
        ))
        .await
        .map_err(|e| format!("client send: {e}"))?;
    Ok(())
}

async fn process_exec_stdout_chunk(
    carry: &mut String,
    chunk: &str,
    cli_tx: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    active_turn: &Arc<Mutex<Option<ActiveOvsTurn>>>,
    session_db: &Arc<GatewaySessionDb>,
) {
    carry.push_str(chunk);
    let (frames, clean) = extract_cdp_frames(carry);
    *carry = clean;
    for ev in frames {
        if let Some(text) = ev.get("text").and_then(|v| v.as_str()) {
            if ev.get("ev").and_then(|v| v.as_str()) == Some("content.delta") && !text.is_empty() {
                let mut turn_guard = active_turn.lock().await;
                if let Some(active) = turn_guard.as_mut() {
                    active.buffer.push_str(text);
                }
            }
        }
        if let (Some("status"), Some(phase)) = (
            ev.get("ev").and_then(|v| v.as_str()),
            ev.get("phase").and_then(|v| v.as_str()),
        ) {
            if phase == "failed" {
                let mut turn_guard = active_turn.lock().await;
                finalize_active_turn(session_db, &mut turn_guard, "failed").await;
            }
        }
        if let Err(e) = send_cdp_event(cli_tx, &ev).await {
            warn!(target: "claw_gateway_agent", error = %e, "send cdp to client failed");
        }
    }
}

async fn run_ovs_interactive_prompt(
    ctx: &TerminalApiContext,
    proj_id: i64,
    record_session_id: &str,
    worker_session_id: &str,
    ovs_chat_key: Option<&str>,
    active: &ActiveTerminalSession,
    text: &str,
    cli_tx: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    active_turn: &Arc<Mutex<Option<ActiveOvsTurn>>>,
) -> Result<(), String> {
    let _exec_guard = try_acquire_record_exec(record_session_id).await?;

    {
        let mut guard = active_turn.lock().await;
        finalize_active_turn(&ctx.session_db, &mut guard, "failed").await;
    }

    ensure_ovs_chat_record_session(ctx, proj_id, record_session_id).await?;
    let turn_id = start_ovs_turn(
        &ctx.session_db,
        proj_id,
        record_session_id,
        worker_session_id,
        ovs_chat_key,
        text,
    )
    .await?;
    assign_ovs_turn_pool_worker(&ctx.session_db, &turn_id, active).await;
    stage_gateway_record_session_id(ctx, active, record_session_id).await?;

    *active_turn.lock().await = Some(ActiveOvsTurn {
        turn_id: turn_id.clone(),
        buffer: String::new(),
    });

    let segment = crate::session_merge::sessions_directory_segment(record_session_id);
    if ctx.pool_clients.e2b_nas_layout_active() {
        let worker_id = active
            .e2b_worker_id
            .as_deref()
            .ok_or_else(|| "ovs interactive: missing fc worker id".to_string())?;
        ctx.pool_clients
            .nas_layout()
            .ensure_session_context(proj_id, &segment, worker_id)
            .await
            .map_err(|e| format!("ensure ovs session root: {e}"))?;
    }
    let script = build_ovs_interactive_prompt_script(&segment, record_session_id, text);
    let handle = fc_handle_for_active(ctx, active).await?;
    let client = ctx
        .pool_clients
        .e2b_sandbox_client()
        .ok_or_else(|| "fc interactive: sandbox client not configured".to_string())?;

    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let hook = {
        let line_tx = line_tx.clone();
        Arc::new(move |chunk: String| {
            let _ = line_tx.send(chunk);
        })
    };

    let cli_tx_stream = Arc::clone(cli_tx);
    let active_turn_stream = Arc::clone(active_turn);
    let session_db_stream = Arc::clone(&ctx.session_db);
    let mut carry = String::new();
    let pump = tokio::spawn(async move {
        while let Some(chunk) = line_rx.recv().await {
            process_exec_stdout_chunk(
                &mut carry,
                &chunk,
                &cli_tx_stream,
                &active_turn_stream,
                &session_db_stream,
            )
            .await;
        }
        carry
    });

    let outcome = client
        .exec_shell_script_streaming(&handle, &script, Some(hook))
        .await?;
    drop(line_tx);
    let mut tail_carry = pump.await.map_err(|e| format!("stdout pump join: {e}"))?;
    if !tail_carry.is_empty() {
        process_exec_stdout_chunk(&mut tail_carry, "", cli_tx, active_turn, &ctx.session_db).await;
    }

    if outcome.exit_code != 0 {
        let mut guard = active_turn.lock().await;
        finalize_active_turn(&ctx.session_db, &mut guard, "failed").await;
        return Err(format!(
            "ovs interactive exec exit {}: {}",
            outcome.exit_code,
            outcome.stderr.trim()
        ));
    }

    if let Err(e) = import_interactive_turn_from_jsonl(
        &ctx.session_db,
        &ctx.work_root,
        proj_id,
        record_session_id,
        &turn_id,
    )
    .await
    {
        warn!(
            target: "claw_gateway_agent",
            turn_id = %turn_id,
            error = %e,
            "import interactive turn to cc_messages failed"
        );
    }

    let mut guard = active_turn.lock().await;
    if let Some(active_turn_state) = guard.as_mut() {
        if active_turn_state.buffer.trim().is_empty() {
            let seg = crate::session_merge::sessions_directory_segment(record_session_id);
            let cluster_id = nas_cluster_id().unwrap_or_default();
            let path = ovs_interactive_jsonl_host(&ctx.work_root, &cluster_id, proj_id, &seg);
            if let Ok(contents) = tokio::fs::read_to_string(&path).await {
                let groups = turn_message_groups_from_jsonl_contents(&contents);
                if let Some(messages) = groups.last() {
                    active_turn_state.buffer = report_body_from_turn_messages(messages);
                }
            }
        }
    }
    finalize_active_turn(&ctx.session_db, &mut guard, "succeeded").await;
    Ok(())
}

#[allow(clippy::unused_async)]
pub fn agent_ws_upgrade(
    ctx: TerminalApiContext,
    session_id: &str,
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

    let active = match ensure_terminal_active(&ctx, proj_id, &worker_session_id).await {
        Ok(a) => a,
        Err(e) => {
            send_agent_error(&mut cli_tx, &e.message).await;
            return Err(e.message);
        }
    };

    let cli_tx = Arc::new(tokio::sync::Mutex::new(cli_tx));
    let active_terminal = active.clone();

    while let Some(msg) = cli_rx.next().await {
        let msg = msg.map_err(|e| format!("client ws: {e}"))?;
        match msg {
            Message::Text(t) => {
                let parsed: AgentClientMsg =
                    serde_json::from_str(&t).map_err(|e| format!("invalid agent json: {e}"))?;
                match parsed {
                    AgentClientMsg::Spawn => {
                        // Legacy no-op: context is per-record jsonl + exec, not ttyd respawn.
                    }
                    AgentClientMsg::Prompt { text } => {
                        if text.is_empty() {
                            continue;
                        }
                        if let Err(e) = run_ovs_interactive_prompt(
                            &ctx,
                            proj_id,
                            &record_session_id,
                            &worker_session_id,
                            ovs_chat_key.as_deref(),
                            &active_terminal,
                            &text,
                            &cli_tx,
                            &active_turn,
                        )
                        .await
                        {
                            let mut guard = cli_tx.lock().await;
                            send_agent_error(&mut guard, &e).await;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    {
        let mut guard = active_turn.lock().await;
        finalize_active_turn(&ctx.session_db, &mut guard, "cancelled").await;
    }
    Ok(())
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

    #[test]
    fn fc_exec_handle_from_ttyd_host_parses_domain() {
        use crate::pool::interactive_backend::TtydConnectTarget;

        let active = ActiveTerminalSession {
            slot_index: 0,
            worker_name: Some("e2b:sbx_abc".into()),
            ttyd_host_port: 80,
            pool_id: E2B_INTERACTIVE_POOL_ID.into(),
            backend: InteractiveBackendKind::E2b,
            e2b_sandbox_id: Some("sbx_abc".into()),
            e2b_warm_slot: Some(1),
            e2b_warm_proj_id: Some(3),
            e2b_session_segment: None,
            e2b_worker_id: None,
            ttyd: TtydConnectTarget::e2b_self_hosted_proxy(
                "10.8.0.1".into(),
                80,
                "7681-sbx_abc.supone.top".into(),
                None,
            ),
        };
        let h = fc_exec_handle_from_active(&active).expect("handle");
        assert_eq!(h.sandbox_id, "sbx_abc");
        assert_eq!(h.sandbox_domain, "supone.top");
    }

    #[tokio::test]
    async fn record_exec_lock_rejects_concurrent_same_record() {
        let _a = try_acquire_record_exec("ovs-chat-1-x")
            .await
            .expect("first");
        let err = try_acquire_record_exec("ovs-chat-1-x")
            .await
            .expect_err("second");
        assert!(err.contains("previous turn still running"));
        let _b = try_acquire_record_exec("ovs-chat-1-y")
            .await
            .expect("other record");
    }
}
