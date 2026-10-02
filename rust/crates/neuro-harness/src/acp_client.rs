//! One ACP turn against an engine subprocess: initialize → new/resume/load → prompt.
//! Author: kejiqing

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock as AcpBlock, InitializeRequest, LoadSessionRequest, McpServer,
    NewSessionRequest, PermissionOptionKind, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionId, SessionNotification, SessionUpdate,
    StopReason,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, LineDirection};
use tokio::sync::mpsc;

use crate::mapper::{token_usage, OutEvent, TurnMapper, TurnTranscript};
use crate::profile::{AgentLaunch, EngineProfile, TurnSignal};
use crate::state::{write_state, HarnessState};
use crate::HarnessError;

pub struct TurnSpec {
    pub profile: &'static dyn EngineProfile,
    pub launch: AgentLaunch,
    pub session_root: PathBuf,
    pub mcp_servers: Vec<McpServer>,
    pub prompt: Vec<AcpBlock>,
    pub max_iterations: Option<usize>,
    /// Agent session to continue (`.neuro-harness/state.json`); `None` → `session/new`.
    pub resume_session_id: Option<String>,
    pub mapper: TurnMapper,
    pub trace_path: PathBuf,
}

#[derive(Debug)]
pub struct TurnOutcome {
    pub completion_reason: &'static str,
    pub usage: Option<runtime::TokenUsage>,
    pub transcript: TurnTranscript,
}

enum Inbound {
    Update(Box<SessionUpdate>),
    PromptDone(Box<Result<PromptResponse, agent_client_protocol::Error>>),
}

