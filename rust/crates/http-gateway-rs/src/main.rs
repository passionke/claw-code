//! Axum gateway: single-binary integration surface (keeps clippy noise localized).
#![allow(clippy::too_many_lines)]
#![allow(clippy::type_complexity)]
#![allow(clippy::result_large_err)]
#![allow(clippy::await_holding_lock)]
#![allow(clippy::format_push_string)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::implicit_clone)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::unnecessary_filter_map)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::{
    ContentBlockDelta, InputContentBlock, InputMessage, MessageRequest, OutputContentBlock,
    ProviderClient, StreamEvent, ToolChoice, ToolDefinition, ToolResultContentBlock,
};
use axum::extract::{Extension, Path as AxumPath, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use runtime::{
    apply_config_env_if_unset, load_system_prompt, ApiClient as RuntimeApiClient, ApiRequest,
    AssistantEvent, ConfigLoader, ContentBlock, ConversationMessage, ConversationRuntime,
    McpServerManager, MessageRole, PermissionMode, PermissionPolicy, RuntimeConfig, Session,
    ToolError, ToolExecutor as RuntimeToolExecutor,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use telemetry::{JsonlTelemetrySink, SessionTracer};
use tokio::fs;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;
use tokio::time::timeout;
use tools::{
    execute_mcp_tool_with_extra_session, execute_tool, initialize_mcp_bridge, mvp_tool_specs,
};
use tower_http::trace::TraceLayer;
use tracing::field::Empty;
use tracing::{info, warn};
use uuid::Uuid;

fn default_system_date() -> String {
    match option_env!("BUILD_DATE") {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => current_utc_date(),
    }
}

/// Session id from `claw-session-id` (fallback `x-request-id`) or generated. kejiqing
#[derive(Clone)]
struct HttpRequestId(pub String);

#[derive(Clone)]
struct RunSolveContext {
    request_id: String,
    task_id: Option<String>,
}

#[derive(Clone)]
struct AppState {
    tasks: Arc<Mutex<HashMap<String, TaskInner>>>,
    injected_mcp: Arc<Mutex<HashMap<i64, HashMap<String, Value>>>>,
    ds_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    cfg: Arc<GatewayConfig>,
}

#[derive(Clone)]
struct GatewayConfig {
    claw_bin: String,
    work_root: PathBuf,
    ds_registry_path: PathBuf,
    default_timeout_seconds: u64,
    default_max_iterations: usize,
    default_http_mcp_name: Option<String>,
    default_http_mcp_url: Option<String>,
    default_http_mcp_transport: String,
    config_mcp_servers: HashMap<String, Value>,
    allowed_tools: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SolveRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "userPrompt")]
    user_prompt: String,
    model: Option<String>,
    #[serde(rename = "timeoutSeconds")]
    timeout_seconds: Option<u64>,
    #[serde(rename = "extraSession")]
    extra_session: Option<Value>,
    #[serde(rename = "allowedTools")]
    allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SolveResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    #[serde(rename = "durationMs")]
    duration_ms: i64,
    #[serde(rename = "clawExitCode")]
    claw_exit_code: i32,
    #[serde(rename = "outputText")]
    output_text: String,
    #[serde(rename = "outputJson")]
    output_json: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SolveAsyncResponse {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    status: String,
    #[serde(rename = "pollUrl")]
    poll_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InitRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateProjectClaudeRequest {
    content: String,
}

#[derive(Debug, Serialize)]
struct InitResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    initialized: bool,
}

#[derive(Debug, Serialize)]
struct ProjectClaudeResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    path: String,
    exists: bool,
    content: String,
}

#[derive(Debug, Serialize)]
struct EffectivePromptResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    sections: Vec<String>,
    message: String,
}

/// In-memory task row plus a handle to abort the async worker (not serialized). kejiqing
struct TaskInner {
    record: TaskRecord,
    /// Present while `queued` / `running`; cleared when the worker finishes or after cancel.
    cancel: Option<AbortHandle>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TaskRecord {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    status: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "startedAtMs")]
    started_at_ms: Option<i64>,
    #[serde(rename = "finishedAtMs")]
    finished_at_ms: Option<i64>,
    result: Option<SolveResponse>,
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ProbeQuery {
    #[serde(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct BizAdviceReportQuery {
    task_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BizAdviceReportResponse {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sourceRequestId")]
    source_request_id: String,
    #[serde(rename = "sourceDsId")]
    source_ds_id: i64,
    #[serde(rename = "sourceStatus")]
    source_status: String,
    #[serde(rename = "reportText")]
    report_text: String,
    #[serde(rename = "reportJson")]
    report_json: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct DeleteQuery {
    server_names: Option<String>,
    #[serde(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InjectMcpRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, Value>,
    replace: Option<bool>,
}

#[derive(Debug, Serialize)]
struct McpResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "injectedServerNames")]
    injected_server_names: Vec<String>,
    loaded: bool,
    #[serde(rename = "missingServers")]
    missing_servers: Vec<String>,
    #[serde(rename = "configuredServers")]
    configured_servers: i64,
    status: String,
    #[serde(rename = "mcpReport")]
    mcp_report: Value,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
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
    ) -> Result<Self, ApiError> {
        let provider = ProviderClient::from_model(&model).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("provider init failed: {e}"),
            )
        })?;
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
    fn stream(
        &mut self,
        request: ApiRequest,
    ) -> Result<Vec<AssistantEvent>, runtime::RuntimeError> {
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
            tokio::runtime::Handle::current().block_on(stream_events(&self.provider, &req))
        })
        .map_err(|e| runtime::RuntimeError::new(e.to_string()))
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
            let result = response
                .result
                .ok_or_else(|| ToolError::new("MCP tool call returned no result"))?;
            return serde_json::to_string(&result)
                .map_err(|e| ToolError::new(format!("serialize MCP result failed: {e}")));
        }
        let parsed = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
        execute_tool(tool_name, &parsed).map_err(ToolError::new)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "detail": self.message }))).into_response()
    }
}

