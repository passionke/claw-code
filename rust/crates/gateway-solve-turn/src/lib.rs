//! One-turn gateway solve (shared by `http-gateway-rs` in-process path and `claw gateway-solve-once`).
//! Author: kejiqing
#![allow(
    clippy::await_holding_lock,
    clippy::cast_possible_wrap,
    clippy::match_same_arms,
    clippy::result_large_err,
    clippy::type_complexity,
    clippy::unnecessary_filter_map
)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use api::{
    ContentBlockDelta, InputContentBlock, InputMessage, MessageRequest, OutputContentBlock,
    ProviderClient, StreamEvent, ToolChoice, ToolDefinition, ToolResultContentBlock,
};
use runtime::{
    apply_config_env_if_unset, load_system_prompt, ApiClient as RuntimeApiClient, ApiRequest,
    AssistantEvent, ConfigLoader, ContentBlock, ConversationMessage, ConversationRuntime,
    McpServerManager, MessageRole, PermissionMode, PermissionPolicy, RuntimeConfig, RuntimeError,
    Session, ToolError, ToolExecutor as RuntimeToolExecutor,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use telemetry::{JsonlTelemetrySink, SessionTracer};
use tools::{
    execute_mcp_tool_with_extra_session, execute_tool, initialize_mcp_bridge, mvp_tool_specs,
};

const HTTP_INTERNAL: u16 = 500;

/// DeepSeek-official routing for `/v1/biz_advice_report` polish only (`REPORT_LLM_PROVIDER=deepseek`). kejiqing
#[derive(Clone, Debug)]
pub struct ReportPolishDeepseek {
    pub api_key: String,
    pub model: String,
}

/// Fixed `ds_id` for boss-report skill content (`home/skills/GPOS_BOSS_REPORT_WRITER/SKILL.md`). kejiqing
pub const BOSS_REPORT_SKILL_DS_ID: i64 = 1;

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

fn err(status: u16, msg: impl Into<String>) -> GatewaySolveTurnError {
    GatewaySolveTurnError {
        status,
        message: msg.into(),
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
}

fn default_system_date() -> String {
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
    clippy::cast_possible_truncation
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

fn project_config_loader_root() -> Option<PathBuf> {
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

fn gateway_session_tracer(request_id: &str, work_root: &Path) -> Option<SessionTracer> {
    let trace_id = std::env::var("CLAW_TRACE_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| request_id.to_string());
    let path = gateway_trace_file_path(&trace_id, work_root)?;
    let sink = JsonlTelemetrySink::new(path).ok()?;
    Some(SessionTracer::new(trace_id, Arc::new(sink)))
}

fn initialize_mcp_runtime(
    work_dir: &Path,
) -> Result<
    (
        Vec<ToolDefinition>,
        HashSet<String>,
        Option<Arc<StdMutex<McpServerManager>>>,
    ),
    GatewaySolveTurnError,
> {
    let runtime_cfg = ConfigLoader::default_for(work_dir)
        .load()
        .map_err(|e| err(HTTP_INTERNAL, format!("load runtime config failed: {e}")))?;
    let mut manager = McpServerManager::from_runtime_config(&runtime_cfg);
    if manager.server_names().is_empty() && manager.unsupported_servers().is_empty() {
        return Ok((Vec::new(), HashSet::new(), None));
    }

    let report = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(async { manager.discover_tools_best_effort().await })
    });

    let manager = Arc::new(StdMutex::new(manager));
    initialize_mcp_bridge(Arc::clone(&manager), &report);

    let mut runtime_mcp_tools = Vec::new();
    let mut runtime_mcp_tool_names = HashSet::new();
    for discovered in report.tools {
        let name = discovered.qualified_name;
        let input_schema = discovered
            .tool
            .input_schema
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        runtime_mcp_tools.push(ToolDefinition {
            name: name.clone(),
            description: discovered.tool.description,
            input_schema,
        });
        runtime_mcp_tool_names.insert(name);
    }

    Ok((runtime_mcp_tools, runtime_mcp_tool_names, Some(manager)))
}

struct DirectApiClient {
    model: String,
    provider: ProviderClient,
    tools: Vec<ToolDefinition>,
    clawcode_session_id: String,
}

impl DirectApiClient {
    fn new(
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
        Ok(Self {
            model,
            provider,
            tools,
            clawcode_session_id,
        })
    }
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
            ..Default::default()
        };
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(stream_events::<fn(&str)>(
                &self.provider,
                &req,
                None,
            ))
        })
        .map_err(|e| RuntimeError::new(e.to_string()))
    }
}

