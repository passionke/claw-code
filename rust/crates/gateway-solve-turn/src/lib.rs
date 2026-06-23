//! One-turn gateway solve (used by worker containers via `claw gateway-solve-once`).
//! Author: kejiqing
#![allow(
    clippy::await_holding_lock,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::format_push_string,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::needless_lifetimes,
    clippy::needless_pass_by_value,
    clippy::result_large_err,
    clippy::single_match_else,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::implicit_hasher,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::used_underscore_binding,
    clippy::redundant_closure_for_method_calls,
    clippy::type_complexity,
    clippy::unnecessary_filter_map,
    clippy::useless_format
)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use api::{
    ContentBlockDelta, InputContentBlock, InputMessage, MessageRequest, OutputContentBlock,
    ProviderClient, StreamEvent, ToolChoice, ToolDefinition, ToolResultContentBlock,
};
use runtime::{
    apply_config_env_if_unset, apply_mcp_tool_annotations_from_config, concurrent_mcp_tool_names,
    default_mcp_max_concurrent, gateway_git_import_prompt_section,
    gateway_pool_layout_prompt_section, gateway_schema_prompt_section, load_system_prompt,
    mcp_description_parallel_friendly, mcp_tool_parallel_fanout_eligible,
    ApiClient as RuntimeApiClient, ApiRequest, AssistantEvent, ConfigLoader, ContentBlock,
    ConversationMessage, ConversationRuntime, McpServerManager, McpTool, MessageRole,
    PermissionMode, PermissionPolicy, RuntimeConfig, RuntimeError, Session, SharedToolExecutor,
    ToolError, ToolExecutor as RuntimeToolExecutor,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use telemetry::{JsonlTelemetrySink, SessionTracer};
use tokio::sync::Semaphore;
use tools::{
    execute_agent_with_mcp_context_and_spawn, execute_mcp_tool_with_meta, execute_tool,
    initialize_mcp_bridge, mvp_tool_specs, AgentInput,
};

pub mod agent_orchestration;
pub mod entity_labels;
pub mod gateway_stdout;
pub mod mcp_call_context;
pub mod multi_agent;
mod otel_solve_turn;
pub mod ovs_interactive;
pub mod project_language_pipeline;
pub mod project_orchestration;
pub mod project_preflight;
pub mod session_report;
pub mod solve_timing;
pub mod sqlbot_preflight;
pub mod task_progress;
pub mod turn_language;
pub mod turn_tools;
pub mod worker_env;
pub use gateway_stdout::{
    emit_report_delta, emit_solve_done, emit_solve_error, parse_stdout_line,
    GATEWAY_STDOUT_LINE_PREFIX,
};
pub use mcp_call_context::{
    build_mcp_call_meta, build_sqlbot_mcp_start_arguments, gateway_mcp_call_context_from_task,
    inject_mcp_call_meta, resolve_gateway_mcp_call_context, resolve_gateway_trace_id,
    GatewayMcpCallContext, CLAW_EXTRA_SESSION_SESSION_ID, CLAW_EXTRA_SESSION_TURN_ID,
};
pub use ovs_interactive::{
    build_ensure_ovs_interactive_session_script, build_ovs_interactive_prompt_script,
    ovs_interactive_guest_symlink_host, ovs_interactive_session_dir_host,
    ovs_interactive_session_jsonl_guest, ovs_interactive_session_jsonl_host,
    ovs_interactive_symlink_target, OVS_INTERACTIVE_GUEST_REL_PREFIX,
    OVS_INTERACTIVE_LEGACY_GUEST_REL_PREFIX, OVS_INTERACTIVE_PROJ_HOME,
    OVS_INTERACTIVE_SESSION_FILENAME, OVS_INTERACTIVE_WORK_ROOT,
};
pub use runtime::McpCallContext;
pub use session_report::{
    final_assistant_report_text_from_jsonl,
    final_assistant_report_text_from_jsonl_for_user_turn_index,
};
pub use solve_timing::{
    append_solve_timing_point, read_solve_timing_events, truncate_solve_timing_events,
    SolveTimingEvent, SolveTimingRecorder, SOLVE_TIMING_EVENTS_REL,
};
pub use task_progress::{
    progress_events_path, progress_message_from_mcp_input, read_progress_events,
    read_progress_history, read_task_progress, record_mcp_tool_started,
    report_progress_tool_definition, reset_task_progress, run_report_progress,
    sanitize_current_task_desc, should_emit_tool_progress_event, task_progress_history_path,
    task_progress_json_path, truncate_progress_history, ProgressEvent, ReportProgressInput,
    TaskProgressFile, TaskProgressTodo, REPORT_PROGRESS_TOOL_NAME,
};
pub use worker_env::{
    apply_worker_env, build_write_gateway_record_session_script, gateway_llm_session_extra_headers,
    otel_forward_env, resolve_gateway_llm_session_id, worker_env_keys_set,
    GATEWAY_RECORD_SESSION_ID_GUEST, GATEWAY_RECORD_SESSION_ID_REL, WORKER_ENV_KEYS,
    WORKER_ENV_MOUNT_PATH,
};

pub(crate) const HTTP_INTERNAL: u16 = 500;

/// Suffix appended to the LLM-facing description when MCP `tools/list` annotations allow concurrent calls.
/// Author: kejiqing
const PARALLEL_FRIENDLY_TOOL_DESCRIPTION_HINT: &str = "\n\n[parallel-friendly] This tool is safe to call multiple times concurrently within a single assistant turn. When you have N independent sub-questions, emit N tool_use blocks in the same response instead of one per turn; the backend executes them in parallel with bounded concurrency.";

fn decorate_mcp_tool_description(tool: &McpTool, original: Option<String>) -> Option<String> {
    if default_mcp_max_concurrent() <= 1 || !mcp_tool_parallel_fanout_eligible(tool) {
        return original;
    }
    let base = original.unwrap_or_default();
    if base.contains("[parallel-friendly]") || mcp_description_parallel_friendly(tool) {
        return Some(base);
    }
    let mut decorated = base;
    decorated.push_str(PARALLEL_FRIENDLY_TOOL_DESCRIPTION_HINT);
    Some(decorated)
}

/// DeepSeek-official routing for `/v1/biz_advice_report` polish only (`REPORT_LLM_PROVIDER=deepseek`). kejiqing
#[derive(Clone, Debug)]
pub struct ReportPolishDeepseek {
    pub api_key: String,
    pub model: String,
}

/// Fixed `proj_id` for boss-report skill content (`home/skills/GPOS_BOSS_REPORT_WRITER/SKILL.md`). kejiqing
pub const BOSS_REPORT_SKILL_PROJ_ID: i64 = 1;

/// Fixed transcript path under a session workspace (gateway continues-by-sid). kejiqing
#[must_use]
pub fn gateway_solve_session_persistence_path(work_dir: &Path) -> PathBuf {
    work_dir.join(".claw").join("gateway-solve-session.jsonl")
}

/// Error from a single gateway solve turn (HTTP status hint for gateway mapping).
#[derive(Debug)]
pub struct GatewaySolveTurnError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for GatewaySolveTurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GatewaySolveTurnError {}

pub(crate) fn err(status: u16, msg: impl Into<String>) -> GatewaySolveTurnError {
    GatewaySolveTurnError {
        status,
        message: msg.into(),
    }
}

/// SQLBot session variables: always include `org_id` (empty string if omitted). Author: kejiqing
#[must_use]
pub fn normalize_extra_session(extra_session: Option<Value>) -> Option<Value> {
    match extra_session {
        None => Some(json!({ "org_id": "" })),
        Some(Value::Object(mut map)) => {
            if !map.contains_key("org_id") {
                map.insert("org_id".to_string(), Value::String(String::new()));
            }
            Some(Value::Object(map))
        }
        Some(other) => Some(other),
    }
}

/// Task payload written by the gateway / read by `claw gateway-solve-once`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySolveTaskFile {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "userPrompt")]
    pub user_prompt: String,
    pub model: Option<String>,
    #[serde(rename = "timeoutSeconds")]
    pub timeout_seconds: Option<u64>,
    #[serde(rename = "extraSession")]
    pub extra_session: Option<Value>,
    #[serde(rename = "allowedTools")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(rename = "maxIterations")]
    pub max_iterations: Option<usize>,
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(rename = "poolId", skip_serializing_if = "Option::is_none")]
    pub pool_id: Option<String>,
    #[serde(rename = "workerName", skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    /// Per-solve LLM routing snapshot (gateway-injected). Author: kejiqing
    #[serde(rename = "llmRoute", skip_serializing_if = "Option::is_none")]
    pub llm_route: Option<Value>,
    /// W3C traceparent from gateway `gateway.solve` span for distributed tracing. Author: kejiqing
    #[serde(rename = "otelTraceparent", skip_serializing_if = "Option::is_none")]
    pub otel_traceparent: Option<String>,
}