fn init_tracing() {
    let filter = if let Ok(level) = std::env::var("CLAW_LOG_LEVEL") {
        tracing_subscriber::EnvFilter::new(level)
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    let format = std::env::var("CLAW_LOG_FORMAT")
        .unwrap_or_else(|_| "json".to_string())
        .trim()
        .to_ascii_lowercase();
    if format == "json" {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_current_span(false)
            .with_target(true)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    }
}

async fn inject_http_request_id(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get("claw-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            req.headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    req.extensions_mut().insert(HttpRequestId(id.clone()));
    let mut res = next.run(req).await;
    if let Ok(value) = http::HeaderValue::from_str(&id) {
        res.headers_mut()
            .insert(http::header::HeaderName::from_static("x-request-id"), value);
    }
    if let Ok(value) = http::HeaderValue::from_str(&id) {
        res.headers_mut().insert(
            http::header::HeaderName::from_static("claw-session-id"),
            value,
        );
    }
    res
}

#[tokio::main]
async fn main() {
    init_tracing();

    let cfg = GatewayConfig {
        claw_bin: std::env::var("CLAW_BIN").unwrap_or_else(|_| "claw".to_string()),
        work_root: PathBuf::from(
            std::env::var("CLAW_WORK_ROOT").unwrap_or_else(|_| "/tmp/claw-workspace".to_string()),
        ),
        ds_registry_path: PathBuf::from(std::env::var("CLAW_DS_REGISTRY").unwrap_or_else(|_| {
            "third_party/claw-http-gateway/http_gateway/config/datasources.example.yaml".to_string()
        })),
        default_timeout_seconds: std::env::var("CLAW_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120),
        default_max_iterations: std::env::var("CLAW_MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(64),
        default_http_mcp_name: std::env::var("CLAW_DEFAULT_HTTP_MCP_NAME")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        default_http_mcp_url: std::env::var("CLAW_DEFAULT_HTTP_MCP_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        default_http_mcp_transport: std::env::var("CLAW_DEFAULT_HTTP_MCP_TRANSPORT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| v == "http" || v == "sse")
            .unwrap_or_else(|| "http".to_string()),
        config_mcp_servers: load_mcp_servers_from_claw_config(),
        allowed_tools: parse_allowed_tools(std::env::var("CLAW_ALLOWED_TOOLS").ok()),
    };
    let state = AppState {
        tasks: Arc::new(Mutex::new(HashMap::new())),
        injected_mcp: Arc::new(Mutex::new(HashMap::new())),
        ds_locks: Arc::new(Mutex::new(HashMap::new())),
        cfg: Arc::new(cfg),
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/docs", get(docs))
        .route("/dos", get(docs))
        .route("/openapi.json", get(openapi))
        .route("/healthz", get(healthz))
        .route("/v1/init", post(init_workspace))
        .route("/v1/solve", post(solve))
        .route("/v1/solve_async", post(solve_async))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route("/v1/biz_advice_report", get(get_biz_advice_report))
        .route(
            "/v1/project/claude/{ds_id}",
            get(get_project_claude_md).post(update_project_claude_md),
        )
        .route(
            "/v1/project/prompt/{ds_id}/effective",
            get(get_effective_prompt).post(post_effective_prompt),
        )
        .route("/v1/mcp/inject", post(inject_mcp))
        .route("/v1/mcp/injected/{ds_id}", get(get_injected_mcp))
        .route("/v1/mcp/injected/{ds_id}", delete(delete_injected_mcp))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &http::Request<axum::body::Body>| {
                    let request_id = request
                        .extensions()
                        .get::<HttpRequestId>()
                        .map_or("-", |h| h.0.as_str());
                    tracing::info_span!(
                        "http_request",
                        http.method = %request.method(),
                        http.uri = %request.uri(),
                        http.version = ?request.version(),
                        request_id = %request_id,
                        http.status_code = Empty,
                        latency_ms = Empty,
                    )
                })
                .on_response(
                    |response: &http::Response<axum::body::Body>,
                     latency: std::time::Duration,
                     span: &tracing::Span| {
                        span.record(
                            "http.status_code",
                            tracing::field::display(response.status().as_u16()),
                        );
                        span.record("latency_ms", latency.as_millis() as u64);
                    },
                ),
        )
        .layer(middleware::from_fn(inject_http_request_id))
        .with_state(state);

    let addr = std::env::var("CLAW_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    info!("http gateway rs listening on {}", addr);
    axum::serve(listener, app).await.expect("start axum");
}

async fn root() -> Html<&'static str> {
    Html("<h3>claw gateway rs</h3><p>Open <a href=\"/docs\">/docs</a> to view all endpoints.</p>")
}

async fn docs() -> Html<String> {
    let rows = [
        ("GET", "/", "Gateway welcome page"),
        ("GET", "/docs", "API docs page"),
        ("GET", "/dos", "Alias of /docs"),
        ("GET", "/openapi.json", "OpenAPI-style JSON"),
        ("GET", "/healthz", "Health check"),
        ("POST", "/v1/init", "Initialize workspace for dsId"),
        ("POST", "/v1/solve", "Run sync solve"),
        ("POST", "/v1/solve_async", "Create async solve task"),
        ("GET", "/v1/tasks/{task_id}", "Get async task status"),
        (
            "POST",
            "/v1/tasks/{task_id}/cancel",
            "Cancel a queued or running async solve task",
        ),
        (
            "GET",
            "/v1/biz_advice_report?task_id=xx",
            "Generate cleaned final report from async task output",
        ),
        (
            "GET",
            "/v1/project/claude/{ds_id}",
            "Get project CLAUDE.md for ds",
        ),
        (
            "POST",
            "/v1/project/claude/{ds_id}",
            "Update project CLAUDE.md for ds",
        ),
        (
            "GET",
            "/v1/project/prompt/{ds_id}/effective",
            "Get effective system prompt for ds",
        ),
        (
            "POST",
            "/v1/project/prompt/{ds_id}/effective",
            "Reload and get effective system prompt for ds",
        ),
        ("POST", "/v1/mcp/inject", "Inject MCP servers"),
        ("GET", "/v1/mcp/injected/{ds_id}", "Get MCP servers for ds"),
        (
            "DELETE",
            "/v1/mcp/injected/{ds_id}",
            "Delete MCP servers for ds",
        ),
    ];
    let mut body = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>claw gateway docs</title></head><body>\
         <h2>claw gateway rs - API docs</h2>\
         <p>OpenAPI JSON: <a href=\"/openapi.json\">/openapi.json</a></p>\
         <table border=\"1\" cellpadding=\"8\" cellspacing=\"0\">\
         <tr><th>Method</th><th>Path</th><th>Description</th></tr>",
    );
    for (method, path, desc) in rows {
        body.push_str(&format!(
            "<tr><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
            method, path, desc
        ));
    }
    body.push_str("</table></body></html>");
    Html(body)
}

async fn openapi() -> Json<Value> {
    Json(json!({
        "openapi": "3.0.0",
        "info": {
            "title": "claw gateway rs",
            "version": "0.1.0"
        },
        "components": {
            "schemas": {
                "SolveRequest": {
                    "type": "object",
                    "required": ["dsId", "userPrompt"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1, "description": "Datasource ID" },
                        "userPrompt": { "type": "string", "minLength": 1, "description": "User prompt text" },
                        "model": { "type": "string", "nullable": true, "description": "Optional model override" },
                        "timeoutSeconds": { "type": "integer", "format": "int64", "nullable": true, "description": "Optional timeout in seconds" },
                        "allowedTools": {
                            "type": "array",
                            "nullable": true,
                            "description": "Optional per-request tool allowlist/patterns; applied to both /v1/solve and /v1/solve_async.",
                            "items": { "type": "string" }
                        }
                    }
                },
                "InitRequest": {
                    "type": "object",
                    "required": ["dsId"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1, "description": "Datasource ID" }
                    }
                },
                "InitResponse": {
                    "type": "object",
                    "required": ["dsId", "workDir", "initialized"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "initialized": { "type": "boolean" }
                    }
                },
                "UpdateProjectClaudeRequest": {
                    "type": "object",
                    "required": ["content"],
                    "properties": {
                        "content": { "type": "string", "description": "CLAUDE.md content" }
                    }
                },
                "ProjectClaudeResponse": {
                    "type": "object",
                    "required": ["dsId", "workDir", "path", "exists", "content"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "path": { "type": "string" },
                        "exists": { "type": "boolean" },
                        "content": { "type": "string" }
                    }
                },
                "EffectivePromptResponse": {
                    "type": "object",
                    "required": ["dsId", "workDir", "sections", "message"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "sections": { "type": "array", "items": { "type": "string" } },
                        "message": { "type": "string" }
                    }
                },
                "SolveResponse": {
                    "type": "object",
                    "required": ["sessionId", "requestId", "dsId", "workDir", "durationMs", "clawExitCode", "outputText"],
                    "properties": {
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "durationMs": { "type": "integer", "format": "int64" },
                        "clawExitCode": { "type": "integer", "format": "int32" },
                        "outputText": { "type": "string" },
                        "outputJson": { "type": "object", "nullable": true }
                    }
                },
                "SolveAsyncResponse": {
                    "type": "object",
                    "required": ["taskId", "sessionId", "requestId", "status", "pollUrl"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "status": { "type": "string" },
                        "pollUrl": { "type": "string" }
                    }
                },
                "InjectMcpRequest": {
                    "type": "object",
                    "required": ["dsId", "mcpServers"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1 },
                        "mcpServers": { "type": "object", "description": "MCP server config map" },
                        "replace": { "type": "boolean", "nullable": true }
                    }
                },
                "McpResponse": {
                    "type": "object",
                    "required": ["sessionId", "requestId", "dsId", "injectedServerNames", "loaded", "missingServers", "configuredServers", "status", "mcpReport"],
                    "properties": {
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "dsId": { "type": "integer", "format": "int64" },
                        "injectedServerNames": { "type": "array", "items": { "type": "string" } },
                        "loaded": { "type": "boolean" },
                        "missingServers": { "type": "array", "items": { "type": "string" } },
                        "configuredServers": { "type": "integer", "format": "int64" },
                        "status": { "type": "string" },
                        "mcpReport": { "type": "object" }
                    }
                },
                "TaskRecord": {
                    "type": "object",
                    "required": ["taskId", "sessionId", "requestId", "status", "createdAtMs"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "status": { "type": "string" },
                        "createdAtMs": { "type": "integer", "format": "int64" },
                        "startedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "finishedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "result": { "$ref": "#/components/schemas/SolveResponse", "nullable": true },
                        "error": { "type": "object", "nullable": true }
                    }
                },
                "BizAdviceReportResponse": {
                    "type": "object",
                    "required": ["taskId", "sourceRequestId", "sourceDsId", "sourceStatus", "reportText"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sourceRequestId": { "type": "string" },
                        "sourceDsId": { "type": "integer", "format": "int64" },
                        "sourceStatus": { "type": "string" },
                        "reportText": { "type": "string" },
                        "reportJson": { "type": "object", "nullable": true }
                    }
                }
            }
        },
        "paths": {
            "/": { "get": { "summary": "Gateway welcome page" } },
            "/docs": { "get": { "summary": "API docs page" } },
            "/dos": { "get": { "summary": "Alias of /docs" } },
            "/openapi.json": { "get": { "summary": "OpenAPI-style JSON" } },
            "/healthz": {
                "get": {
                    "summary": "Health check",
                    "responses": {
                        "200": { "description": "Gateway health and runtime settings", "content": { "application/json": { "schema": { "type": "object" } } } }
                    }
                }
            },
            "/v1/init": {
                "post": {
                    "summary": "Initialize workspace for dsId",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InitRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Workspace initialized", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InitResponse" } } } }
                    }
                }
            },
            "/v1/solve": {
                "post": {
                    "summary": "Run sync solve",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Solve finished", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveResponse" } } } }
                    }
                }
            },
            "/v1/solve_async": {
                "post": {
                    "summary": "Create async solve task",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Task created", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveAsyncResponse" } } } }
                    }
                }
            },
            "/v1/tasks/{task_id}": {
                "get": {
                    "summary": "Get async task status",
                    "parameters": [
                        { "name": "task_id", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Task status", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TaskRecord" } } } }
                    }
                }
            },
            "/v1/tasks/{task_id}/cancel": {
                "post": {
                    "summary": "Cancel a queued or running async solve task",
                    "parameters": [
                        { "name": "task_id", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Task marked cancelled", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TaskRecord" } } } },
                        "400": { "description": "Task already finished" },
                        "404": { "description": "Unknown task id" }
                    }
                }
            },
            "/v1/biz_advice_report": {
                "get": {
                    "summary": "Generate cleaned business advice report from async task output",
                    "parameters": [
                        { "name": "task_id", "in": "query", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Cleaned business advice report", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BizAdviceReportResponse" } } } }
                    }
                }
            },
            "/v1/project/claude/{ds_id}": {
                "get": {
                    "summary": "Get project CLAUDE.md for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Current CLAUDE.md", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ProjectClaudeResponse" } } } }
                    }
                },
                "post": {
                    "summary": "Update project CLAUDE.md for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateProjectClaudeRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Updated CLAUDE.md", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ProjectClaudeResponse" } } } }
                    }
                }
            },
            "/v1/project/prompt/{ds_id}/effective": {
                "get": {
                    "summary": "Get effective system prompt for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Effective system prompt", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EffectivePromptResponse" } } } }
                    }
                },
                "post": {
                    "summary": "Reload and get effective system prompt for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Effective system prompt", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EffectivePromptResponse" } } } }
                    }
                }
            },
            "/v1/mcp/inject": {
                "post": {
                    "summary": "Inject MCP servers",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InjectMcpRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Injection result", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/McpResponse" } } } }
                    }
                }
            },
            "/v1/mcp/injected/{ds_id}": {
                "get": {
                    "summary": "Get MCP servers for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } },
                        { "name": "probe_timeout_seconds", "in": "query", "required": false, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "MCP status", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/McpResponse" } } } }
                    }
                },
                "delete": {
                    "summary": "Delete MCP servers for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } },
                        { "name": "server_names", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "probe_timeout_seconds", "in": "query", "required": false, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Delete result", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/McpResponse" } } } }
                    }
                }
            }
        }
    }))
}

