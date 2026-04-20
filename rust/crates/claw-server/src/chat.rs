use std::collections::BTreeSet;
use std::convert::Infallible;

use api::{
    max_tokens_for_model, metadata_for_model, resolve_model_alias, OpenAiCompatConfig,
    PromptCache, ProviderClient,
};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tools::GlobalToolRegistry;

use runtime::{
    ConfigLoader, ConversationRuntime, PermissionEnforcer, PermissionMode, PermissionPolicy,
    RuntimeFeatureConfig, Session, SessionStore, ToolError, ToolExecutor, ToolWorkspaceRootGuard,
    TurnSummary,
};

use crate::auth::require_user;
use crate::blocking_client::BlockingRoundTripClient;
use crate::crypto;
use crate::error::ServerError;
use crate::profiles::{load_profile, ProviderProfileRow};
use crate::server::AppState;
use crate::workspaces::load_workspace_for_user;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub workspace_id: String,
    pub provider_profile_id: String,
    pub prompt: String,
    pub session_id: Option<String>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub session_id: String,
    pub session_path: String,
    pub reply: String,
    pub iterations: usize,
}

struct WebToolExecutor {
    registry: GlobalToolRegistry,
    allowed_tools: Option<BTreeSet<String>>,
}

impl ToolExecutor for WebToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if self
            .allowed_tools
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(tool_name))
        {
            return Err(ToolError::new(format!(
                "tool `{tool_name}` is not enabled for this request"
            )));
        }
        let value = serde_json::from_str(input)
            .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
        self.registry
            .execute(tool_name, &value)
            .map_err(ToolError::new)
    }
}

fn build_permission_policy(
    mode: PermissionMode,
    feature_config: &RuntimeFeatureConfig,
    registry: &GlobalToolRegistry,
) -> Result<PermissionPolicy, ServerError> {
    let specs = registry
        .permission_specs(None)
        .map_err(|e| ServerError::Internal(e))?;
    Ok(specs.into_iter().fold(
        PermissionPolicy::new(mode).with_permission_rules(feature_config.permission_rules()),
        |policy, (name, required_permission)| {
            policy.with_tool_requirement(name, required_permission)
        },
    ))
}

fn build_provider_client(
    master_key: &str,
    row: &ProviderProfileRow,
    session_id: &str,
) -> Result<ProviderClient, ServerError> {
    let key = crypto::decrypt_secret(master_key, &row.api_key_ciphertext)?;
    let client = match row.provider_kind.as_str() {
        "anthropic" => ProviderClient::from_explicit_anthropic(key, row.base_url.clone())
            .with_prompt_cache(PromptCache::new(session_id)),
        "openai_compat" | "dashscope" | "xai" => {
            let base = row.base_url.clone().ok_or_else(|| {
                ServerError::BadRequest("profile missing base_url".into())
            })?;
            match row.provider_kind.as_str() {
                "xai" => ProviderClient::from_explicit_xai(key, base),
                "dashscope" => ProviderClient::from_explicit_openai_compat(
                    key,
                    base,
                    OpenAiCompatConfig::dashscope(),
                ),
                _ => {
                    let model = resolve_model_alias(&row.model);
                    let config = match metadata_for_model(&model) {
                        Some(m) if m.auth_env == "DASHSCOPE_API_KEY" => {
                            OpenAiCompatConfig::dashscope()
                        }
                        _ => OpenAiCompatConfig::openai(),
                    };
                    ProviderClient::from_explicit_openai_compat(key, base, config)
                }
            }
        }
        other => {
            return Err(ServerError::BadRequest(format!(
                "unsupported provider_kind: {other}"
            )));
        }
    };
    Ok(client)
}

