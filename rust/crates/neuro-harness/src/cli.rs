//! Shared entrypoint for every engine bin: `gateway-solve-once` and `mcp-proxy`.
//! Author: kejiqing

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    ContentBlock as AcpBlock, EnvVariable, McpServer, McpServerStdio, ResourceLink, TextContent,
};
use gateway_solve_turn::{GatewaySolveTaskFile, SolveAttachment};
use runtime::{ConfigLoader, Session};
use serde_json::{json, Value};

use crate::acp_client::{run_turn, TurnOutcome, TurnSpec};
use crate::mapper::{OutEvent, TurnMapper};
use crate::profile::{EngineProfile, PrepareContext};
use crate::projection::{instructions_path, render_instructions, write_file};
use crate::state::read_state;
use crate::task::{load_task, reject_unsupported, resolve_model};
use crate::turn_context::{write_turn_context, TurnContext};
use crate::{HarnessError, HARNESS_DIR};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_MAX_ITERATIONS: usize = 64;

enum Command {
    Solve(PathBuf),
    McpProxy {
        server: String,
        session_root: PathBuf,
    },
}

/// `main` of `neuro-<engine>` bins.
pub fn main_with_profile(profile: &dyn EngineProfile) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Solve(task_file)) => run_solve(profile, &task_file),
        Ok(Command::McpProxy {
            server,
            session_root,
        }) => run_mcp_proxy(&server, &session_root),
        Err(usage) => {
            eprintln!("{usage}");
            ExitCode::from(2)
        }
    }
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    let usage = "usage: <bin> gateway-solve-once [--task-file] PATH | <bin> mcp-proxy --server NAME --session-root DIR";
    match args.first().map(String::as_str) {
        Some("gateway-solve-once") => {
            let rest = &args[1..];
            let path = match rest {
                [flag, p] if flag == "--task-file" => p,
                [p] if !p.starts_with('-') => p,
                _ => return Err(usage.to_string()),
            };
            Ok(Command::Solve(PathBuf::from(path)))
        }
        Some("mcp-proxy") => {
            let mut server = None;
            let mut root = None;
            let mut it = args[1..].iter();
            while let Some(flag) = it.next() {
                match flag.as_str() {
                    "--server" => server = it.next().cloned(),
                    "--session-root" => root = it.next().map(PathBuf::from),
                    _ => return Err(usage.to_string()),
                }
            }
            match (server, root) {
                (Some(server), Some(session_root)) => Ok(Command::McpProxy {
                    server,
                    session_root,
                }),
                _ => Err(usage.to_string()),
            }
        }
        _ => Err(usage.to_string()),
    }
}

fn run_mcp_proxy(server: &str, session_root: &Path) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[mcp-proxy {server}] runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(crate::mcp_proxy::run(server, session_root)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[mcp-proxy {server}] {e}");
            ExitCode::FAILURE
        }
    }
}

/// Always ends with exactly one `solve.done`.
fn run_solve(profile: &dyn EngineProfile, task_file: &Path) -> ExitCode {
    gateway_solve_turn::apply_worker_env();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| solve(profile, task_file)))
        .unwrap_or_else(|panic| {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(HarnessError::internal(format!("panic: {msg}")))
        });
    match result {
        Ok(output) => {
            let text = output.to_string();
            let _ = gateway_solve_turn::emit_solve_done(0, &text, Some(&output));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[{}] solve failed: {}", profile.engine(), e.message);
            let _ = gateway_solve_turn::emit_solve_error(&e.message, e.status);
            ExitCode::FAILURE
        }
    }
}