async fn healthz(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "clawBin": state.cfg.claw_bin,
        "workRoot": state.cfg.work_root.display().to_string(),
        "registryPath": state.cfg.ds_registry_path.display().to_string(),
        "defaultTimeoutSeconds": state.cfg.default_timeout_seconds,
        "defaultMaxIterations": state.cfg.default_max_iterations,
        "defaultHttpMcpName": state.cfg.default_http_mcp_name,
        "defaultHttpMcpUrl": state.cfg.default_http_mcp_url,
        "defaultHttpMcpTransport": state.cfg.default_http_mcp_transport,
        "allowedTools": state.cfg.allowed_tools
    }))
}

async fn solve(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Json(req): Json<SolveRequest>,
) -> Result<Json<SolveResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    info!(
        request_id = %request_id,
        ds_id = req.ds_id,
        endpoint = "/v1/solve",
        phase = "accepted",
        "gateway_solve"
    );
    let result = run_solve_request(
        state,
        req,
        RunSolveContext {
            request_id,
            task_id: None,
        },
    )
    .await?;
    Ok(Json(result))
}

async fn init_workspace(
    State(state): State<AppState>,
    Json(req): Json<InitRequest>,
) -> Result<Json<InitResponse>, ApiError> {
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = state.cfg.work_root.join(format!("ds_{}", req.ds_id));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let lock = get_ds_lock(&state, req.ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let settings = build_settings(&state, req.ds_id).await;
    let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize settings failed: {e}"),
        )
    })?;
    fs::write(work_dir.join(".claw/settings.json"), settings_content)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write settings failed: {e}"),
            )
        })?;
    let claude_md_path = work_dir.join("CLAUDE.md");
    match fs::metadata(&claude_md_path).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::write(&claude_md_path, "").await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write CLAUDE.md failed: {e}"),
                )
            })?;
        }
        Err(error) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("stat CLAUDE.md failed: {error}"),
            ));
        }
    }
    Ok(Json(InitResponse {
        ds_id: req.ds_id,
        work_dir: work_dir.display().to_string(),
        initialized: true,
    }))
}