pub(crate) fn default_system_date() -> String {
    match option_env!("BUILD_DATE") {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => current_utc_date(),
    }
}

fn current_utc_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let days_since_epoch = (now.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::similar_names
)]
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

pub(crate) fn project_config_loader_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_PROJECT_CONFIG_ROOT") {
        let root = PathBuf::from(raw.trim());
        if root.as_os_str().is_empty() {
            return None;
        }
        return Some(root);
    }
    let Ok(cfg_file) = std::env::var("CLAW_CONFIG_FILE") else {
        return None;
    };
    let path = PathBuf::from(cfg_file.trim());
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

fn gateway_trace_file_path(trace_id: &str, work_root: &Path) -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_TRACE_FILE") {
        let p = raw.trim();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let enabled = std::env::var("CLAW_TRACE_ENABLED")
        .unwrap_or_else(|_| "1".to_string())
        .trim()
        .to_ascii_lowercase();
    if !matches!(enabled.as_str(), "1" | "true" | "yes" | "on") {
        return None;
    }
    let dir = match std::env::var("CLAW_TRACE_DIR") {
        Ok(raw) => {
            let s = raw.trim();
            if s.is_empty() {
                work_root.join("traces")
            } else {
                PathBuf::from(s)
            }
        }
        Err(_) => work_root.join("traces"),
    };
    Some(dir.join(format!("{trace_id}.ndjson")))
}

pub(crate) fn gateway_session_tracer(request_id: &str, work_root: &Path) -> Option<SessionTracer> {
    let trace_id = resolve_gateway_trace_id(request_id);
    let path = gateway_trace_file_path(&trace_id, work_root)?;
    let sink = JsonlTelemetrySink::new(path).ok()?;
    Some(SessionTracer::new(trace_id, Arc::new(sink)))
}

/// Pick MCP tool for multi-agent parallel fan-out: `queryMcpTool`, else description `parallel-friendly`, else annotations.
#[must_use]
pub fn resolve_query_fanout_tool_name(
    registered: &HashSet<String>,
    concurrent: &HashSet<String>,
    parallel_friendly: &HashSet<String>,
    query_mcp_tool: Option<&str>,
) -> Option<String> {
    if let Some(spec) = query_mcp_tool.map(str::trim).filter(|s| !s.is_empty()) {
        if registered.contains(spec) {
            return Some(spec.to_string());
        }
        let suffix = format!("__{spec}");
        return registered
            .iter()
            .find(|n| n.as_str() == spec || n.ends_with(&suffix))
            .cloned();
    }
    parallel_friendly
        .iter()
        .find(|name| registered.contains(*name))
        .cloned()
        .or_else(|| {
            concurrent
                .iter()
                .find(|name| registered.contains(*name))
                .cloned()
        })
}

pub(crate) fn initialize_mcp_runtime(
    work_dir: &Path,
) -> Result<
    (
        Vec<ToolDefinition>,
        HashSet<String>,
        HashSet<String>,
        HashSet<String>,
        Option<Arc<StdMutex<McpServerManager>>>,
    ),
    GatewaySolveTurnError,
> {
    let config_root = runtime::gateway_project_config_root(work_dir);
    let runtime_cfg = ConfigLoader::default_for(&config_root)
        .load()
        .map_err(|e| err(HTTP_INTERNAL, format!("load runtime config failed: {e}")))?;
    let mut manager = McpServerManager::from_runtime_config(&runtime_cfg);
    if manager.server_names().is_empty() && manager.unsupported_servers().is_empty() {
        return Ok((
            Vec::new(),
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
            None,
        ));
    }

    let report = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(async { manager.discover_tools_best_effort().await })
    });

    let manager = Arc::new(StdMutex::new(manager));
    initialize_mcp_bridge(Arc::clone(&manager), &report);

    let mut discovered_tools = report.tools;
    apply_mcp_tool_annotations_from_config(&mut discovered_tools, runtime_cfg.mcp().servers());

    let concurrent_mcp_tool_names = concurrent_mcp_tool_names(&discovered_tools);
    let parallel_friendly_mcp_tool_names: HashSet<String> = discovered_tools
        .iter()
        .filter(|entry| mcp_description_parallel_friendly(&entry.tool))
        .map(|entry| entry.qualified_name.clone())
        .collect();
    let mut runtime_mcp_tools = Vec::new();
    let mut runtime_mcp_tool_names = HashSet::new();
    for discovered in discovered_tools {
        let name = discovered.qualified_name;
        let input_schema = discovered
            .tool
            .input_schema
            .clone()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        let description =
            decorate_mcp_tool_description(&discovered.tool, discovered.tool.description.clone());
        runtime_mcp_tools.push(ToolDefinition {
            name: name.clone(),
            description,
            input_schema,
        });
        runtime_mcp_tool_names.insert(name);
    }

    Ok((
        runtime_mcp_tools,
        runtime_mcp_tool_names,
        concurrent_mcp_tool_names,
        parallel_friendly_mcp_tool_names,
        Some(manager),
    ))
}

pub(crate) struct DirectApiClient {
    model: String,
    provider: ProviderClient,
    tools: Vec<ToolDefinition>,
    clawcode_session_id: String,
    /// When true, LLM text chunks are mirrored to live report SSE. Author: kejiqing
    stream_report_deltas: bool,
}