struct DirectToolExecutor {
    allowed_tools: Vec<String>,
    extra_session: Option<Value>,
    runtime_mcp_manager: Option<Arc<StdMutex<McpServerManager>>>,
    runtime_mcp_tool_names: HashSet<String>,
}

impl RuntimeToolExecutor for DirectToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if !is_tool_allowed(tool_name, &self.allowed_tools) {
            return Err(ToolError::new(format!("tool not allowed: {tool_name}")));
        }
        if tool_name == "MCP" {
            return execute_mcp_tool_with_extra_session(input, self.extra_session.as_ref())
                .map_err(ToolError::new);
        }
        if self.runtime_mcp_tool_names.contains(tool_name) {
            let args = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
            let meta = self
                .extra_session
                .as_ref()
                .map(|value| json!({ "extra_session": value }));
            let Some(manager) = &self.runtime_mcp_manager else {
                return Err(ToolError::new("MCP manager not initialized"));
            };
            let manager = Arc::clone(manager);
            let response = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    let mut guard = manager
                        .lock()
                        .map_err(|_| ToolError::new("MCP manager lock poisoned"))?;
                    guard
                        .call_tool(tool_name, Some(args), meta)
                        .await
                        .map_err(|e| ToolError::new(e.to_string()))
                })
            })?;
            if let Some(error) = response.error {
                return Err(ToolError::new(format!(
                    "MCP tool call failed: {} ({})",
                    error.message, error.code
                )));
            }
            let Some(result) = response.result else {
                return Err(ToolError::new("MCP tool call returned no result"));
            };
            return serde_json::to_string(&result)
                .map_err(|e| ToolError::new(format!("serialize MCP result failed: {e}")));
        }
        let parsed = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
        execute_tool(tool_name, &parsed).map_err(ToolError::new)
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

async fn stream_events<F>(
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

fn polish_output_from_events(
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

#[allow(clippy::too_many_arguments)]
pub fn run_gateway_solve_turn(
    work_dir: &Path,
    work_root: &Path,
    prompt: &str,
    model: Option<&str>,
    _timeout_seconds: u64,
    clawcode_session_id: &str,
    extra_session: Option<Value>,
    allowed_tools: Vec<String>,
    max_iterations: usize,
) -> Result<(i32, String, Option<Value>), GatewaySolveTurnError> {
    std::env::set_current_dir(work_dir)
        .map_err(|e| err(HTTP_INTERNAL, format!("set current dir failed: {e}")))?;
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
    let system_prompt = load_system_prompt(
        work_dir.to_path_buf(),
        default_system_date(),
        std::env::consts::OS,
        "unknown",
        extra_session.clone(),
    )
    .map_err(|e| err(HTTP_INTERNAL, format!("load system prompt failed: {e}")))?;
    let (runtime_mcp_tools, runtime_mcp_tool_names, runtime_mcp_manager) =
        initialize_mcp_runtime(work_dir)?;
    let api_client = DirectApiClient::new(
        effective_model.clone(),
        &allowed_tools,
        runtime_mcp_tools,
        clawcode_session_id.to_string(),
    )?;
    let tool_executor = DirectToolExecutor {
        allowed_tools,
        extra_session,
        runtime_mcp_manager,
        runtime_mcp_tool_names,
    };
    let mut policy = PermissionPolicy::new(PermissionMode::DangerFullAccess);
    for spec in mvp_tool_specs() {
        policy = policy.with_tool_requirement(spec.name.to_string(), spec.required_permission);
    }
    let gateway_jsonl = gateway_solve_session_persistence_path(work_dir);
    let session = if gateway_jsonl.exists() {
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
    let mut runtime =
        ConversationRuntime::new(session, api_client, tool_executor, policy, system_prompt);
    runtime = runtime.with_max_iterations(max_iterations);
    if let Some(tracer) = gateway_session_tracer(clawcode_session_id, work_root) {
        runtime = runtime.with_session_tracer(tracer);
    }
    let result = runtime
        .run_turn(prompt, None)
        .map_err(|e| err(HTTP_INTERNAL, format!("runtime prompt failed: {e}")))?;
    let message = result
        .assistant_messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let out_json = json!({
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
    Ok((
        0,
        serde_json::to_string(&out_json).unwrap_or_default(),
        Some(out_json),
    ))
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