async fn get_project_claude_md(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<ProjectClaudeResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = state.cfg.work_root.join(format!("ds_{ds_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let claude_md_path = work_dir.join("CLAUDE.md");
    let content = fs::read_to_string(&claude_md_path).await;
    let (exists, content) = match content {
        Ok(text) => (true, text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
        Err(error) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read CLAUDE.md failed: {error}"),
            ));
        }
    };
    Ok(Json(ProjectClaudeResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        path: claude_md_path.display().to_string(),
        exists,
        content,
    }))
}

async fn update_project_claude_md(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Json(req): Json<UpdateProjectClaudeRequest>,
) -> Result<Json<ProjectClaudeResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = state.cfg.work_root.join(format!("ds_{ds_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let claude_md_path = work_dir.join("CLAUDE.md");
    fs::write(&claude_md_path, req.content.as_bytes())
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write CLAUDE.md failed: {e}"),
            )
        })?;
    Ok(Json(ProjectClaudeResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        path: claude_md_path.display().to_string(),
        exists: true,
        content: req.content,
    }))
}

async fn get_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id)
        .await
        .map(Json)
}

async fn post_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id)
        .await
        .map(Json)
}

async fn build_effective_prompt_response(
    state: &AppState,
    ds_id: i64,
) -> Result<EffectivePromptResponse, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = state.cfg.work_root.join(format!("ds_{ds_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let lock = get_ds_lock(state, ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let sections = load_system_prompt(
        work_dir.to_path_buf(),
        default_system_date(),
        std::env::consts::OS,
        "unknown",
        None,
    )
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load system prompt failed: {e}"),
        )
    })?;
    let message = sections.join("\n\n");
    Ok(EffectivePromptResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        sections,
        message,
    })
}