impl DirectApiClient {
    pub(crate) fn new(
        model: String,
        allowed_tools: &[String],
        runtime_mcp_tools: Vec<ToolDefinition>,
        clawcode_session_id: String,
    ) -> Result<Self, GatewaySolveTurnError> {
        let provider = ProviderClient::from_model(&model)
            .map_err(|e| err(HTTP_INTERNAL, format!("provider init failed: {e}")))?;
        let mut tools: Vec<ToolDefinition> = mvp_tool_specs()
            .into_iter()
            .filter(|spec| is_tool_allowed(spec.name, allowed_tools))
            .map(|spec| ToolDefinition {
                name: spec.name.to_string(),
                description: Some(spec.description.to_string()),
                input_schema: spec.input_schema,
            })
            .collect();
        tools.extend(
            runtime_mcp_tools
                .into_iter()
                .filter(|tool| is_tool_allowed(&tool.name, allowed_tools)),
        );
        if is_tool_allowed(REPORT_PROGRESS_TOOL_NAME, allowed_tools) {
            tools.push(report_progress_tool_definition());
        }
        let tools = dedupe_tool_definitions_by_name(tools);
        Ok(Self {
            model,
            provider,
            tools,
            clawcode_session_id,
            stream_report_deltas: true,
        })
    }

    #[must_use]
    pub(crate) fn with_stream_report_deltas(mut self, enabled: bool) -> Self {
        self.stream_report_deltas = enabled;
        self
    }
}

/// Keep the first tool per name; upstream APIs reject duplicate tool names. Author: kejiqing
fn dedupe_tool_definitions_by_name(tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
    let mut seen = HashSet::new();
    tools
        .into_iter()
        .filter(|tool| seen.insert(tool.name.clone()))
        .collect()
}

impl RuntimeApiClient for DirectApiClient {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let system =
            (!request.system_prompt.is_empty()).then(|| request.system_prompt.join("\n\n"));
        let messages = convert_runtime_messages_to_api(&request.messages);
        let req = MessageRequest {
            model: self.model.clone(),
            max_tokens: api::max_tokens_for_model(&self.model),
            messages,
            system,
            tools: Some(self.tools.clone()),
            tool_choice: Some(ToolChoice::Auto),
            stream: true,
            extra_headers: BTreeMap::from([
                (
                    "clawcode-session-id".to_string(),
                    self.clawcode_session_id.clone(),
                ),
                (
                    "claw-session-id".to_string(),
                    self.clawcode_session_id.clone(),
                ),
            ]),
            // Boss report needs visible `content` text (live SSE + outputJson.message).
            thinking_enabled: Some(false),
            ..Default::default()
        };
        let stream_report_deltas = self.stream_report_deltas;
        let mut on_delta = move |text: &str| {
            if stream_report_deltas {
                let _ = emit_report_delta(text);
            }
        };
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(stream_events(
                &self.provider,
                &req,
                Some(&mut on_delta),
            ))
        })
        .map_err(|e| RuntimeError::new(e.to_string()))
    }
}

pub struct DirectToolExecutor {
    inner: Arc<DirectToolExecutorInner>,
}

impl Clone for DirectToolExecutor {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct DirectToolExecutorInner {
    session_home: PathBuf,
    mcp_context: GatewayMcpCallContext,
    /// Parent solve turn model; sub-agents inherit when `Agent` omits `model`. Author: kejiqing
    turn_model: String,
    allowed_tools: Vec<String>,
    runtime_mcp_manager: Option<Arc<StdMutex<McpServerManager>>>,
    runtime_mcp_tool_names: HashSet<String>,
    concurrent_mcp_tools: HashSet<String>,
    parallel_friendly_mcp_tools: HashSet<String>,
    session_tracer: Option<SessionTracer>,
    timing: Option<Arc<SolveTimingRecorder>>,
    mcp_semaphore: Arc<Semaphore>,
    /// `gateway-solve-once` enters this runtime on the main thread; background analysis
    /// tools call MCP via `Handle::block_on` from worker threads. Author: kejiqing
    async_runtime: tokio::runtime::Handle,
}

impl DirectToolExecutorInner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        session_home: PathBuf,
        mcp_context: GatewayMcpCallContext,
        turn_model: String,
        allowed_tools: Vec<String>,
        runtime_mcp_manager: Option<Arc<StdMutex<McpServerManager>>>,
        runtime_mcp_tool_names: HashSet<String>,
        concurrent_mcp_tools: HashSet<String>,
        parallel_friendly_mcp_tools: HashSet<String>,
        session_tracer: Option<SessionTracer>,
        timing: Option<Arc<SolveTimingRecorder>>,
        async_runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            session_home,
            mcp_context,
            turn_model,
            allowed_tools,
            runtime_mcp_manager,
            runtime_mcp_tool_names,
            concurrent_mcp_tools,
            parallel_friendly_mcp_tools,
            session_tracer,
            timing,
            mcp_semaphore: Arc::new(Semaphore::new(default_mcp_max_concurrent().max(1))),
            async_runtime,
        }
    }

    fn execute_impl(&self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        self.execute_impl_inner(tool_name, input)
    }

    fn execute_impl_inner(&self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if !is_tool_allowed(tool_name, &self.allowed_tools) {
            return Err(ToolError::new(format!("tool not allowed: {tool_name}")));
        }
        if tool_name == REPORT_PROGRESS_TOOL_NAME {
            let parsed = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
            let result = run_report_progress(
                &self.session_home,
                self.mcp_context.clawcode_session_id(),
                &parsed,
            )
            .map_err(ToolError::new)?;
            if let Some(tracer) = &self.session_tracer {
                if let Some(progress) = read_task_progress(&self.session_home) {
                    let mut attrs = serde_json::Map::new();
                    attrs.insert(
                        "current_task_desc".to_string(),
                        Value::String(progress.current_task_desc.clone()),
                    );
                    attrs.insert("phase".to_string(), Value::String(progress.phase.clone()));
                    tracer.record("user_progress", attrs);
                }
            }
            return Ok(result);
        }
        if tool_name == "MCP" {
            let args = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
            if should_emit_tool_progress_event(tool_name, false, Some(&args)) {
                let _ = record_mcp_tool_started(
                    &self.session_home,
                    self.mcp_context.clawcode_session_id(),
                    self.mcp_context.extra_session.as_ref(),
                    &args,
                );
            }
            let meta = inject_mcp_call_meta(&self.mcp_context);
            let out = execute_mcp_tool_with_meta(input, Some(&meta)).map_err(ToolError::new);
            if should_emit_tool_progress_event(tool_name, false, Some(&args)) {
                if let Ok(ref text) = &out {
                    let _ = entity_labels::ingest_entity_labels_from_mcp_response(
                        &self.session_home,
                        self.mcp_context.extra_session.as_ref(),
                        &args,
                        text,
                        false,
                    );
                }
            }
            return out;
        }
        if self.runtime_mcp_tool_names.contains(tool_name) {
            return self.call_runtime_mcp_tool(tool_name, input);
        }
        if tool_name == "Agent" {
            let agent_input: AgentInput = serde_json::from_str(input)
                .map_err(|e| ToolError::new(format!("invalid Agent tool JSON: {e}")))?;
            let bus = crate::multi_agent::EventBus::new(&self.session_home);
            let out = execute_agent_with_mcp_context_and_spawn(
                agent_input,
                Some(self.mcp_context.clone()),
                Some(self.turn_model.as_str()),
                |job| crate::agent_orchestration::spawn_gateway_agent_with_events(&bus, job),
            )
            .map_err(ToolError::new)?;
            return serde_json::to_string_pretty(&out).map_err(|e| ToolError::new(e.to_string()));
        }
        let parsed = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
        execute_tool(tool_name, &parsed).map_err(ToolError::new)
    }

    fn call_runtime_mcp_tool(&self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        let args = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
        let emit = should_emit_tool_progress_event(tool_name, true, Some(&args));
        if emit {
            let _ = record_mcp_tool_started(
                &self.session_home,
                self.mcp_context.clawcode_session_id(),
                self.mcp_context.extra_session.as_ref(),
                &args,
            );
        }
        let meta = inject_mcp_call_meta(&self.mcp_context);
        let Some(manager) = &self.runtime_mcp_manager else {
            return Err(ToolError::new("MCP manager not initialized"));
        };
        let manager = Arc::clone(manager);
        let semaphore = Arc::clone(&self.mcp_semaphore);
        let tool_name_owned = tool_name.to_string();
        let args_for_labels = args.clone();
        let response = self.async_runtime.block_on(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|_| ToolError::new("MCP concurrency semaphore closed"))?;
            McpServerManager::call_tool_concurrent(
                manager,
                &tool_name_owned,
                Some(args),
                Some(meta),
            )
            .await
            .map_err(|e| ToolError::new(e.to_string()))
        });
        match response {
            Ok(resp) => {
                if let Some(error) = resp.error {
                    return Err(ToolError::new(format!(
                        "MCP tool call failed: {} ({})",
                        error.message, error.code
                    )));
                }
                let Some(result) = resp.result else {
                    return Err(ToolError::new("MCP tool call returned no result"));
                };
                let output = serde_json::to_string(&result)
                    .map_err(|e| ToolError::new(format!("serialize MCP result failed: {e}")))?;
                if emit {
                    let _ = entity_labels::ingest_entity_labels_from_mcp_response(
                        &self.session_home,
                        self.mcp_context.extra_session.as_ref(),
                        &args_for_labels,
                        &output,
                        false,
                    );
                }
                Ok(output)
            }
            Err(e) => Err(e),
        }
    }
}

