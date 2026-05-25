//! Parallel ProgressNarrator LLM lane. Author: kejiqing

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use api::ToolDefinition;

use crate::multi_agent::event_bus::EventBus;
use crate::multi_agent::phase_turn::{format_events_for_narrator, run_phase_turn};
use crate::multi_agent::phases::{load_phase_prompt, DEFAULT_NARRATOR_MD};
use crate::project_orchestration::SolveOrchestrationConfig;
use crate::task_progress::REPORT_PROGRESS_TOOL_NAME;
use crate::{DirectApiClient, DirectToolExecutor, GatewaySolveTurnError};

pub struct NarratorHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl NarratorHandle {
    pub fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Spawn background narrator thread (LLM + report_progress only).
#[allow(clippy::too_many_arguments)]
pub fn spawn_narrator_lane(
    work_dir: std::path::PathBuf,
    session_id: String,
    orch: SolveOrchestrationConfig,
    model: String,
    executor: DirectToolExecutor,
    event_bus: EventBus,
) -> Result<NarratorHandle, GatewaySolveTurnError> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = Arc::clone(&stop);
    let throttle = Duration::from_millis(orch.narrator_throttle_ms.max(500));
    let join = thread::Builder::new()
        .name("claw-narrator".into())
        .spawn(move || {
            narrator_loop(
                &work_dir,
                &session_id,
                &model,
                &executor,
                &event_bus,
                stop_t,
                throttle,
            );
        })
        .map_err(|e| GatewaySolveTurnError {
            status: 500,
            message: format!("spawn narrator failed: {e}"),
        })?;
    Ok(NarratorHandle {
        stop,
        join: Some(join),
    })
}

fn narrator_loop(
    work_dir: &Path,
    session_id: &str,
    model: &str,
    executor: &DirectToolExecutor,
    event_bus: &EventBus,
    stop: Arc<AtomicBool>,
    throttle: Duration,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("claw-narrator-rt")
        .build()
        .expect("narrator tokio runtime");
    let _ = rt.block_on(async {
        narrator_loop_async(
            work_dir,
            session_id,
            model,
            executor,
            event_bus,
            stop,
            throttle,
        )
        .await;
    });
}

async fn narrator_loop_async(
    work_dir: &Path,
    session_id: &str,
    model: &str,
    executor: &DirectToolExecutor,
    event_bus: &EventBus,
    stop: Arc<AtomicBool>,
    throttle: Duration,
) {
    let mut last_event_count = 0usize;
    while !stop.load(Ordering::SeqCst) {
        tokio::time::sleep(throttle).await;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let events = match event_bus.read_all() {
            Ok(e) => e,
            Err(_) => continue,
        };
        if events.len() <= last_event_count {
            continue;
        }
        let batch = events[last_event_count..].to_vec();
        last_event_count = events.len();
        let _ = run_narrator_batch(work_dir, session_id, model, executor, &batch);
    }
    // Final flush
    if let Ok(events) = event_bus.read_all() {
        if events.len() > last_event_count {
            let batch = events[last_event_count..].to_vec();
            let _ = run_narrator_batch(work_dir, session_id, model, executor, &batch);
        }
    }
}

fn run_narrator_batch(
    work_dir: &Path,
    session_id: &str,
    model: &str,
    executor: &DirectToolExecutor,
    batch: &[crate::multi_agent::event_bus::OrchestrationEvent],
) -> Result<(), GatewaySolveTurnError> {
    if batch.is_empty() {
        return Ok(());
    }
    let system = load_phase_prompt(work_dir, "narrator.md", DEFAULT_NARRATOR_MD);
    let allowed = vec![REPORT_PROGRESS_TOOL_NAME.to_string()];
    let tools: Vec<ToolDefinition> = vec![crate::report_progress_tool_definition()];
    let api = DirectApiClient::new(
        model.to_string(),
        &allowed,
        tools,
        session_id.to_string(),
    )
    .map_err(|e| GatewaySolveTurnError {
        status: 500,
        message: e.message,
    })?;
    let phase_executor = executor.clone_with_allowed_tools(allowed);
    let user = format_events_for_narrator(batch);
    let _ = run_phase_turn(user, api, phase_executor, vec![system], 2, false);
    Ok(())
}