async fn solve_async(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Json(req): Json<SolveRequest>,
) -> Result<Json<SolveAsyncResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    // Keep async tracking stable: taskId is the same logical id as requestId.
    let task_id = request_id.clone();
    info!(
        request_id = %request_id,
        task_id = %task_id,
        ds_id = req.ds_id,
        endpoint = "/v1/solve_async",
        phase = "queued",
        "gateway_solve_async"
    );
    {
        let mut tasks = state.tasks.lock().await;
        tasks.insert(
            task_id.clone(),
            TaskInner {
                record: TaskRecord {
                    task_id: task_id.clone(),
                    session_id: request_id.clone(),
                    request_id: request_id.clone(),
                    status: "queued".to_string(),
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                    result: None,
                    error: None,
                },
                cancel: None,
            },
        );
    }
    let state_clone = state.clone();
    let task_id_for_worker = task_id.clone();
    let rid = request_id.clone();
    let join = tokio::spawn(async move {
        {
            let mut tasks = state_clone.tasks.lock().await;
            if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
                if inner.record.status == "cancelled" {
                    inner.cancel = None;
                    return;
                }
                inner.record.status = "running".to_string();
                inner.record.started_at_ms = Some(now_ms());
            }
        }
        info!(
            request_id = %rid,
            task_id = %task_id_for_worker,
            phase = "running",
            "gateway_solve_async"
        );
        let result = run_solve_request(
            state_clone.clone(),
            req,
            RunSolveContext {
                request_id: rid.clone(),
                task_id: Some(task_id_for_worker.clone()),
            },
        )
        .await;
        let mut tasks = state_clone.tasks.lock().await;
        if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
            inner.cancel = None;
            if inner.record.status == "cancelled" {
                return;
            }
            inner.record.finished_at_ms = Some(now_ms());
            match result {
                Ok(v) => {
                    let duration_ms = v.duration_ms;
                    inner.record.status = "succeeded".to_string();
                    inner.record.result = Some(v);
                    info!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "succeeded",
                        duration_ms,
                        "gateway_solve_async"
                    );
                }
                Err(e) => {
                    inner.record.status = "failed".to_string();
                    inner.record.error =
                        Some(json!({"status_code": e.status.as_u16(), "detail": e.message}));
                    warn!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "failed",
                        status_code = e.status.as_u16(),
                        error = %e.message,
                        "gateway_solve_async"
                    );
                }
            }
        }
    });
    let cancel = join.abort_handle();
    {
        let mut tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get_mut(&task_id) {
            inner.cancel = Some(cancel);
        }
    }
    Ok(Json(SolveAsyncResponse {
        task_id: task_id.clone(),
        session_id: request_id.clone(),
        request_id: request_id.clone(),
        status: "queued".to_string(),
        poll_url: format!("/v1/tasks/{task_id}"),
    }))
}

async fn get_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    let tasks = state.tasks.lock().await;
    let task = tasks
        .get(&task_id)
        .map(|inner| inner.record.clone())
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
        })?;
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        task_request_id = %task.request_id,
        task_status = %task.status,
        endpoint = "/v1/tasks/{task_id}",
        phase = "poll",
        "gateway_task"
    );
    Ok(Json(task))
}