impl SharedToolExecutor for DirectToolExecutorInner {
    fn execute_shared(&self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        self.execute_impl(tool_name, input)
    }

    fn allows_concurrent_mcp_call(&self, tool_name: &str) -> bool {
        self.concurrent_mcp_tools.contains(tool_name)
    }
}

impl RuntimeToolExecutor for DirectToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        self.inner.execute_impl(tool_name, input)
    }

    fn shared_executor(&self) -> Option<Arc<dyn SharedToolExecutor>> {
        if default_mcp_max_concurrent() <= 1 {
            return None;
        }
        Some(self.inner.clone())
    }
}

impl DirectToolExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_home: PathBuf,
        mcp_context: GatewayMcpCallContext,
        turn_model: String,
        allowed_tools: Vec<String>,
        runtime_mcp_manager: Option<Arc<StdMutex<McpServerManager>>>,
        runtime_mcp_tool_names: HashSet<String>,
        concurrent_mcp_tools: HashSet<String>,
        parallel_friendly_mcp_tools: HashSet<String>,
        session_tracer: Option<SessionTracer>,
        timing: Option<Arc<SolveTimingRecorder>>,
        async_runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            inner: Arc::new(DirectToolExecutorInner::new(
                session_home,
                mcp_context,
                turn_model,
                allowed_tools,
                runtime_mcp_manager,
                runtime_mcp_tool_names,
                concurrent_mcp_tools,
                parallel_friendly_mcp_tools,
                session_tracer,
                timing,
                async_runtime,
            )),
        }
    }

    #[must_use]
    pub fn turn_timing(&self) -> Option<Arc<SolveTimingRecorder>> {
        self.inner.timing.clone()
    }

    /// Normalized `extraSession` for this solve turn (`resolve_gateway_mcp_call_context`). Author: kejiqing
    #[must_use]
    pub fn mcp_extra_session(&self) -> Option<&Value> {
        self.inner.mcp_context.extra_session.as_ref()
    }

    /// Explicit timing for loop-outside tool calls (`preflight`, `query_fanout`, …).
    pub fn record_out_of_loop_tool_timing(
        &self,
        tool_name: &str,
        started: Instant,
        is_error: bool,
        source: &str,
    ) {
        if let Some(timing) = self.turn_timing() {
            let _ = timing.record_out_of_loop_tool(
                tool_name,
                started.elapsed().as_millis(),
                is_error,
                source,
            );
        }
    }

    /// First registered MCP tool whose `tools/list` annotations allow concurrent calls.
    #[must_use]
    pub fn first_concurrent_mcp_tool(&self, registered: &HashSet<String>) -> Option<String> {
        self.inner
            .concurrent_mcp_tools
            .iter()
            .find(|name| registered.contains(*name))
            .cloned()
    }

    /// Resolve MCP tool for multi-agent `query_fanout` (`queryMcpTool` or `parallel-friendly` in description).
    #[must_use]
    pub fn resolve_query_fanout_tool(
        &self,
        registered: &HashSet<String>,
        query_mcp_tool: Option<&str>,
    ) -> Option<String> {
        resolve_query_fanout_tool_name(
            registered,
            &self.inner.concurrent_mcp_tools,
            &self.inner.parallel_friendly_mcp_tools,
            query_mcp_tool,
        )
    }

    /// Whether fanout should use isolated SQLBot args (`token` + `question`, no shared `chat_id`).
    #[must_use]
    pub fn query_fanout_uses_isolated_args(&self, tool_name: &str) -> bool {
        self.inner.parallel_friendly_mcp_tools.contains(tool_name)
    }

    pub fn call_tool(&self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        self.inner.execute_impl(tool_name, input)
    }

    pub fn clone_with_allowed_tools(&self, allowed_tools: Vec<String>) -> Self {
        Self {
            inner: Arc::new(DirectToolExecutorInner {
                session_home: self.inner.session_home.clone(),
                mcp_context: self.inner.mcp_context.clone(),
                turn_model: self.inner.turn_model.clone(),
                allowed_tools,
                runtime_mcp_manager: self.inner.runtime_mcp_manager.clone(),
                runtime_mcp_tool_names: self.inner.runtime_mcp_tool_names.clone(),
                concurrent_mcp_tools: self.inner.concurrent_mcp_tools.clone(),
                parallel_friendly_mcp_tools: self.inner.parallel_friendly_mcp_tools.clone(),
                session_tracer: self.inner.session_tracer.clone(),
                timing: self.inner.timing.clone(),
                mcp_semaphore: Arc::clone(&self.inner.mcp_semaphore),
                async_runtime: self.inner.async_runtime.clone(),
            }),
        }
    }

    #[must_use]
    pub fn allows_tool(&self, tool_name: &str) -> bool {
        is_tool_allowed(tool_name, &self.inner.allowed_tools)
    }
}