fn final_assistant_text(summary: &TurnSummary) -> String {
    summary
        .assistant_messages
        .last()
        .map(|message| {
            message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    runtime::ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

pub async fn chat_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, ServerError> {
    if body.workspace_id.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "workspace_id is required for every prompt".into(),
        ));
    }
    if body.provider_profile_id.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "provider_profile_id is required".into(),
        ));
    }
    let user = require_user(&state, &headers).await?;
    let (_ws, root) = load_workspace_for_user(&state, &user, &body.workspace_id).await?;
    let profile = load_profile(&state, &user.id, &body.provider_profile_id).await?;

    let _guard = ToolWorkspaceRootGuard::enter(root.clone());

    let store = SessionStore::from_cwd(&root).map_err(|e| ServerError::Internal(e.to_string()))?;

    let (mut session, session_path) = if let Some(ref sid) = body.session_id {
        if sid == "new" || sid.is_empty() {
            let mut s = Session::new().with_workspace_root(root.clone());
            let h = store.create_handle(&s.session_id);
            let path = h.path.clone();
            s = s.with_persistence_path(path.clone());
            s.save_to_path(&path)
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            (s, path)
        } else {
            let loaded = store
                .load_session(sid)
                .map_err(|e| ServerError::BadRequest(e.to_string()))?;
            (loaded.session, loaded.handle.path)
        }
    } else {
        let mut s = Session::new().with_workspace_root(root.clone());
        let h = store.create_handle(&s.session_id);
        let path = h.path.clone();
        s = s.with_persistence_path(path.clone());
        s.save_to_path(&path)
            .map_err(|e| ServerError::Internal(e.to_string()))?;
        (s, path)
    };

    if session.model.is_none() {
        session.model = Some(profile.model.clone());
    }

    let loader = ConfigLoader::default_for(&root);
    let runtime_config = loader
        .load()
        .unwrap_or_else(|_| runtime::RuntimeConfig::empty());
    let feature_config = runtime_config.feature_config().clone();

    let mut tool_registry = GlobalToolRegistry::builtin();
    let policy = build_permission_policy(
        PermissionMode::WorkspaceWrite,
        &feature_config,
        &tool_registry,
    )?;
    tool_registry.set_enforcer(PermissionEnforcer::new(policy.clone()));

    let model = profile.model.clone();
    let max_tokens = max_tokens_for_model(&resolve_model_alias(&model));
    let provider_client = build_provider_client(
        &state.master_key,
        &profile,
        &session.session_id,
    )?;

    let tool_definitions = tool_registry.definitions(None);
    let api_client = BlockingRoundTripClient::new(
        provider_client,
        session.session_id.clone(),
        model.clone(),
        max_tokens,
        true,
        None,
        tool_definitions,
    )
    .map_err(|e| ServerError::Internal(e))?;

    let tool_executor = WebToolExecutor {
        registry: tool_registry,
        allowed_tools: None,
    };

    let system_prompt = runtime::load_system_prompt(
        &root,
        chrono::Utc::now().format("%Y-%m-%d").to_string(),
        std::env::consts::OS,
        "web",
    )
    .map_err(|e| ServerError::Internal(e.to_string()))?;

    let mut runtime = ConversationRuntime::new_with_features(
        session,
        api_client,
        tool_executor,
        policy,
        system_prompt,
        &feature_config,
    );

    let summary = runtime
        .run_turn(body.prompt.trim().to_string(), None)
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let sid = runtime.session().session_id.clone();
    runtime
        .session()
        .save_to_path(&session_path)
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let reply = final_assistant_text(&summary);
    Ok(Json(ChatResponse {
        session_id: sid,
        session_path: session_path.to_string_lossy().into_owned(),
        reply,
        iterations: summary.iterations,
    }))
}

#[derive(Deserialize)]
pub struct ChatSseQuery {
    pub workspace_id: String,
    pub provider_profile_id: String,
    pub prompt: String,
    pub session_id: Option<String>,
}