async fn cancel_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    let mut tasks = state.tasks.lock().await;
    let inner = tasks.get_mut(&task_id).ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
    })?;
    match inner.record.status.as_str() {
        "succeeded" | "failed" | "cancelled" => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "task {} is already finished (status: {})",
                    task_id, inner.record.status
                ),
            ));
        }
        _ => {}
    }
    if let Some(h) = inner.cancel.take() {
        h.abort();
    }
    inner.record.status = "cancelled".to_string();
    inner.record.finished_at_ms = Some(now_ms());
    inner.record.result = None;
    inner.record.error = Some(json!({"detail": "cancelled by client"}));
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        endpoint = "/v1/tasks/{task_id}/cancel",
        phase = "cancel",
        "gateway_task"
    );
    Ok(Json(inner.record.clone()))
}

async fn get_biz_advice_report(
    State(state): State<AppState>,
    Query(query): Query<BizAdviceReportQuery>,
) -> Result<Json<BizAdviceReportResponse>, ApiError> {
    let task = {
        let tasks = state.tasks.lock().await;
        tasks
            .get(&query.task_id)
            .map(|inner| inner.record.clone())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    format!("task not found: {}", query.task_id),
                )
            })?
    };
    let source_status = task.status.clone();
    if source_status != "succeeded" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} is not succeeded yet (status: {})",
                query.task_id, source_status
            ),
        ));
    }
    let source_result = task.result.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} has no result yet (status: {})",
                query.task_id, source_status
            ),
        )
    })?;
    let raw_json = source_result.output_json.as_ref().map_or_else(
        || "null".to_string(),
        |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    );
    let prompt = format!(
        "你是资深业务分析顾问。下面给你一段原始输出，其中包含中间过程、思考草稿或噪声信息。\n\
请只输出“最终干净报告”，要求：\n\
1) 不要输出任何中间过程、思考轨迹、工具调用痕迹。\n\
2) 结构清晰，使用简洁中文。\n\
3) 保留关键结论、依据与可执行建议。\n\
4) 如果信息不足，明确写出“信息不足”并给出最小补充数据清单。\n\
5) 不要添加与原文无关的事实。\n\n\
【原始文本输出】\n{}\n\n\
【原始 JSON 输出】\n{}",
        source_result.output_text, raw_json
    );
    let report = run_solve_request(
        state,
        SolveRequest {
            ds_id: source_result.ds_id,
            user_prompt: prompt,
            model: None,
            timeout_seconds: None,
            extra_session: None,
            allowed_tools: None,
        },
        RunSolveContext {
            request_id: Uuid::new_v4().simple().to_string(),
            task_id: None,
        },
    )
    .await?;
    Ok(Json(BizAdviceReportResponse {
        task_id: query.task_id,
        source_request_id: task.request_id,
        source_ds_id: source_result.ds_id,
        source_status,
        report_text: report.output_text,
        report_json: report.output_json,
    }))
}

async fn run_solve_request(
    state: AppState,
    req: SolveRequest,
    ctx: RunSolveContext,
) -> Result<SolveResponse, ApiError> {
    let RunSolveContext {
        request_id,
        task_id,
    } = ctx;
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if req.user_prompt.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "userPrompt cannot be empty",
        ));
    }
    if let Some(extra_session) = &req.extra_session {
        if !extra_session.is_object() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "extraSession must be a JSON object when present",
            ));
        }
        if let Ok(serialized) = serde_json::to_vec(extra_session) {
            if serialized.len() > 8 * 1024 {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "extraSession is too large (max 8KB)",
                ));
            }
        }
    }
    let started = Instant::now();
    info!(
        request_id = %request_id,
        task_id = task_id.as_deref().unwrap_or("-"),
        ds_id = req.ds_id,
        phase = "solve_run_start",
        "gateway_solve"
    );
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);
    let effective_allowed_tools =
        resolve_effective_allowed_tools(&state.cfg.allowed_tools, req.allowed_tools.as_deref())?;
    validate_ds_exists(req.ds_id, &state.cfg.ds_registry_path).await?;

    let work_dir = state.cfg.work_root.join(format!("ds_{}", req.ds_id));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;

    let ds_lock = get_ds_lock(&state, req.ds_id).await;
    let _guard = ds_lock.lock().await;

    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let settings = build_settings(&state, req.ds_id).await;
    let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize settings failed: {e}"),
        )
    })?;
    fs::write(work_dir.join(".claw/settings.json"), settings_content)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write settings failed: {e}"),
            )
        })?;
    let _ = fs::remove_file(work_dir.join(".claw/mcp_discovery_cache.json")).await;

    let (code, output_text, output_json) = run_runtime_prompt(
        &work_dir,
        &req.user_prompt,
        req.model.as_deref(),
        timeout_seconds,
        &request_id,
        req.extra_session.clone(),
        effective_allowed_tools,
        state.cfg.default_max_iterations,
    )?;
    let duration_ms = started.elapsed().as_millis() as i64;
    info!(
        request_id = %request_id,
        task_id = task_id.as_deref().unwrap_or("-"),
        ds_id = req.ds_id,
        phase = "solve_run_ok",
        duration_ms,
        "gateway_solve"
    );
    Ok(SolveResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id: req.ds_id,
        work_dir: work_dir.display().to_string(),
        duration_ms,
        claw_exit_code: code,
        output_text,
        output_json,
    })
}

async fn inject_mcp(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Json(req): Json<InjectMcpRequest>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let replace = req.replace.unwrap_or(false);
    {
        let mut injected = state.injected_mcp.lock().await;
        if replace {
            injected.insert(req.ds_id, req.mcp_servers.clone());
        } else {
            let current = injected.entry(req.ds_id).or_default();
            for (k, v) in req.mcp_servers {
                current.insert(k, v);
            }
        }
    }
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, req.ds_id, 15).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id: req.ds_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