fn is_tool_allowed(tool_name: &str, allowed_tools: &[String]) -> bool {
    if allowed_tools.is_empty() {
        return true;
    }
    for pattern in allowed_tools {
        if pattern == tool_name {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if tool_name.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

fn convert_runtime_messages_to_api(messages: &[ConversationMessage]) -> Vec<InputMessage> {
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "user",
            }
            .to_string();
            let content = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => {
                        Some(InputContentBlock::Text { text: text.clone() })
                    }
                    ContentBlock::ReasoningContent { text } => {
                        Some(InputContentBlock::ReasoningContent { text: text.clone() })
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        let parsed =
                            serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
                        Some(InputContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: parsed,
                        })
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                        ..
                    } => Some(InputContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: vec![ToolResultContentBlock::Text {
                            text: output.clone(),
                        }],
                        is_error: *is_error,
                    }),
                })
                .collect::<Vec<_>>();
            InputMessage { role, content }
        })
        .collect()
}

fn push_text_delta<F>(
    events: &mut Vec<AssistantEvent>,
    text: String,
    on_text_delta: &mut Option<&mut F>,
) where
    F: FnMut(&str),
{
    if text.is_empty() {
        return;
    }
    if let Some(cb) = on_text_delta.as_deref_mut() {
        cb(&text);
    }
    events.push(AssistantEvent::TextDelta(text));
}

pub(crate) async fn stream_events<F>(
    provider: &ProviderClient,
    req: &MessageRequest,
    on_text_delta: Option<&mut F>,
) -> Result<Vec<AssistantEvent>, api::ApiError>
where
    F: FnMut(&str),
{
    let mut stream = provider.stream_message(req).await?;
    let mut events = Vec::new();
    let mut pending_tools: HashMap<u32, (String, String, String)> = HashMap::new();
    let mut on_text_delta = on_text_delta;
    while let Some(event) = stream.next_event().await? {
        match event {
            StreamEvent::MessageStart(start) => {
                for block in start.message.content {
                    match block {
                        OutputContentBlock::Text { text } => {
                            push_text_delta(&mut events, text, &mut on_text_delta);
                        }
                        OutputContentBlock::ToolUse { id, name, input } => {
                            let initial_input = if input.is_object()
                                && input.as_object().is_some_and(serde_json::Map::is_empty)
                            {
                                String::new()
                            } else {
                                input.to_string()
                            };
                            pending_tools.insert(0, (id, name, initial_input));
                        }
                        OutputContentBlock::Thinking { thinking, .. } => {
                            if !thinking.is_empty() {
                                events.push(AssistantEvent::ThinkingDelta(thinking));
                            }
                        }
                        OutputContentBlock::RedactedThinking { .. } => {}
                    }
                }
            }
            StreamEvent::ContentBlockStart(start) => match start.content_block {
                OutputContentBlock::ToolUse { id, name, input } => {
                    let initial_input = if input.is_object()
                        && input.as_object().is_some_and(serde_json::Map::is_empty)
                    {
                        String::new()
                    } else {
                        input.to_string()
                    };
                    pending_tools.insert(start.index, (id, name, initial_input));
                }
                OutputContentBlock::Text { text } => {
                    push_text_delta(&mut events, text, &mut on_text_delta);
                }
                OutputContentBlock::Thinking { thinking, .. } => {
                    if !thinking.is_empty() {
                        events.push(AssistantEvent::ThinkingDelta(thinking));
                    }
                }
                OutputContentBlock::RedactedThinking { .. } => {}
            },
            StreamEvent::ContentBlockDelta(delta) => match delta.delta {
                ContentBlockDelta::TextDelta { text } => {
                    push_text_delta(&mut events, text, &mut on_text_delta);
                }
                ContentBlockDelta::InputJsonDelta { partial_json } => {
                    if let Some((_, _, input)) = pending_tools.get_mut(&delta.index) {
                        input.push_str(&partial_json);
                    }
                }
                ContentBlockDelta::ThinkingDelta { thinking } => {
                    if !thinking.is_empty() {
                        events.push(AssistantEvent::ThinkingDelta(thinking));
                    }
                }
                ContentBlockDelta::SignatureDelta { .. } => {}
            },
            StreamEvent::ContentBlockStop(stop) => {
                if let Some((id, name, input)) = pending_tools.remove(&stop.index) {
                    events.push(AssistantEvent::ToolUse { id, name, input });
                }
            }
            StreamEvent::MessageDelta(delta) => {
                events.push(AssistantEvent::Usage(delta.usage.token_usage()));
            }
            StreamEvent::MessageStop(_) => events.push(AssistantEvent::MessageStop),
        }
    }
    Ok(events)
}

fn resolve_polish_model(model: Option<&str>) -> String {
    model
        .map(str::to_string)
        .or_else(|| std::env::var("CLAW_DEFAULT_MODEL").ok())
        .unwrap_or_else(|| "openai/deepseek-v4-pro".to_string())
}

