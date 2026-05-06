use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::{
    ContentBlockDelta, InputContentBlock, InputMessage, MessageRequest, OutputContentBlock,
    ProviderClient, StreamEvent, ToolChoice, ToolDefinition, ToolResultContentBlock,
};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use runtime::{
    ApiClient as RuntimeApiClient, ApiRequest, AssistantEvent, ConfigLoader, ContentBlock,
    ConversationMessage, ConversationRuntime, McpServerManager, MessageRole, PermissionMode,
    PermissionPolicy, Session, ToolError, ToolExecutor as RuntimeToolExecutor, load_system_prompt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tools::{execute_tool, initialize_mcp_bridge, mvp_tool_specs};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

const DEFAULT_SYSTEM_DATE: &str = match option_env!("BUILD_DATE") {
    Some(value) if !value.is_empty() => value,
    _ => "1970-01-01",
};

#[derive(Clone)]
struct AppState {
    tasks: Arc<Mutex<HashMap<String, TaskRecord>>>,
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SolveResponse {
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

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TaskRecord {
    #[serde(rename = "taskId")]
    task_id: String,
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
    runtime_mcp_manager: Option<Arc<StdMutex<McpServerManager>>>,
    runtime_mcp_tool_names: HashSet<String>,
}

impl RuntimeToolExecutor for DirectToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if !is_tool_allowed(tool_name, &self.allowed_tools) {
            return Err(ToolError::new(format!("tool not allowed: {tool_name}")));
        }
        if self.runtime_mcp_tool_names.contains(tool_name) {
            let args = serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
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
                        .call_tool(tool_name, Some(args))
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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

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
        .layer(TraceLayer::new_for_http())
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
                        "timeoutSeconds": { "type": "integer", "format": "int64", "nullable": true, "description": "Optional timeout in seconds" }
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
                    "required": ["requestId", "dsId", "workDir", "durationMs", "clawExitCode", "outputText"],
                    "properties": {
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
                    "required": ["taskId", "requestId", "status", "pollUrl"],
                    "properties": {
                        "taskId": { "type": "string" },
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
                    "required": ["requestId", "dsId", "injectedServerNames", "loaded", "missingServers", "configuredServers", "status", "mcpReport"],
                    "properties": {
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
                    "required": ["taskId", "requestId", "status", "createdAtMs"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "status": { "type": "string" },
                        "createdAtMs": { "type": "integer", "format": "int64" },
                        "startedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "finishedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "result": { "$ref": "#/components/schemas/SolveResponse", "nullable": true },
                        "error": { "type": "object", "nullable": true }
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
    Json(req): Json<SolveRequest>,
) -> Result<Json<SolveResponse>, ApiError> {
    let request_id = Uuid::new_v4().simple().to_string();
    let result = run_solve_request(state, req, request_id).await?;
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
    build_effective_prompt_response(&state, ds_id).await.map(Json)
}

async fn post_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id).await.map(Json)
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
        DEFAULT_SYSTEM_DATE.to_string(),
        std::env::consts::OS,
        "unknown",
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
    Json(req): Json<SolveRequest>,
) -> Result<Json<SolveAsyncResponse>, ApiError> {
    let task_id = Uuid::new_v4().simple().to_string();
    {
        let mut tasks = state.tasks.lock().await;
        tasks.insert(
            task_id.clone(),
            TaskRecord {
                task_id: task_id.clone(),
                request_id: task_id.clone(),
                status: "queued".to_string(),
                created_at_ms: now_ms(),
                started_at_ms: None,
                finished_at_ms: None,
                result: None,
                error: None,
            },
        );
    }
    let state_clone = state.clone();
    let task_id_for_worker = task_id.clone();
    tokio::spawn(async move {
        {
            let mut tasks = state_clone.tasks.lock().await;
            if let Some(task) = tasks.get_mut(&task_id_for_worker) {
                task.status = "running".to_string();
                task.started_at_ms = Some(now_ms());
            }
        }
        let result = run_solve_request(state_clone.clone(), req, task_id_for_worker.clone()).await;
        let mut tasks = state_clone.tasks.lock().await;
        if let Some(task) = tasks.get_mut(&task_id_for_worker) {
            task.finished_at_ms = Some(now_ms());
            match result {
                Ok(v) => {
                    task.status = "succeeded".to_string();
                    task.result = Some(v);
                }
                Err(e) => {
                    task.status = "failed".to_string();
                    task.error =
                        Some(json!({"status_code": e.status.as_u16(), "detail": e.message}));
                }
            }
        }
    });
    Ok(Json(SolveAsyncResponse {
        task_id: task_id.clone(),
        request_id: task_id.clone(),
        status: "queued".to_string(),
        poll_url: format!("/v1/tasks/{task_id}"),
    }))
}

async fn get_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
) -> Result<Json<TaskRecord>, ApiError> {
    let tasks = state.tasks.lock().await;
    let task = tasks.get(&task_id).cloned().ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
    })?;
    Ok(Json(task))
}

async fn run_solve_request(
    state: AppState,
    req: SolveRequest,
    request_id: String,
) -> Result<SolveResponse, ApiError> {
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if req.user_prompt.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "userPrompt cannot be empty",
        ));
    }
    let started = Instant::now();
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);
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
        state.cfg.default_max_iterations,
    )?;
    Ok(SolveResponse {
        request_id,
        ds_id: req.ds_id,
        work_dir: work_dir.display().to_string(),
        duration_ms: started.elapsed().as_millis() as i64,
        claw_exit_code: code,
        output_text,
        output_json,
    })
}