async fn get_injected_mcp(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Query(query): Query<ProbeQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    let timeout_seconds = query.probe_timeout_seconds.unwrap_or(15);
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, ds_id, timeout_seconds).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

async fn delete_injected_mcp(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    {
        let mut injected = state.injected_mcp.lock().await;
        if let Some(names) = query.server_names {
            let targets = names
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let current = injected.entry(ds_id).or_default();
            for name in targets {
                current.remove(&name);
            }
            if current.is_empty() {
                injected.remove(&ds_id);
            }
        } else {
            injected.remove(&ds_id);
        }
    }
    let timeout_seconds = query.probe_timeout_seconds.unwrap_or(15);
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, ds_id, timeout_seconds).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

async fn apply_settings_and_probe(
    state: &AppState,
    ds_id: i64,
    probe_timeout_seconds: u64,
) -> Result<(Value, Vec<String>, i64, String, Vec<String>), ApiError> {
    let work_dir = state.cfg.work_root.join(format!("ds_{ds_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let lock = get_ds_lock(state, ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let settings = build_settings(state, ds_id).await;
    let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize settings failed: {e}"),
        )
    })?;
    fs::write(work_dir.join(".claw/settings.json"), settings_content)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write settings failed: {e}"),
            )
        })?;
    let _ = fs::remove_file(work_dir.join(".claw/mcp_discovery_cache.json")).await;
    let (report, loaded_names, configured_servers, status) =
        probe_mcp_load(&state.cfg.claw_bin, &work_dir, probe_timeout_seconds).await?;
    let names = {
        let injected = state.injected_mcp.lock().await;
        injected
            .get(&ds_id)
            .map(|v| v.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    Ok((report, loaded_names, configured_servers, status, names))
}

async fn build_settings(state: &AppState, ds_id: i64) -> Value {
    let mut servers = HashMap::<String, Value>::new();
    for (k, v) in &state.cfg.config_mcp_servers {
        servers.insert(k.clone(), v.clone());
    }
    if let (Some(name), Some(url)) = (
        state.cfg.default_http_mcp_name.as_ref(),
        state.cfg.default_http_mcp_url.as_ref(),
    ) {
        servers.insert(
            name.clone(),
            json!({
                "type": state.cfg.default_http_mcp_transport,
                "url": url
            }),
        );
    }
    let injected = state.injected_mcp.lock().await;
    if let Some(extra) = injected.get(&ds_id) {
        for (k, v) in extra {
            servers.insert(k.clone(), v.clone());
        }
    }
    json!({ "mcpServers": servers })
}

async fn ensure_workspace_initialized(_claw_bin: &str, work_dir: &Path) -> Result<(), ApiError> {
    let marker = work_dir.join(".claw/.gateway_init_done");
    if fs::metadata(&marker).await.is_ok() {
        return Ok(());
    }
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("workspace init failed: {e}"),
            )
        })?;
    fs::write(marker, now_ms().to_string()).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write init marker failed: {e}"),
        )
    })?;
    Ok(())
}

fn initialize_mcp_runtime(
    work_dir: &Path,
) -> Result<
    (
        Vec<ToolDefinition>,
        HashSet<String>,
        Option<Arc<StdMutex<McpServerManager>>>,
    ),
    ApiError,
> {
    let runtime_cfg = ConfigLoader::default_for(work_dir).load().map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load runtime config failed: {e}"),
        )
    })?;
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

/// Runtime JSONL trace for gateway solves; mirrors CLI env (`CLAW_TRACE_*`). kejiqing
fn gateway_trace_file_path(trace_id: &str) -> Option<PathBuf> {
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
    let dir = std::env::var("CLAW_TRACE_DIR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/var/log/claw/traces".to_string());
    Some(PathBuf::from(dir).join(format!("{trace_id}.ndjson")))
}

fn gateway_session_tracer(request_id: &str) -> Option<SessionTracer> {
    let trace_id = std::env::var("CLAW_TRACE_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| request_id.to_string());
    let path = gateway_trace_file_path(&trace_id)?;
    let sink = JsonlTelemetrySink::new(path).ok()?;
    Some(SessionTracer::new(trace_id, Arc::new(sink)))
}

#[allow(clippy::too_many_arguments)]
fn run_runtime_prompt(
    work_dir: &Path,
    prompt: &str,
    model: Option<&str>,
    timeout_seconds: u64,
    clawcode_session_id: &str,
    extra_session: Option<Value>,
    allowed_tools: Vec<String>,
    max_iterations: usize,
) -> Result<(i32, String, Option<Value>), ApiError> {
    std::env::set_current_dir(work_dir).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("set current dir failed: {e}"),
        )
    })?;
    // Project config: `CLAW_PROJECT_CONFIG_ROOT` or parent of `CLAW_CONFIG_FILE` (set by deploy; no path hardcoded).
    let project_cfg = match project_config_loader_root() {
        Some(root) => ConfigLoader::default_for(&root).load().map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
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
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load system prompt failed: {e}"),
        )
    })?;
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
    let mut runtime = ConversationRuntime::new(
        Session::new().with_workspace_root(work_dir),
        api_client,
        tool_executor,
        policy,
        system_prompt,
    );
    runtime = runtime.with_max_iterations(max_iterations);
    if let Some(tracer) = gateway_session_tracer(clawcode_session_id) {
        runtime = runtime.with_session_tracer(tracer);
    }
    let started = Instant::now();
    let result = runtime.run_turn(prompt, None).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("runtime prompt failed: {e}"),
        )
    })?;
    if started.elapsed() > Duration::from_secs(timeout_seconds) {
        return Err(ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            format!("claw prompt timeout: {timeout_seconds}s"),
        ));
    }
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

