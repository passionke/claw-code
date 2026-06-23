//! Agent WebSocket bridge for OVS `@claw` Chat (exec + resume jsonl + CDP). Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, LazyLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use claw_sandbox_protocol::GuestExecActor;
use futures_util::{SinkExt, StreamExt};
use gateway_solve_turn::{
    build_ovs_interactive_prompt_script, build_write_gateway_record_session_script,
    ovs_interactive_guest_symlink_host, ovs_interactive_session_jsonl_host,
    ovs_interactive_symlink_target,
};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

use crate::client_origin::CLIENT_ORIGIN_OVS_CHAT;
use crate::persistence::transcript::{
    self, import_turn_messages_to_db, turn_message_groups_from_jsonl_contents,
};
use crate::pool::interactive_backend::{
    interactive_backend_is_fc, InteractiveBackendKind, FC_INTERACTIVE_POOL_ID,
};
use crate::pool::nas_host_root;
use crate::session_db::GatewaySessionDb;
use crate::session_ovs_api::{ovs_agent_session_id, ovs_chat_record_session_id};
use crate::session_terminal_api::{
    ensure_terminal_active, ActiveTerminalSession, TerminalApiContext, TerminalApiError,
};
use crate::turn_id;
use claw_fc_sandbox_client::FcSandboxHandle;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProjQuery {
    pub proj_id: i64,
    /// OVS Chat panel id for `gateway_turns` only; worker REPL stays `ovs-{projId}`.
    pub chat_session_id: Option<String>,
}

const OSC_PREFIX: &str = "\x1b]1337;Claw;";
const OSC_SUFFIX: char = '\x07';
const OVS_AGENT_BUSY_CODE: u16 = 409;

static OVS_AGENT_BUSY: LazyLock<Mutex<HashMap<String, ()>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

async fn try_acquire_ovs_record(record_session_id: &str) -> Result<(), String> {
    let mut guard = OVS_AGENT_BUSY.lock().await;
    if guard.contains_key(record_session_id) {
        return Err(format!(
            "OVS agent turn already in progress ({OVS_AGENT_BUSY_CODE})"
        ));
    }
    guard.insert(record_session_id.to_string(), ());
    Ok(())
}

async fn release_ovs_record(record_session_id: &str) {
    let mut guard = OVS_AGENT_BUSY.lock().await;
    guard.remove(record_session_id);
}

async fn import_interactive_turn_to_cc_messages(
    ctx: &TerminalApiContext,
    proj_id: i64,
    record_session_id: &str,
    turn_id: &str,
) -> Result<(), String> {
    let segment = crate::session_merge::sessions_directory_segment(record_session_id);
    let nas_root = nas_host_root(&ctx.work_root, ctx.pool_rpc_host_work_root.as_deref());
    let path = ovs_interactive_session_jsonl_host(&nas_root, proj_id, &segment);
    let contents = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read ovs interactive jsonl {}: {e}", path.display()))?;
    let groups = turn_message_groups_from_jsonl_contents(&contents);
    let messages = groups
        .last()
        .ok_or_else(|| "ovs interactive jsonl: no message groups".to_string())?;
    let now = transcript::now_ms();
    import_turn_messages_to_db(
        &ctx.session_db,
        record_session_id,
        proj_id,
        turn_id,
        messages,
        now,
    )
    .await
    .map_err(|e| format!("import ovs interactive cc_messages: {e}"))
}