/// Run one turn. `on_event` receives contract events in ACP order.
pub async fn run_turn(
    spec: TurnSpec,
    mut on_event: impl FnMut(&OutEvent) + Send + 'static,
) -> Result<TurnOutcome, HarnessError> {
    let TurnSpec {
        profile,
        launch,
        session_root,
        mcp_servers,
        prompt,
        max_iterations,
        resume_session_id,
        mut mapper,
        trace_path,
    } = spec;
    let engine = profile.engine();

    let agent = AcpAgent::new(
        AcpAgentConfig::new(launch.command.clone())
            .args(launch.args.clone())
            .envs(launch.env.clone()),
    )
    .with_debug(trace_writer(engine, &trace_path)?);

    let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();
    let tx_updates = tx.clone();

    let result = Client
        .builder()
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                let _ = tx_updates.send(Inbound::Update(Box::new(n.update)));
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, _cx| {
                responder.respond(RequestPermissionResponse::new(allow_outcome(&req)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async move |cx: ConnectionTo<Agent>| {
            let session_id =
                open_session(&cx, engine, &session_root, mcp_servers, resume_session_id).await?;
            // Updates queued so far belong to setup (session/load replays history): drop them.
            while rx.try_recv().is_ok() {}

            let prompt_cx = cx.clone();
            let prompt_req = PromptRequest::new(session_id.clone(), prompt);
            cx.spawn(async move {
                let r = prompt_cx.send_request(prompt_req).block_task().await;
                let _ = tx.send(Inbound::PromptDone(Box::new(r)));
                Ok(())
            })?;

            let mut cancelled_for_iterations = false;
            let mut failure = EngineFailure::default();
            loop {
                match rx.recv().await {
                    Some(Inbound::Update(update)) => {
                        failure.observe(profile.turn_signal(&update));
                        for ev in mapper.on_update(&update) {
                            on_event(&ev);
                        }
                        let over = max_iterations.is_some_and(|max| mapper.tool_calls() > max);
                        if over && !cancelled_for_iterations {
                            cancelled_for_iterations = true;
                            cx.send_notification(CancelNotification::new(session_id.clone()))?;
                        }
                    }
                    Some(Inbound::PromptDone(r)) => {
                        let resp = (*r)?;
                        return Ok((resp, mapper, cancelled_for_iterations, failure, on_event));
                    }
                    None => return Err(acp_internal("ACP connection closed during prompt")),
                }
            }
        })
        .await;

    let (resp, mapper, cancelled_for_iterations, failure, mut on_event) =
        result.map_err(|e| HarnessError::internal(format!("acp: {}", describe_acp_error(&e))))?;
    if let Some(err) = failure.into_error(engine) {
        return Err(err);
    }

    let completion_reason = match resp.stop_reason {
        StopReason::EndTurn => "model_end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Cancelled if cancelled_for_iterations => "max_iterations",
        other => {
            return Err(HarnessError::internal(format!(
                "engine stopped the turn: {other:?}"
            )));
        }
    };
    let usage = resp.usage.as_ref().map(token_usage);
    let (transcript, tail) = mapper.finish(usage);
    for ev in &tail {
        on_event(ev);
    }
    Ok(TurnOutcome {
        completion_reason,
        usage,
        transcript,
    })
}

/// Folds [`TurnSignal`]s of one turn; `Failed` wins over the prompt's stop reason.
#[derive(Debug, Default)]
struct EngineFailure {
    detail: Option<String>,
    failed: bool,
}

impl EngineFailure {
    fn observe(&mut self, signal: Option<TurnSignal>) {
        match signal {
            Some(TurnSignal::ErrorDetail(d)) => self.detail = Some(d),
            Some(TurnSignal::Failed) => self.failed = true,
            None => {}
        }
    }

    fn into_error(self, engine: &str) -> Option<HarnessError> {
        self.failed.then(|| {
            let detail = self.detail.unwrap_or_else(|| "no error detail".into());
            HarnessError::new(502, format!("{engine} turn failed: {detail}"))
        })
    }
}

/// initialize → `session/new` (persisting the id) or `session/resume`, else `session/load`.
async fn open_session(
    cx: &ConnectionTo<Agent>,
    engine: &str,
    session_root: &Path,
    mcp_servers: Vec<McpServer>,
    resume_session_id: Option<String>,
) -> Result<SessionId, agent_client_protocol::Error> {
    let init = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let caps = init.agent_capabilities;
    let cwd = session_root.to_path_buf();
    let Some(id) = resume_session_id else {
        let resp = cx
            .send_request(NewSessionRequest::new(cwd).mcp_servers(mcp_servers))
            .block_task()
            .await?;
        write_state(
            session_root,
            &HarnessState {
                engine: engine.to_string(),
                agent_session_id: resp.session_id.0.to_string(),
            },
        )
        .map_err(|e| acp_internal(e.message))?;
        return Ok(resp.session_id);
    };
    let sid = SessionId::new(id);
    if caps.session_capabilities.resume.is_some() {
        cx.send_request(ResumeSessionRequest::new(sid.clone(), cwd).mcp_servers(mcp_servers))
            .block_task()
            .await?;
    } else if caps.load_session {
        cx.send_request(LoadSessionRequest::new(sid.clone(), cwd).mcp_servers(mcp_servers))
            .block_task()
            .await?;
    } else {
        return Err(acp_internal(format!(
            "engine {engine} supports neither session/resume nor session/load"
        )));
    }
    Ok(sid)
}

fn allow_outcome(req: &RequestPermissionRequest) -> RequestPermissionOutcome {
    let pick = |kind: PermissionOptionKind| req.options.iter().find(|o| o.kind == kind);
    pick(PermissionOptionKind::AllowOnce)
        .or_else(|| pick(PermissionOptionKind::AllowAlways))
        .or_else(|| req.options.first())
        .map_or(RequestPermissionOutcome::Cancelled, |o| {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o.option_id.clone()))
        })
}

fn acp_internal(message: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(serde_json::Value::String(message.into()))
}

fn describe_acp_error(e: &agent_client_protocol::Error) -> String {
    match &e.data {
        Some(serde_json::Value::String(s)) => format!("{}: {s}", e.message),
        Some(other) => format!("{}: {other}", e.message),
        None => e.message.clone(),
    }
}

/// Raw ACP stream → `.neuro-harness/acp-trace.ndjson` (truncated per turn); engine stderr →
/// our stderr (worker log).
fn trace_writer(
    engine: &'static str,
    path: &Path,
) -> Result<impl Fn(&str, LineDirection) + Send + Sync + 'static, HarnessError> {
    let file = std::fs::File::create(path)
        .map_err(|e| HarnessError::internal(format!("create {}: {e}", path.display())))?;
    let file = Arc::new(Mutex::new(file));
    Ok(move |line: &str, dir: LineDirection| {
        let dir = match dir {
            LineDirection::Stdin => "out",
            LineDirection::Stdout => "in",
            LineDirection::Stderr => {
                eprintln!("[{engine}] {line}");
                "stderr"
            }
        };
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let rec = serde_json::json!({"dir": dir, "t": t, "line": line});
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{rec}");
        }
    })
}