pub async fn chat_sse(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<ChatSseQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ServerError> {
    let workspace_id = q.workspace_id.clone();
    if workspace_id.trim().is_empty() {
        return Err(ServerError::BadRequest(
            "workspace_id is required".into(),
        ));
    }
    let user = require_user(&state, &headers).await?;
    let (_ws, root) = load_workspace_for_user(&state, &user, &workspace_id).await?;
    let profile = load_profile(&state, &user.id, &q.provider_profile_id).await?;
    let profile = profile.clone();

    let prompt = q.prompt.clone();
    let session_id_opt = q.session_id.clone();
    let master_key = state.master_key.clone();

    let stream = async_stream::stream! {
        let _guard = ToolWorkspaceRootGuard::enter(root.clone());
        let result: Result<String, ServerError> = (|| {
            let store = SessionStore::from_cwd(&root).map_err(|e| ServerError::Internal(e.to_string()))?;
            let (mut session, session_path) = if let Some(ref sid) = session_id_opt {
                if sid == "new" || sid.is_empty() {
                    let mut s = Session::new().with_workspace_root(root.clone());
                    let h = store.create_handle(&s.session_id);
                    let path = h.path.clone();
                    s = s.with_persistence_path(path.clone());
                    s.save_to_path(&path).map_err(|e| ServerError::Internal(e.to_string()))?;
                    (s, path)
                } else {
                    let loaded = store.load_session(sid).map_err(|e| ServerError::BadRequest(e.to_string()))?;
                    (loaded.session, loaded.handle.path)
                }
            } else {
                let mut s = Session::new().with_workspace_root(root.clone());
                let h = store.create_handle(&s.session_id);
                let path = h.path.clone();
                s = s.with_persistence_path(path.clone());
                s.save_to_path(&path).map_err(|e| ServerError::Internal(e.to_string()))?;
                (s, path)
            };

            if session.model.is_none() {
                session.model = Some(profile.model.clone());
            }

            let loader = ConfigLoader::default_for(&root);
            let runtime_config = loader.load().unwrap_or_else(|_| runtime::RuntimeConfig::empty());
            let feature_config = runtime_config.feature_config().clone();
            let mut tool_registry = GlobalToolRegistry::builtin();
            let policy = build_permission_policy(
                PermissionMode::WorkspaceWrite,
                &feature_config,
                &tool_registry,
            )?;
            tool_registry.set_enforcer(PermissionEnforcer::new(policy.clone()));

            let model = profile.model.clone();
            let max_tokens = max_tokens_for_model(&resolve_model_alias(&model));
            let provider_client = build_provider_client(&master_key, &profile, &session.session_id)?;
            let tool_definitions = tool_registry.definitions(None);
            let api_client = BlockingRoundTripClient::new(
                provider_client,
                session.session_id.clone(),
                model.clone(),
                max_tokens,
                true,
                None,
                tool_definitions,
            ).map_err(|e| ServerError::Internal(e))?;
            let tool_executor = WebToolExecutor { registry: tool_registry, allowed_tools: None };
            let system_prompt = runtime::load_system_prompt(
                &root,
                chrono::Utc::now().format("%Y-%m-%d").to_string(),
                std::env::consts::OS,
                "web",
            ).map_err(|e| ServerError::Internal(e.to_string()))?;

            let mut runtime = ConversationRuntime::new_with_features(
                session,
                api_client,
                tool_executor,
                policy,
                system_prompt,
                &feature_config,
            );
            let summary = runtime.run_turn(prompt.trim().to_string(), None)
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            runtime.session().save_to_path(&session_path).map_err(|e| ServerError::Internal(e.to_string()))?;
            let reply = final_assistant_text(&summary);
            let payload = json!({
                "session_id": runtime.session().session_id,
                "session_path": session_path.to_string_lossy(),
                "reply": reply,
                "iterations": summary.iterations,
            });
            Ok(payload.to_string())
        })();

        match result {
            Ok(text) => {
                for chunk in text.as_bytes().chunks(64) {
                    let piece = String::from_utf8_lossy(chunk);
                    yield Ok(Event::default().data(piece.to_string()));
                }
                yield Ok(Event::default().event("done").data("[DONE]"));
            }
            Err(e) => {
                let msg = serde_json::json!({"error": e.to_string()}).to_string();
                yield Ok(Event::default().event("error").data(msg));
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