/// Concatenate assistant `Text` blocks; when empty, fall back to the final message's
/// `ReasoningContent` (Qwen3 thinking-only streams before `enable_thinking=false` fix).
pub(crate) fn assistant_report_text_from_turn(
    assistant_messages: &[ConversationMessage],
) -> String {
    let text = assistant_messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !text.trim().is_empty() {
        return text;
    }
    let reasoning = assistant_messages
        .last()
        .map(|m| {
            m.blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ReasoningContent { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if !reasoning.trim().is_empty() {
        tracing::warn!(
            target: "claw_gateway_orchestration",
            component = "gateway_solve_turn",
            "assistant turn had no Text blocks; using ReasoningContent fallback"
        );
    }
    reasoning
}

pub(crate) fn polish_output_from_events(
    events: &[AssistantEvent],
    model: &str,
) -> Result<(String, Value), GatewaySolveTurnError> {
    let mut text = String::new();
    let mut usage = runtime::TokenUsage::default();
    let mut finished = false;
    for event in events {
        match event {
            AssistantEvent::TextDelta(delta) => text.push_str(delta.as_str()),
            AssistantEvent::Usage(value) => usage = *value,
            AssistantEvent::MessageStop => finished = true,
            AssistantEvent::ToolUse { .. }
            | AssistantEvent::ThinkingDelta(_)
            | AssistantEvent::PromptCache(_) => {
                return Err(err(
                    HTTP_INTERNAL,
                    "biz polish must not use tools or extended reasoning output",
                ));
            }
        }
    }
    if !finished {
        return Err(err(
            HTTP_INTERNAL,
            "polish stream ended without a message stop event",
        ));
    }
    if text.is_empty() {
        return Err(err(
            HTTP_INTERNAL,
            "polish stream produced no assistant text",
        ));
    }
    let out_json = json!({
        "model": model,
        "iterations": 1,
        "message": text,
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_creation_input_tokens": usage.cache_creation_input_tokens,
            "cache_read_input_tokens": usage.cache_read_input_tokens
        }
    });
    Ok((
        serde_json::to_string(&out_json).unwrap_or_default(),
        out_json,
    ))
}

async fn run_gateway_biz_polish_llm_inner<F>(
    user_prompt: &str,
    model: Option<&str>,
    _timeout_seconds: u64,
    clawcode_session_id: &str,
    mut on_text_delta: Option<F>,
    report_deepseek: Option<&ReportPolishDeepseek>,
) -> Result<(String, Option<Value>), GatewaySolveTurnError>
where
    F: FnMut(&str),
{
    let (provider, effective_model) = if let Some(ds) = report_deepseek {
        (
            ProviderClient::from_deepseek_official(ds.api_key.as_str()),
            ds.model.clone(),
        )
    } else {
        let effective_model = resolve_polish_model(model);
        let provider = ProviderClient::from_model(&effective_model)
            .map_err(|e| err(HTTP_INTERNAL, format!("provider init failed: {e}")))?;
        (provider, effective_model)
    };
    let req = MessageRequest {
        model: effective_model.clone(),
        max_tokens: api::max_tokens_for_model(&effective_model),
        messages: vec![InputMessage {
            role: "user".to_string(),
            content: vec![InputContentBlock::Text {
                text: user_prompt.to_string(),
            }],
        }],
        system: None,
        tools: None,
        tool_choice: None,
        // Polish must return plain assistant text only; DeepSeek defaults thinking on.
        thinking_enabled: Some(false),
        stream: true,
        extra_headers: BTreeMap::from([
            (
                "clawcode-session-id".to_string(),
                clawcode_session_id.to_string(),
            ),
            (
                "claw-session-id".to_string(),
                clawcode_session_id.to_string(),
            ),
        ]),
        ..Default::default()
    };
    let events = stream_events(&provider, &req, on_text_delta.as_mut())
        .await
        .map_err(|e| err(HTTP_INTERNAL, format!("polish stream failed: {e}")))?;
    let (output_text, output_json) = polish_output_from_events(&events, &effective_model)?;
    Ok((output_text, Some(output_json)))
}

/// Single-turn LLM polish for boss reports: no workspace, MCP, or session setup.
pub fn run_gateway_biz_polish_llm<F>(
    user_prompt: &str,
    model: Option<&str>,
    timeout_seconds: u64,
    clawcode_session_id: &str,
    on_text_delta: Option<F>,
    report_deepseek: Option<&ReportPolishDeepseek>,
) -> Result<(String, Option<Value>), GatewaySolveTurnError>
where
    F: FnMut(&str),
{
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(run_gateway_biz_polish_llm_inner(
            user_prompt,
            model,
            timeout_seconds,
            clawcode_session_id,
            on_text_delta,
            report_deepseek,
        ))
    })
}