async fn inject_mcp(
    State(state): State<AppState>,
    Json(req): Json<InjectMcpRequest>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = Uuid::new_v4().simple().to_string();
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
    Query(query): Query<ProbeQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = Uuid::new_v4().simple().to_string();
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
    Query(query): Query<DeleteQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = Uuid::new_v4().simple().to_string();
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

fn run_runtime_prompt(
    work_dir: &Path,
    prompt: &str,
    model: Option<&str>,
    timeout_seconds: u64,
    clawcode_session_id: &str,
    max_iterations: usize,
) -> Result<(i32, String, Option<Value>), ApiError> {
    std::env::set_current_dir(work_dir).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("set current dir failed: {e}"),
        )
    })?;
    let effective_model = model
        .map(str::to_string)
        .or_else(|| std::env::var("CLAW_DEFAULT_MODEL").ok())
        .unwrap_or_else(|| "openai/deepseek-v4-pro".to_string());
    let system_prompt = load_system_prompt(
        work_dir.to_path_buf(),
        DEFAULT_SYSTEM_DATE.to_string(),
        std::env::consts::OS,
        "unknown",
    )
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load system prompt failed: {e}"),
        )
    })?;
    let (runtime_mcp_tools, runtime_mcp_tool_names, runtime_mcp_manager) =
        initialize_mcp_runtime(work_dir)?;
    let allowed_tools = parse_allowed_tools(std::env::var("CLAW_ALLOWED_TOOLS").ok());
    let api_client = DirectApiClient::new(
        effective_model.clone(),
        &allowed_tools,
        runtime_mcp_tools,
        clawcode_session_id.to_string(),
    )?;
    let tool_executor = DirectToolExecutor {
        allowed_tools,
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

fn load_mcp_servers_from_claw_config() -> HashMap<String, Value> {
    let path = std::env::var("CLAW_CONFIG_FILE").unwrap_or_else(|_| "/app/.claw.json".to_string());
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
        let mut name = token.trim().to_string();
        if name.is_empty() {
            continue;
        }
        name = match name.as_str() {
            "read" | "ReadFile" | "ead_file" => "read_file".to_string(),
            "glob" | "GlobSearch" | "glob_searchr" => "glob_search".to_string(),
            "grep" | "GrepSearch" => "grep_search".to_string(),
            "MCPTool" => "MCP".to_string(),
            "ListMcpResourcesToolMCP" => "ListMcpResources".to_string(),
            other => other.to_string(),
        };
        if !values.contains(&name) {
            values.push(name);
        }
    }
    values
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