async fn finalize_active_turn(
    ctx: &TerminalApiContext,
    proj_id: i64,
    record_session_id: &str,
    active: &mut Option<ActiveOvsTurn>,
    status: &str,
) {
    let Some(turn) = active.take() else {
        return;
    };
    if status == "succeeded" {
        if let Err(e) =
            import_interactive_turn_to_cc_messages(ctx, proj_id, record_session_id, &turn.turn_id)
                .await
        {
            warn!(
                target: "claw_gateway_agent",
                turn_id = %turn.turn_id,
                error = %e,
                "import ovs interactive cc_messages failed"
            );
        }
    }
    let finished_at = transcript::now_ms();
    let report = if turn.buffer.trim().is_empty() {
        None
    } else {
        Some(turn.buffer.as_str())
    };
    if let Err(e) = ctx
        .session_db
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

async fn ensure_ovs_interactive_guest_symlink(
    nas_root: &Path,
    proj_id: i64,
    segment: &str,
) -> Result<(), String> {
    let link_path = ovs_interactive_guest_symlink_host(nas_root, proj_id, segment);
    let target = ovs_interactive_symlink_target(segment);
    if let Some(parent) = link_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir ovs interactive symlink parent: {e}"))?;
    }
    if link_path.is_symlink() {
        let current = tokio::fs::read_link(&link_path)
            .await
            .map_err(|e| format!("read ovs interactive symlink: {e}"))?;
        if current == Path::new(&target) {
            return Ok(());
        }
        tokio::fs::remove_file(&link_path)
            .await
            .map_err(|e| format!("replace ovs interactive symlink: {e}"))?;
    } else if link_path.exists() {
        let meta = tokio::fs::symlink_metadata(&link_path)
            .await
            .map_err(|e| format!("stat ovs interactive guest path: {e}"))?;
        if meta.is_dir() {
            return Ok(());
        }
        tokio::fs::remove_file(&link_path)
            .await
            .map_err(|e| format!("replace ovs interactive guest path: {e}"))?;
    }
    #[cfg(unix)]
    {
        tokio::fs::symlink(&target, &link_path)
            .await
            .map_err(|e| format!("symlink {} -> {target}: {e}", link_path.display()))?;
    }
    #[cfg(not(unix))]
    {
        return Err("ovs interactive guest symlink requires unix".into());
    }
    Ok(())
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
    let nas_root = nas_host_root(&ctx.work_root, ctx.pool_rpc_host_work_root.as_deref());
    let session_home = nas_root
        .join(format!("proj_{proj_id}"))
        .join("sessions")
        .join(&seg);
    tokio::fs::create_dir_all(session_home.join(".claw"))
        .await
        .map_err(|e| format!("mkdir chat record session: {e}"))?;
    ensure_ovs_interactive_guest_symlink(&nas_root, proj_id, &seg).await?;
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

fn ovs_turn_pool_id(active: &ActiveTerminalSession) -> &str {
    if active.backend == InteractiveBackendKind::Fc {
        FC_INTERACTIVE_POOL_ID
    } else {
        active.pool_id.as_str()
    }
}

fn ovs_turn_exec_user(active: &ActiveTerminalSession) -> Option<&'static str> {
    if active.backend == InteractiveBackendKind::Fc {
        Some("0:0")
    } else {
        Some("claw")
    }
}