/// Async polish for gateway SSE (`biz.report.delta`); avoids `spawn_blocking` + `block_on` batching.
pub async fn run_gateway_biz_polish_llm_async<F>(
    user_prompt: &str,
    model: Option<&str>,
    timeout_seconds: u64,
    clawcode_session_id: &str,
    on_text_delta: Option<F>,
    report_deepseek: Option<&ReportPolishDeepseek>,
) -> Result<(String, Option<Value>), GatewaySolveTurnError>
where
    F: FnMut(&str),
{
    run_gateway_biz_polish_llm_inner(
        user_prompt,
        model,
        timeout_seconds,
        clawcode_session_id,
        on_text_delta,
        report_deepseek,
    )
    .await
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn run_gateway_solve_turn(
    work_dir: &Path,
    work_root: &Path,
    prompt: &str,
    model: Option<&str>,
    timeout_seconds: u64,
    mcp: GatewayMcpCallContext,
    allowed_tools: Vec<String>,
    max_iterations: usize,
    llm_route: Option<Value>,
) -> Result<(i32, String, Option<Value>), GatewaySolveTurnError> {
    std::env::set_current_dir(work_dir)
        .map_err(|e| err(HTTP_INTERNAL, format!("set current dir failed: {e}")))?;

    let mut otel_turn = otel_solve_turn::SolveTurnOtelGuard::start(&mcp, prompt);
    let _otel_turn_scope = otel_turn.enter();

    let clawcode_session_id = mcp.clawcode_session_id().to_string();
    let turn_id_attr = std::env::var("CLAW_TURN_ID").ok();
    let _ = truncate_solve_timing_events(work_dir);
    let _ = append_solve_timing_point(
        work_dir,
        "bootstrap_worker_entered",
        turn_id_attr.as_deref(),
    );

    let orch_cfg = project_orchestration::resolve_solve_orchestration_config(work_dir);
    if orch_cfg.is_multi_agent_analysis() {
        let result = multi_agent::run_multi_agent_solve_turn(
            work_dir,
            work_root,
            prompt,
            model,
            timeout_seconds,
            mcp,
            allowed_tools,
            max_iterations,
            orch_cfg,
        );
        return match &result {
            Ok((_, output, _)) => {
                otel_turn.mark_ok(output);
                result
            }
            Err(e) => {
                otel_turn.mark_error(&e.message);
                result
            }
        };
    }

    let project_cfg = match project_config_loader_root() {
        Some(root) => ConfigLoader::default_for(&root).load().map_err(|e| {
            err(
                HTTP_INTERNAL,
                format!("load claw config from {}: {e}", root.display()),
            )
        })?,
        None => RuntimeConfig::empty(),
    };
    apply_config_env_if_unset(&project_cfg);
    let effective_model = model
        .map(str::to_string)
        .or_else(|| std::env::var("CLAW_DEFAULT_MODEL").ok())
        .or_else(|| project_cfg.model().map(str::to_string))
        .unwrap_or_else(|| "openai/deepseek-v4-pro".to_string());
    let mut system_prompt = load_system_prompt(
        work_dir.to_path_buf(),
        default_system_date(),
        std::env::consts::OS,
        "unknown",
        Some(effective_model.clone()),
        mcp.extra_session.clone(),
    )
    .map_err(|e| err(HTTP_INTERNAL, format!("load system prompt failed: {e}")))?;

    let gateway_jsonl = gateway_solve_session_persistence_path(work_dir);
    let session_is_continuation = gateway_jsonl.exists();
    let pipeline_cfg = project_language_pipeline::resolve_language_pipeline_config(work_dir);
    let turn_id_for_language = turn_id_attr
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("unknown");
    let locked_language = turn_language::infer_and_persist_turn_language_blocking(
        work_dir,
        prompt,
        turn_id_for_language,
        &clawcode_session_id,
        &effective_model,
        &pipeline_cfg,
    )?;
    turn_language::inject_language_into_system_prompt(&mut system_prompt, &locked_language);
    let _ = append_solve_timing_point(work_dir, "turn_language_inferred", turn_id_attr.as_deref());

    let (
        runtime_mcp_tools,
        runtime_mcp_tool_names,
        concurrent_mcp_tool_names,
        parallel_friendly_mcp_tool_names,
        runtime_mcp_manager,
    ) = initialize_mcp_runtime(work_dir)?;
    let _ = append_solve_timing_point(work_dir, "bootstrap_mcp_ready", turn_id_attr.as_deref());
    let api_client = DirectApiClient::new(
        effective_model.clone(),
        &allowed_tools,
        runtime_mcp_tools,
        clawcode_session_id.clone(),
    )?;
    reset_task_progress(work_dir, &clawcode_session_id)
        .map_err(|e| err(HTTP_INTERNAL, format!("reset task progress failed: {e}")))?;
    let _ = truncate_progress_history(work_dir);
    let turn_timing = Arc::new(SolveTimingRecorder::new(work_dir));

    let orchestration_bus = crate::multi_agent::EventBus::new(work_dir);
    let _ = orchestration_bus.session_started();

    // `true` when this `sessionId` already has `.claw/gateway-solve-session.jsonl` (续聊 turn).
    let mut session = if session_is_continuation {
        Session::load_from_path(&gateway_jsonl).map_err(|e| {
            err(
                HTTP_INTERNAL,
                format!("load gateway session transcript: {e}"),
            )
        })?
    } else {
        Session::new().with_persistence_path(gateway_jsonl.clone())
    }
    .with_workspace_root(work_dir);
    let session_tracer = gateway_session_tracer(&mcp.request_id, work_root);
    let async_runtime = tokio::runtime::Handle::try_current().map_err(|_| {
        err(
            HTTP_INTERNAL,
            "gateway solve requires a Tokio runtime (gateway-solve-once must call run_gateway_solve_turn inside rt.enter())",
        )
    })?;
    let mut tool_executor = DirectToolExecutor::new(
        work_dir.to_path_buf(),
        mcp,
        effective_model.clone(),
        allowed_tools,
        runtime_mcp_manager,
        runtime_mcp_tool_names,
        concurrent_mcp_tool_names,
        parallel_friendly_mcp_tool_names,
        session_tracer.clone(),
        Some(Arc::clone(&turn_timing)),
        async_runtime,
    );
    let mut policy = PermissionPolicy::new(PermissionMode::DangerFullAccess);
    for spec in mvp_tool_specs() {
        policy = policy.with_tool_requirement(spec.name.to_string(), spec.required_permission);
    }
    policy = policy.with_tool_requirement(
        REPORT_PROGRESS_TOOL_NAME.to_string(),
        PermissionMode::ReadOnly,
    );

    session
        .push_user_text(prompt)
        .map_err(|e| err(HTTP_INTERNAL, format!("push user message failed: {e}")))?;
    // Project preflight (e.g. SQLBot `mcp_start`) once per sessionId when not yet in transcript.
    if !project_preflight::preflight_satisfied(work_dir, &session) {
        project_preflight::run_first_turn_preflight(work_dir, &mut session, &mut tool_executor)?;
        let _ = orchestration_bus.preflight_done();
        if let Some(section) = gateway_schema_prompt_section(work_dir) {
            system_prompt.push(section);
        }
    }
    if let Some(section) = gateway_pool_layout_prompt_section() {
        system_prompt.push(section);
    }
    if let Some(section) = gateway_git_import_prompt_section(work_dir) {
        system_prompt.push(section);
    }

    let mut runtime =
        ConversationRuntime::new(session, api_client, tool_executor, policy, system_prompt);
    runtime = runtime.with_max_iterations(max_iterations);
    runtime = runtime.with_turn_timing(turn_timing);
    if let Some(tracer) = session_tracer {
        runtime = runtime.with_session_tracer(tracer);
    }
    // Turn deadline is enforced by the gateway pool (`timeout` on `docker exec` + `force_kill_slot`).
    let turn_result = runtime.run_turn_after_user_message(None);
    let result = turn_result.map_err(|e| {
        let message = format!("runtime prompt failed: {e}");
        otel_turn.mark_error(&message);
        err(HTTP_INTERNAL, message)
    })?;
    let message = assistant_report_text_from_turn(&result.assistant_messages);
    let mut out_json = json!({
        "model": effective_model,
        "iterations": result.iterations,
        "message": message,
        "usage": {
            "input_tokens": result.usage.input_tokens,
            "output_tokens": result.usage.output_tokens,
            "cache_creation_input_tokens": result.usage.cache_creation_input_tokens,
            "cache_read_input_tokens": result.usage.cache_read_input_tokens
        }
    });
    if let Some(route) = llm_route {
        if !route.is_null() {
            out_json["llmRoute"] = route;
        }
    }
    let output_text = serde_json::to_string(&out_json).unwrap_or_default();
    otel_turn.mark_ok(&message);
    Ok((0, output_text, Some(out_json)))
}

#[cfg(test)]
mod assistant_report_text_tests {
    use runtime::{ContentBlock, ConversationMessage, MessageRole};

    use super::assistant_report_text_from_turn;

    #[test]
    fn prefers_text_blocks() {
        let messages = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    text: "report".into(),
                },
                ContentBlock::ReasoningContent { text: "cot".into() },
            ],
            usage: None,
        }];
        assert_eq!(assistant_report_text_from_turn(&messages), "report");
    }

    #[test]
    fn falls_back_to_final_reasoning_when_text_empty() {
        let messages = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![ContentBlock::ReasoningContent {
                text: "thinking-only".into(),
            }],
            usage: None,
        }];
        assert_eq!(assistant_report_text_from_turn(&messages), "thinking-only");
    }
}

#[cfg(test)]
mod polish_output_tests {
    use runtime::{AssistantEvent, TokenUsage};

    use super::polish_output_from_events;

    #[test]
    fn polish_rejects_thinking_delta() {
        let events = vec![
            AssistantEvent::ThinkingDelta("cot".into()),
            AssistantEvent::MessageStop,
        ];
        let err = polish_output_from_events(&events, "deepseek-v4-pro").unwrap_err();
        assert!(err
            .message
            .contains("must not use tools or extended reasoning"));
    }

    #[test]
    fn polish_accepts_text_only_stream() {
        let events = vec![
            AssistantEvent::TextDelta("report".into()),
            AssistantEvent::Usage(TokenUsage::default()),
            AssistantEvent::MessageStop,
        ];
        let (text, json) = polish_output_from_events(&events, "deepseek-v4-pro").unwrap();
        assert!(text.contains("report"));
        assert_eq!(json["message"], "report");
    }
}

#[cfg(test)]
mod persistence_path_tests {
    use std::path::Path;

    use super::gateway_solve_session_persistence_path;

    #[test]
    fn gateway_solve_session_jsonl_under_dot_claw() {
        let p = gateway_solve_session_persistence_path(Path::new("/tmp/sess1"));
        assert_eq!(p, Path::new("/tmp/sess1/.claw/gateway-solve-session.jsonl"));
    }
}

#[cfg(test)]
mod parallel_friendly_decoration_tests {
    use super::{decorate_mcp_tool_description, PARALLEL_FRIENDLY_TOOL_DESCRIPTION_HINT};
    use runtime::McpTool;
    use serde_json::json;