fn solve(profile: &dyn EngineProfile, task_file: &Path) -> Result<Value, HarnessError> {
    let task = load_task(task_file)?;
    reject_unsupported(&task)?;
    let session_root =
        std::env::current_dir().map_err(|e| HarnessError::internal(format!("current dir: {e}")))?;
    let harness_dir = session_root.join(HARNESS_DIR);
    let tmp_dir = session_root.join("tmp");
    for dir in [&harness_dir, &tmp_dir] {
        std::fs::create_dir_all(dir)
            .map_err(|e| HarnessError::internal(format!("mkdir {}: {e}", dir.display())))?;
    }
    if std::env::var("TMPDIR").map_or(true, |v| v.trim().is_empty()) {
        // Still single-threaded: no runtime or child exists yet.
        std::env::set_var("TMPDIR", &tmp_dir);
    }
    // Landlock binds the calling thread and its future children: install it before the tokio
    // runtime spawns worker threads.
    apply_landlock(&task, &session_root)?;

    let model = resolve_model(&task)?;
    let openai_base_url = std::env::var("OPENAI_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HarnessError::internal("OPENAI_BASE_URL is not set"))?;
    let project_config_root = runtime::gateway_project_config_root(&session_root);

    write_turn_context(&session_root, &TurnContext::from_task(&task))?;
    let instructions = instructions_path(&session_root);
    write_file(
        &instructions,
        render_instructions(&project_config_root, task.extra_session.as_ref())?.as_bytes(),
    )?;
    let state = read_state(&session_root, profile.engine())?;
    let skills_dir = project_config_root.join(".claw").join("skills");
    let launch = profile.prepare(&PrepareContext {
        session_root: &session_root,
        project_config_root: &project_config_root,
        model: &model,
        openai_base_url: &openai_base_url,
        instructions_path: &instructions,
        skills_dir: &skills_dir,
    })?;
    let (mcp_servers, mcp_names) = mcp_proxy_servers(&session_root)?;

    let attachments = task.attachments.clone().unwrap_or_default();
    let mut session = open_transcript(&session_root)?;
    session
        .push_message(gateway_solve_turn::build_user_turn_message(
            &task.user_prompt,
            &attachments,
        ))
        .map_err(|e| HarnessError::internal(format!("append user message: {e}")))?;

    let spec = TurnSpec {
        engine: profile.engine(),
        launch,
        session_root: session_root.clone(),
        mcp_servers,
        prompt: acp_prompt(&task.user_prompt, &attachments, &session_root),
        max_iterations: Some(task.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS)),
        resume_session_id: state.map(|s| s.agent_session_id),
        mapper: TurnMapper::new(mcp_names, task.responses_stream),
        trace_path: harness_dir.join("acp-trace.ndjson"),
    };
    let timeout_seconds = task.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| HarnessError::internal(format!("tokio runtime: {e}")))?;
    let outcome = rt.block_on(async {
        tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(timeout_seconds), run_turn(spec, emit_event)) => {
                r.unwrap_or_else(|_| Err(HarnessError::new(504, format!("turn timed out after {timeout_seconds}s"))))
            }
            sig = wait_for_signal() => Err(HarnessError::internal(format!("received {sig}"))),
        }
    })?;
    // Child process group is gone once run_turn returned/was dropped; shut the runtime down
    // without waiting on stray blocking tasks.
    rt.shutdown_timeout(Duration::from_secs(1));

    for message in &outcome.transcript.messages {
        session
            .push_message(message.clone())
            .map_err(|e| HarnessError::internal(format!("append transcript: {e}")))?;
    }
    Ok(output_json(&task, &model, profile.engine(), &outcome))
}

fn output_json(
    task: &GatewaySolveTaskFile,
    model: &str,
    engine: &str,
    outcome: &TurnOutcome,
) -> Value {
    let usage = outcome.usage.map(|u| {
        json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "cache_creation_input_tokens": u.cache_creation_input_tokens,
            "cache_read_input_tokens": u.cache_read_input_tokens,
        })
    });
    json!({
        "model": model,
        "iterations": outcome.transcript.tool_calls,
        "message": outcome.transcript.message,
        "completionReason": outcome.completion_reason,
        "usage": usage,
        "harnessEngine": engine,
        "llmRoute": task.llm_route,
    })
}

fn emit_event(ev: &OutEvent) {
    let _ = match ev {
        OutEvent::ReportDelta(text) => gateway_solve_turn::emit_report_delta(text),
        OutEvent::ThinkingDelta(text) => gateway_solve_turn::emit_thinking_delta(text),
        OutEvent::ShellChunk { id, text } => gateway_solve_turn::emit_shell_chunk(id, text),
        OutEvent::ToolStart {
            id,
            name,
            kind,
            input,
        } => gateway_solve_turn::emit_raw_json(&gateway_solve_turn::tool_start_event_with_kind(
            id, name, kind, input,
        )),
        OutEvent::ToolEnd {
            id,
            name,
            kind,
            ok,
            duration_ms,
            output,
        } => gateway_solve_turn::emit_raw_json(&gateway_solve_turn::tool_end_event_with_kind(
            id,
            name,
            kind,
            *ok,
            *duration_ms,
            output,
        )),
    };
}