async fn probe_mcp_load(
    claw_bin: &str,
    work_dir: &Path,
    timeout_seconds: u64,
) -> Result<(Value, Vec<String>, i64, String), ApiError> {
    let mut cmd = Command::new(claw_bin);
    cmd.current_dir(work_dir)
        .arg("mcp")
        .arg("--output-format")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(Duration::from_secs(timeout_seconds), cmd.output())
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                format!("claw mcp probe timeout: {timeout_seconds}s"),
            )
        })?
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawn claw mcp failed: {e}"),
            )
        })?;
    let raw = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let parsed = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({"raw": raw}));
    let loaded_names = parsed
        .get("servers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let configured_servers = parsed
        .get("configured_servers")
        .and_then(Value::as_i64)
        .unwrap_or(loaded_names.len() as i64);
    let status = parsed
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if output.status.success() {
            "ok"
        } else {
            "error"
        })
        .to_string();
    Ok((parsed, loaded_names, configured_servers, status))
}

async fn get_ds_lock(state: &AppState, ds_id: i64) -> Arc<Mutex<()>> {
    let mut locks = state.ds_locks.lock().await;
    locks
        .entry(ds_id)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

async fn validate_ds_exists(ds_id: i64, path: &Path) -> Result<(), ApiError> {
    if fs::metadata(path).await.is_err() {
        warn!("datasource registry not found: {}", path.display());
        return Ok(());
    }
    let text = fs::read_to_string(path).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read datasource registry failed: {e}"),
        )
    })?;
    let parsed = serde_yaml::from_str::<Value>(&text).unwrap_or_else(|_| json!({}));
    if let Some(ds) = parsed
        .get("datasources")
        .and_then(Value::as_object)
        .and_then(|m| m.get(&ds_id.to_string()))
    {
        if ds.is_object() {
            return Ok(());
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn current_utc_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let days_since_epoch = (now.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

// Computes civil (Gregorian) year/month/day from days since the Unix epoch
// (1970-01-01) using Howard Hinnant's `civil_from_days` algorithm.
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
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

/// Config directory for `ConfigLoader` (e.g. contains `.claw.json`). Set via env only. kejiqing
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

fn load_mcp_servers_from_claw_config() -> HashMap<String, Value> {
    let Ok(path) = std::env::var("CLAW_CONFIG_FILE") else {
        return HashMap::new();
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return HashMap::new(),
    };
    let parsed = match serde_json::from_str::<Value>(&raw) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let mut out = HashMap::new();
    let Some(mcp) = parsed.get("mcpServers").and_then(Value::as_object) else {
        return out;
    };
    for (name, cfg) in mcp {
        out.insert(name.clone(), cfg.clone());
    }
    out
}

fn parse_allowed_tools(raw: Option<String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for token in raw.split(',') {
        let name = normalize_allowed_tool_name(token);
        if name.is_empty() {
            continue;
        }
        if !values.contains(&name) {
            values.push(name);
        }
    }
    values
}

fn normalize_allowed_tool_name(raw: &str) -> String {
    let name = raw.trim();
    match name {
        "read" | "ReadFile" | "ead_file" => "read_file".to_string(),
        "glob" | "GlobSearch" | "glob_searchr" => "glob_search".to_string(),
        "grep" | "GrepSearch" => "grep_search".to_string(),
        "MCPTool" => "MCP".to_string(),
        "ListMcpResourcesToolMCP" => "ListMcpResources".to_string(),
        other => other.to_string(),
    }
}

fn resolve_effective_allowed_tools(
    global_allowed_tools: &[String],
    requested_allowed_tools: Option<&[String]>,
) -> Result<Vec<String>, ApiError> {
    let Some(requested) = requested_allowed_tools else {
        return Ok(global_allowed_tools.to_vec());
    };

    let mut normalized = Vec::new();
    for raw in requested {
        let name = normalize_allowed_tool_name(raw);
        if name.is_empty() {
            continue;
        }
        if !normalized.contains(&name) {
            normalized.push(name);
        }
    }
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if global_allowed_tools.is_empty() {
        return Ok(normalized);
    }

    for requested in &normalized {
        let allowed = if requested.ends_with('*') {
            global_allowed_tools.contains(requested)
        } else {
            is_tool_allowed(requested, global_allowed_tools)
        };
        if !allowed {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("requested tool pattern is not allowed by gateway policy: {requested}"),
            ));
        }
    }
    Ok(normalized)
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

async fn stream_events(
    provider: &ProviderClient,
    req: &MessageRequest,
) -> Result<Vec<AssistantEvent>, api::ApiError> {
    let mut stream = provider.stream_message(req).await?;
    let mut events = Vec::new();
    let mut pending_tools: HashMap<u32, (String, String, String)> = HashMap::new();
    while let Some(event) = stream.next_event().await? {
        match event {
            StreamEvent::MessageStart(start) => {
                for block in start.message.content {
                    match block {
                        OutputContentBlock::Text { text } => {
                            if !text.is_empty() {
                                events.push(AssistantEvent::TextDelta(text));
                            }
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
                    if !text.is_empty() {
                        events.push(AssistantEvent::TextDelta(text));
                    }
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
                    if !text.is_empty() {
                        events.push(AssistantEvent::TextDelta(text));
                    }
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