    fn concurrent_analysis_tool() -> McpTool {
        McpTool {
            name: "mcp_question_then_analysis".to_string(),
            description: Some("Run a SQLBot analysis on a sub-question.".to_string()),
            input_schema: None,
            annotations: Some(json!({"readOnlyHint": true})),
            meta: None,
        }
    }

    fn serial_mcp_tool() -> McpTool {
        McpTool {
            name: "mcp_question".to_string(),
            description: Some("Resolve a single entity name.".to_string()),
            input_schema: None,
            annotations: None,
            meta: None,
        }
    }

    fn with_mcp_concurrency<F: FnOnce()>(value: &str, f: F) {
        let _guard = test_env_lock();
        let prev = std::env::var("CLAW_MCP_MAX_CONCURRENT").ok();
        std::env::set_var("CLAW_MCP_MAX_CONCURRENT", value);
        f();
        if let Some(v) = prev {
            std::env::set_var("CLAW_MCP_MAX_CONCURRENT", v);
        } else {
            std::env::remove_var("CLAW_MCP_MAX_CONCURRENT");
        }
    }

    fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn parallel_friendly_tool_gets_hint_appended() {
        with_mcp_concurrency("8", || {
            let tool = concurrent_analysis_tool();
            let decorated = decorate_mcp_tool_description(&tool, tool.description.clone())
                .expect("decorated description is Some");
            assert!(decorated.starts_with("Run a SQLBot analysis on a sub-question."));
            assert!(decorated.contains("[parallel-friendly]"));
            assert!(decorated.contains(PARALLEL_FRIENDLY_TOOL_DESCRIPTION_HINT));
        });
    }

    #[test]
    fn parallel_friendly_tool_without_original_description_still_gets_hint() {
        with_mcp_concurrency("8", || {
            let mut tool = concurrent_analysis_tool();
            tool.description = None;
            let decorated = decorate_mcp_tool_description(&tool, tool.description.clone())
                .expect("decorated description is Some");
            assert!(decorated.contains("[parallel-friendly]"));
        });
    }

    #[test]
    fn non_parallel_tool_description_is_unchanged() {
        let tool = serial_mcp_tool();
        let decorated = decorate_mcp_tool_description(&tool, tool.description.clone());
        assert_eq!(decorated.as_deref(), Some("Resolve a single entity name."));
        let mut bare = serial_mcp_tool();
        bare.description = None;
        assert_eq!(
            decorate_mcp_tool_description(&bare, bare.description.clone()),
            None
        );
    }

    #[test]
    fn decoration_is_idempotent() {
        with_mcp_concurrency("8", || {
            let tool = concurrent_analysis_tool();
            let first = decorate_mcp_tool_description(&tool, Some("Run analysis.".to_string()))
                .expect("first decoration is Some");
            let mut decorated = concurrent_analysis_tool();
            decorated.description = Some(first.clone());
            let twice = decorate_mcp_tool_description(&decorated, decorated.description.clone())
                .expect("second decoration is Some");
            assert_eq!(first, twice);
            assert_eq!(twice.matches("[parallel-friendly]").count(), 1);
        });
    }

    #[test]
    fn serial_when_mcp_max_concurrent_is_one() {
        with_mcp_concurrency("1", || {
            let tool = concurrent_analysis_tool();
            let decorated = decorate_mcp_tool_description(&tool, tool.description.clone());
            assert_eq!(
                decorated.as_deref(),
                Some("Run a SQLBot analysis on a sub-question.")
            );
        });
    }
}

#[cfg(test)]
mod extra_session_tests {
    use super::normalize_extra_session;
    use serde_json::json;

    #[test]
    fn normalize_inserts_empty_org_id_when_missing() {
        let out = normalize_extra_session(Some(json!({"store_id": "S1"}))).unwrap();
        assert_eq!(out["org_id"], "");
        assert_eq!(out["store_id"], "S1");
    }

    #[test]
    fn normalize_preserves_explicit_org_id() {
        let out = normalize_extra_session(Some(json!({"org_id": "O99"}))).unwrap();
        assert_eq!(out["org_id"], "O99");
    }

    #[test]
    fn normalize_none_becomes_empty_org_id_only() {
        let out = normalize_extra_session(None).unwrap();
        assert_eq!(out, json!({"org_id": ""}));
    }
}

#[cfg(test)]
mod gateway_solve_task_file_tests {
    use super::GatewaySolveTaskFile;
    use serde_json::json;

    #[test]
    fn gateway_solve_task_file_serde_roundtrip() {
        let t = GatewaySolveTaskFile {
            request_id: "r1".into(),
            user_prompt: "hello".into(),
            model: Some("claude-sonnet-4-6".into()),
            timeout_seconds: Some(120),
            extra_session: Some(json!({"k": 1})),
            allowed_tools: Some(vec!["bash".into()]),
            max_iterations: Some(4),
            turn_id: "T_a1b2c3d4e5f6478990abcdef12345678".into(),
            session_id: Some("sess-1".into()),
            pool_id: None,
            worker_name: None,
            llm_route: None,
            otel_traceparent: None,
        };
        let v = serde_json::to_value(&t).unwrap();
        let back: GatewaySolveTaskFile = serde_json::from_value(v).unwrap();
        assert_eq!(t.request_id, back.request_id);
        assert_eq!(t.user_prompt, back.user_prompt);
        assert_eq!(t.model, back.model);
        assert_eq!(t.timeout_seconds, back.timeout_seconds);
        assert_eq!(t.max_iterations, back.max_iterations);
    }
}

#[cfg(test)]
mod direct_api_client_tool_tests {
    use api::ToolDefinition;
    use serde_json::json;

    use super::{
        dedupe_tool_definitions_by_name, is_tool_allowed, report_progress_tool_definition,
        REPORT_PROGRESS_TOOL_NAME,
    };

    fn tool_named(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: Some(description.to_string()),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn dedupe_keeps_first_occurrence_per_name() {
        let tools = vec![
            tool_named("report_progress", "first"),
            tool_named("read_file", "read"),
            tool_named("report_progress", "second"),
        ];
        let out = dedupe_tool_definitions_by_name(tools);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "report_progress");
        assert_eq!(out[0].description.as_deref(), Some("first"));
        assert_eq!(out[1].name, "read_file");
    }

    #[test]
    fn dedupe_preserves_order_when_no_duplicates() {
        let tools = vec![
            tool_named("read_file", "read"),
            tool_named("StructuredOutput", "out"),
        ];
        let out = dedupe_tool_definitions_by_name(tools);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "read_file");
        assert_eq!(out[1].name, "StructuredOutput");
    }

    #[test]
    fn narrator_style_duplicate_report_progress_collapses_to_one() {
        let allowed = vec![REPORT_PROGRESS_TOOL_NAME.to_string()];
        let mut tools = vec![report_progress_tool_definition()];
        if is_tool_allowed(REPORT_PROGRESS_TOOL_NAME, &allowed) {
            tools.push(report_progress_tool_definition());
        }
        let out = dedupe_tool_definitions_by_name(tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, REPORT_PROGRESS_TOOL_NAME);
    }
}