fn apply_landlock(task: &GatewaySolveTaskFile, session_root: &Path) -> Result<(), HarnessError> {
    let (Some(dsl), Some(source)) = (task.landlock_dsl.as_ref(), task.landlock_dsl_source) else {
        return Ok(());
    };
    let root = session_root.to_string_lossy().to_string();
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| format!("{root}/tmp"));
    let ctx = gateway_solve_turn::LandlockExpandContext {
        session_root: &root,
        project_home_def: "/claw_ds/project_home_def",
        tmpdir: &tmpdir,
        claw_bin_dir: "/usr/local/bin",
    };
    gateway_solve_turn::apply_strict_landlock_jail(dsl, source, &ctx)
        .map_err(|e| HarnessError::internal(format!("strict Landlock jail install failed: {e}")))
}

/// Every MCP server in the session settings becomes a stdio `mcp-proxy` child of the engine.
/// The proxy gets the full worker env, like claw's own MCP children.
fn mcp_proxy_servers(session_root: &Path) -> Result<(Vec<McpServer>, Vec<String>), HarnessError> {
    let config = ConfigLoader::default_for(session_root)
        .load()
        .map_err(|e| HarnessError::internal(format!("load session mcp config: {e}")))?;
    let names: Vec<String> = config.mcp().servers().keys().cloned().collect();
    if names.is_empty() {
        return Ok((Vec::new(), names));
    }
    let exe =
        std::env::current_exe().map_err(|e| HarnessError::internal(format!("current exe: {e}")))?;
    let env: Vec<EnvVariable> = std::env::vars()
        .map(|(k, v)| EnvVariable::new(k, v))
        .collect();
    let root = session_root.to_string_lossy().to_string();
    let servers = names
        .iter()
        .map(|name| {
            McpServer::Stdio(
                McpServerStdio::new(name.clone(), exe.clone())
                    .args(vec![
                        "mcp-proxy".to_string(),
                        "--server".to_string(),
                        name.clone(),
                        "--session-root".to_string(),
                        root.clone(),
                    ])
                    .env(env.clone()),
            )
        })
        .collect();
    Ok((servers, names))
}

fn open_transcript(session_root: &Path) -> Result<Session, HarnessError> {
    let path = gateway_solve_turn::gateway_solve_session_persistence_path(session_root);
    if path.exists() {
        Session::load_from_path(&path)
            .map_err(|e| HarnessError::internal(format!("load {}: {e}", path.display())))
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HarnessError::internal(format!("mkdir {}: {e}", parent.display())))?;
        }
        Ok(Session::new().with_persistence_path(path))
    }
}

/// Prompt text plus one `resource_link` per attachment (baseline ACP content every agent accepts).
fn acp_prompt(prompt: &str, attachments: &[SolveAttachment], session_root: &Path) -> Vec<AcpBlock> {
    let mut blocks: Vec<AcpBlock> = attachments
        .iter()
        .map(|a| {
            let path = session_root.join(&a.path);
            let name = a.name.clone().unwrap_or_else(|| a.path.clone());
            AcpBlock::ResourceLink(
                ResourceLink::new(name, format!("file://{}", path.display()))
                    .mime_type(a.mime.clone()),
            )
        })
        .collect();
    let text = prompt.trim();
    if !text.is_empty() || blocks.is_empty() {
        blocks.push(AcpBlock::Text(TextContent::new(text.to_string())));
    }
    blocks
}

async fn wait_for_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_subcommands() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        assert!(
            matches!(parse_args(&s(&["gateway-solve-once", "--task-file", "/t.json"])), Ok(Command::Solve(p)) if p == Path::new("/t.json"))
        );
        assert!(matches!(
            parse_args(&s(&["gateway-solve-once", "/t.json"])),
            Ok(Command::Solve(_))
        ));
        assert!(matches!(
            parse_args(&s(&["mcp-proxy", "--server", "probe", "--session-root", "/s"])),
            Ok(Command::McpProxy { server, .. }) if server == "probe"
        ));
        assert!(parse_args(&s(&["gateway-solve-once", "--engine", "x", "/t"])).is_err());
        assert!(parse_args(&s(&["mcp-proxy", "--server", "p"])).is_err());
    }

    #[test]
    fn prompt_puts_attachment_links_before_text() {
        let att: SolveAttachment = serde_json::from_value(json!({
            "path": "uploads/a.png", "mime": "image/png", "kind": "image"
        }))
        .unwrap();
        let blocks = acp_prompt("look", &[att], Path::new("/claw_sessions/s1"));
        assert_eq!(blocks.len(), 2);
        let AcpBlock::ResourceLink(link) = &blocks[0] else {
            panic!("expected resource link")
        };
        assert_eq!(link.uri, "file:///claw_sessions/s1/uploads/a.png");
        assert!(matches!(&blocks[1], AcpBlock::Text(t) if t.text == "look"));
    }
}