/// Reconstruct [`FcSandboxHandle`] for cold-start workers (warm pool uses [`FcProjWarmPool::leased_handle`]). Author: kejiqing
fn fc_exec_handle_from_active(active: &ActiveTerminalSession) -> Result<FcSandboxHandle, String> {
    let sandbox_id = active
        .fc_sandbox_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "fc interactive: missing sandbox id".to_string())?;
    let host = active
        .ttyd
        .proxy_host_header
        .as_deref()
        .filter(|h| !h.is_empty())
        .or_else(|| {
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
    Ok(FcSandboxHandle {
        sandbox_id: sandbox_id.to_string(),
        sandbox_domain: domain.to_string(),
        envd_access_token: None,
        traffic_access_token: active.ttyd.traffic_access_token.clone(),
        ttyd_public_host: host.to_string(),
        ttyd_use_tls: active.ttyd.use_tls,
    })
}

async fn fc_exec_handle_for_active(
    ctx: &TerminalApiContext,
    active: &ActiveTerminalSession,
) -> Result<FcSandboxHandle, String> {
    if let Some(slot) = active.fc_warm_slot {
        if let Some(pool) = ctx.pool_clients.fc_warm_pool() {
            if let Some(handle) = pool.leased_handle(slot).await {
                return Ok(handle);
            }
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
    let script = build_write_gateway_record_session_script(record_session_id);
    if interactive_backend_is_fc() {
        let client = ctx
            .pool_clients
            .fc_sandbox_client()
            .ok_or_else(|| "fc interactive: sandbox client not configured".to_string())?;
        let handle = fc_exec_handle_for_active(ctx, active).await?;
        return client.exec_shell_script(&handle, &script).await;
    }
    let sandbox = ctx
        .pool_clients
        .sandbox_rpc_client()
        .ok_or_else(|| "podman interactive: sandbox rpc not configured".to_string())?;
    sandbox
        .guest_exec_sh(active.slot_index, &script, GuestExecActor::SlotWorker)
        .await
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
            ovs_turn_exec_user(active),
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

struct CdpRelay {
    carry: String,
    turn_buffer: Arc<Mutex<String>>,
    turn_done: Arc<Mutex<Option<String>>>,
}

impl CdpRelay {
    fn new(turn_buffer: Arc<Mutex<String>>) -> Self {
        Self {
            carry: String::new(),
            turn_buffer,
            turn_done: Arc::new(Mutex::new(None)),
        }
    }

    fn ingest_line(&mut self, line: &str, out: &mut Vec<serde_json::Value>) {
        self.carry.push_str(line);
        let (frames, clean) = extract_cdp_frames(&self.carry);
        self.carry = clean;
        for ev in frames {
            if let Some(text) = ev.get("text").and_then(|v| v.as_str()) {
                if ev.get("ev").and_then(|v| v.as_str()) == Some("content.delta")
                    && !text.is_empty()
                {
                    if let Ok(mut guard) = self.turn_buffer.try_lock() {
                        guard.push_str(text);
                    }
                }
            }
            if let (Some("status"), Some(phase)) = (
                ev.get("ev").and_then(|v| v.as_str()),
                ev.get("phase").and_then(|v| v.as_str()),
            ) {
                if phase == "done" || phase == "failed" {
                    if let Ok(mut guard) = self.turn_done.try_lock() {
                        *guard = Some(phase.to_string());
                    }
                }
            }
            out.push(ev);
        }
    }

    async fn turn_phase(&self) -> Option<String> {
        self.turn_done.lock().await.clone()
    }
}

async fn ovs_exec_prompt_script(
    ctx: &TerminalApiContext,
    active: &ActiveTerminalSession,
    script: &str,
    on_stdout_line: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<i32, String> {
    if interactive_backend_is_fc() {
        let client = ctx
            .pool_clients
            .fc_sandbox_client()
            .ok_or_else(|| "fc interactive: sandbox client not configured".to_string())?;
        let handle = fc_exec_handle_for_active(ctx, active).await?;
        let outcome = client
            .exec_shell_script_streaming(&handle, script, Some(on_stdout_line))
            .await?;
        return Ok(outcome.exit_code);
    }
    let sandbox = ctx
        .pool_clients
        .sandbox_rpc_client()
        .ok_or_else(|| "podman interactive: sandbox rpc not configured".to_string())?;
    let outcome = sandbox
        .exec_sh_streaming(
            active.slot_index,
            script,
            BTreeMap::new(),
            Some(on_stdout_line),
        )
        .await?;
    Ok(outcome.exit_code)
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

async fn send_cdp_events(
    client: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    events: &[serde_json::Value],
) -> Result<(), String> {
    for ev in events {
        let body = serde_json::json!({ "type": "cdp", "event": ev });
        client
            .send(Message::Text(
                serde_json::to_string(&body)
                    .map_err(|e| format!("serialize cdp: {e}"))?
                    .into(),
            ))
            .await
            .map_err(|e| format!("client send: {e}"))?;
    }
    Ok(())
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
    let active_terminal = match ensure_terminal_active(&ctx, proj_id, &worker_session_id).await {
        Ok(a) => a,
        Err(e) => {
            send_agent_error(&mut cli_tx, &e.message).await;
            return Err(e.message);
        }
    };

    let (cdp_tx, mut cdp_rx) = mpsc::unbounded_channel::<Vec<serde_json::Value>>();
    let mut exec_task: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        tokio::select! {
            cdp_batch = cdp_rx.recv() => {
                let Some(events) = cdp_batch else { break };
                send_cdp_events(&mut cli_tx, &events).await?;
            }
            msg = cli_rx.next() => {
                let Some(msg) = msg else { break };
                let msg = msg.map_err(|e| format!("client ws: {e}"))?;
                match msg {
                    Message::Text(t) => {
                        let parsed: AgentClientMsg = serde_json::from_str(&t)
                            .map_err(|e| format!("invalid agent json: {e}"))?;
                        match parsed {
                            AgentClientMsg::Spawn => {}
                            AgentClientMsg::Prompt { text } => {
                                if text.is_empty() {
                                    continue;
                                }
                                if exec_task.as_ref().is_some_and(|h| !h.is_finished()) {
                                    send_agent_error(&mut cli_tx, &format!(
                                        "OVS agent turn already in progress ({OVS_AGENT_BUSY_CODE})"
                                    )).await;
                                    continue;
                                }
                                if let Err(e) = try_acquire_ovs_record(&record_session_id).await {
                                    send_agent_error(&mut cli_tx, &e).await;
                                    continue;
                                }
                                {
                                    let mut guard = active_turn.lock().await;
                                    finalize_active_turn(
                                        &ctx,
                                        proj_id,
                                        &record_session_id,
                                        &mut guard,
                                        "failed",
                                    )
                                    .await;
                                }
                                if let Err(e) = ensure_ovs_chat_record_session(
                                    &ctx,
                                    proj_id,
                                    &record_session_id,
                                )
                                .await
                                {
                                    release_ovs_record(&record_session_id).await;
                                    send_agent_error(&mut cli_tx, &e).await;
                                    return Err(e);
                                }
                                let turn_id = match start_ovs_turn(
                                    &ctx.session_db,
                                    proj_id,
                                    &record_session_id,
                                    &worker_session_id,
                                    ovs_chat_key.as_deref(),
                                    &text,
                                )
                                .await
                                {
                                    Ok(id) => id,
                                    Err(e) => {
                                        release_ovs_record(&record_session_id).await;
                                        send_agent_error(&mut cli_tx, &e).await;
                                        return Err(e);
                                    }
                                };
                                assign_ovs_turn_pool_worker(
                                    &ctx.session_db,
                                    &turn_id,
                                    &active_terminal,
                                )
                                .await;
                                if let Err(e) = stage_gateway_record_session_id(
                                    &ctx,
                                    &active_terminal,
                                    &record_session_id,
                                )
                                .await
                                {
                                    release_ovs_record(&record_session_id).await;
                                    let mut guard = active_turn.lock().await;
                                    *guard = Some(ActiveOvsTurn {
                                        turn_id: turn_id.clone(),
                                        buffer: String::new(),
                                    });
                                    finalize_active_turn(
                                        &ctx,
                                        proj_id,
                                        &record_session_id,
                                        &mut guard,
                                        "failed",
                                    )
                                    .await;
                                    send_agent_error(&mut cli_tx, &e).await;
                                    return Err(e);
                                }
                                let segment =
                                    crate::session_merge::sessions_directory_segment(&record_session_id);
                                let script = build_ovs_interactive_prompt_script(&segment, &text);
                                let turn_buffer = Arc::new(Mutex::new(String::new()));
                                *active_turn.lock().await = Some(ActiveOvsTurn {
                                    turn_id: turn_id.clone(),
                                    buffer: String::new(),
                                });
                                let ctx_exec = ctx.clone();
                                let active_exec = active_terminal.clone();
                                let record_exec = record_session_id.clone();
                                let active_turn_exec = Arc::clone(&active_turn);
                                let turn_buffer_exec = Arc::clone(&turn_buffer);
                                let cdp_tx_exec = cdp_tx.clone();
                                exec_task = Some(tokio::spawn(async move {
                                    let relay = Arc::new(Mutex::new(CdpRelay::new(
                                        Arc::clone(&turn_buffer_exec),
                                    )));
                                    let hook: Arc<dyn Fn(String) + Send + Sync> = {
                                        let relay_hook = Arc::clone(&relay);
                                        let cdp_tx_hook = cdp_tx_exec.clone();
                                        Arc::new(move |line: String| {
                                            let mut batch = Vec::new();
                                            if let Ok(mut guard) = relay_hook.try_lock() {
                                                guard.ingest_line(&line, &mut batch);
                                            }
                                            if !batch.is_empty() {
                                                let _ = cdp_tx_hook.send(batch);
                                            }
                                        })
                                    };
                                    let exec_result = ovs_exec_prompt_script(
                                        &ctx_exec,
                                        &active_exec,
                                        &script,
                                        hook,
                                    )
                                    .await;
                                    let phase = relay.lock().await.turn_phase().await;
                                    let status = match (&exec_result, phase.as_deref()) {
                                        (Ok(code), Some("done")) if *code == 0 => "succeeded",
                                        (Ok(_), Some("done")) => "failed",
                                        (_, Some("failed")) => "failed",
                                        (Err(_), _) => "cancelled",
                                        (Ok(_), _) => "failed",
                                    };
                                    {
                                        let mut guard = active_turn_exec.lock().await;
                                        if let Some(active) = guard.as_mut() {
                                            let buf = turn_buffer_exec.lock().await;
                                            active.buffer = buf.clone();
                                        }
                                        finalize_active_turn(
                                            &ctx_exec,
                                            proj_id,
                                            &record_exec,
                                            &mut guard,
                                            status,
                                        )
                                        .await;
                                    }
                                    release_ovs_record(&record_exec).await;
                                    if let Err(e) = exec_result {
                                        let _ = cdp_tx_exec.send(vec![serde_json::json!({
                                            "ev": "status",
                                            "phase": "failed",
                                            "message": e,
                                        })]);
                                    }
                                }));
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    if let Some(task) = exec_task {
        task.abort();
        let _ = task.await;
    }
    {
        let mut guard = active_turn.lock().await;
        finalize_active_turn(&ctx, proj_id, &record_session_id, &mut guard, "cancelled").await;
    }
    release_ovs_record(&record_session_id).await;
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
            worker_name: Some("fc:sbx_abc".into()),
            ttyd_host_port: 80,
            pool_id: FC_INTERACTIVE_POOL_ID.into(),
            backend: InteractiveBackendKind::Fc,
            fc_sandbox_id: Some("sbx_abc".into()),
            fc_warm_slot: Some(1),
            fc_warm_proj_id: Some(3),
            fc_session_segment: None,
            fc_worker_id: None,
            ttyd: TtydConnectTarget::e2b_self_hosted_proxy(
                "10.8.0.9".into(),
                80,
                "7681-sbx_abc.supone.top".into(),
                None,
            ),
        };
        let h = fc_exec_handle_from_active(&active).expect("handle");
        assert_eq!(h.sandbox_id, "sbx_abc");
        assert_eq!(h.sandbox_domain, "supone.top");
    }

    #[test]
    fn cdp_relay_extracts_status_done() {
        let buf = Arc::new(Mutex::new(String::new()));
        let mut relay = CdpRelay::new(buf);
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"ev":"status","phase":"done"}"#);
        let osc = format!("{OSC_PREFIX}{encoded}{OSC_SUFFIX}");
        let mut out = Vec::new();
        relay.ingest_line(&osc, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("phase").and_then(|v| v.as_str()), Some("done"));
    }
}
