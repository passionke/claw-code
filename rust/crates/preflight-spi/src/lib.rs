//! Preflight SPI v1 types and validation (shared by gateway-solve-turn and http-gateway-rs).
//! Lifecycle events: worker/session/turn × start/end via `steps[].on`. Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Deserialize)]
struct LegacyPreflightKind {
    kind: String,
}

pub const SPI_VERSION: &str = "1";

/// Known builtin handler ids (registry may extend via Admin).
pub const BUILTIN_TURN_LANGUAGE: &str = "turn_language";
pub const BUILTIN_SQLBOT_MCP_START: &str = "sqlbot_mcp_start";

/// Legacy scope (compat). Prefer [`PreflightLifecycleEvent`] via `steps[].on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightScope {
    EveryTurn,
    SessionFirstTurn,
}

/// Closed lifecycle event set for project preflight steps. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PreflightLifecycleEvent {
    #[serde(rename = "worker.init.start")]
    WorkerInitStart,
    #[serde(rename = "worker.init.end")]
    WorkerInitEnd,
    #[serde(rename = "worker.reuse.start")]
    WorkerReuseStart,
    #[serde(rename = "worker.reuse.end")]
    WorkerReuseEnd,
    #[serde(rename = "session.start")]
    SessionStart,
    #[serde(rename = "session.end")]
    SessionEnd,
    #[serde(rename = "turn.start")]
    TurnStart,
    #[serde(rename = "turn.end")]
    TurnEnd,
}

impl PreflightLifecycleEvent {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkerInitStart => "worker.init.start",
            Self::WorkerInitEnd => "worker.init.end",
            Self::WorkerReuseStart => "worker.reuse.start",
            Self::WorkerReuseEnd => "worker.reuse.end",
            Self::SessionStart => "session.start",
            Self::SessionEnd => "session.end",
            Self::TurnStart => "turn.start",
            Self::TurnEnd => "turn.end",
        }
    }

    /// Events dispatched inside `gateway-solve-once` (after Landlock).
    #[must_use]
    pub fn is_solve_path(self) -> bool {
        matches!(
            self,
            Self::SessionStart | Self::SessionEnd | Self::TurnStart | Self::TurnEnd
        )
    }

    /// Events that must run before Landlock (create / acquire).
    #[must_use]
    pub fn is_pre_jail(self) -> bool {
        matches!(
            self,
            Self::WorkerInitStart
                | Self::WorkerInitEnd
                | Self::WorkerReuseStart
                | Self::WorkerReuseEnd
        )
    }
}

#[must_use]
pub fn event_from_scope(scope: PreflightScope) -> PreflightLifecycleEvent {
    match scope {
        PreflightScope::EveryTurn => PreflightLifecycleEvent::TurnStart,
        PreflightScope::SessionFirstTurn => PreflightLifecycleEvent::SessionStart,
    }
}

#[must_use]
pub fn scope_from_event(event: PreflightLifecycleEvent) -> Option<PreflightScope> {
    match event {
        PreflightLifecycleEvent::TurnStart => Some(PreflightScope::EveryTurn),
        PreflightLifecycleEvent::SessionStart => Some(PreflightScope::SessionFirstTurn),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreflightImpl {
    Builtin { handler: String },
    Subprocess { command: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightStep {
    pub plugin_id: String,
    /// Preferred: lifecycle event. Author: kejiqing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<PreflightLifecycleEvent>,
    /// Legacy; used when `on` is absent. Author: kejiqing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PreflightScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#impl: Option<PreflightImpl>,
    #[serde(default)]
    pub config: Value,
}

impl PreflightStep {
    /// Resolve `on` (preferred) or map legacy `scope`; default `turn.start`.
    #[must_use]
    pub fn resolved_event(&self) -> PreflightLifecycleEvent {
        if let Some(on) = self.on {
            return on;
        }
        match self.scope {
            Some(s) => event_from_scope(s),
            None => PreflightLifecycleEvent::TurnStart,
        }
    }

    /// Normalize so `on` is set; keep mapped `scope` for turn/session start compat.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        let event = self.resolved_event();
        self.on = Some(event);
        self.scope = scope_from_event(event);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightPipelineConfig {
    #[serde(default)]
    pub steps: Vec<PreflightStep>,
    /// Legacy ordered kinds (`sqlbot_mcp_start`); migrated to `steps` at read time.
    #[serde(default)]
    pub kinds: Vec<String>,
}

/// Turn/session SPI context (existing shape). Author: kejiqing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightRequestContext {
    pub session_id: String,
    pub turn_id: String,
    pub work_dir: String,
    pub is_continuation: bool,
    pub user_prompt: String,
    #[serde(default)]
    pub prior_user_prompts: Vec<String>,
    #[serde(default)]
    pub extra_session: Value,
    pub model: String,
}

/// Outcome for `*.end` events. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightOutcome {
    Ok,
    Error,
}

/// Flexible lifecycle context (camelCase JSON). Field presence depends on event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEventContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proj_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_profile_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_user_prompts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_session: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_continuation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<PreflightOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Validate required / forbidden fields for an event. Author: kejiqing
pub fn validate_context_for_event(
    event: PreflightLifecycleEvent,
    ctx: &LifecycleEventContext,
) -> Result<(), String> {
    if ctx.work_dir.as_ref().is_none_or(|s| s.is_empty()) {
        return Err(format!(
            "context.workDir required for event {}",
            event.as_str()
        ));
    }
    match event {
        PreflightLifecycleEvent::WorkerInitStart | PreflightLifecycleEvent::WorkerInitEnd => {
            forbid_prompt_fields(event, ctx)?;
            if ctx.user_prompt.is_some() || ctx.turn_id.is_some() {
                return Err(format!(
                    "event {} must not include userPrompt/turnId",
                    event.as_str()
                ));
            }
            if matches!(event, PreflightLifecycleEvent::WorkerInitEnd) {
                require_end_fields(event, ctx)?;
            }
        }
        PreflightLifecycleEvent::WorkerReuseStart | PreflightLifecycleEvent::WorkerReuseEnd => {
            if ctx.user_prompt.is_some() {
                return Err(format!(
                    "event {} must not include userPrompt",
                    event.as_str()
                ));
            }
            if matches!(event, PreflightLifecycleEvent::WorkerReuseEnd) {
                require_end_fields(event, ctx)?;
            }
        }
        PreflightLifecycleEvent::SessionStart => {
            if ctx.session_id.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.sessionId required for session.start".into());
            }
        }
        PreflightLifecycleEvent::SessionEnd => {
            if ctx.session_id.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.sessionId required for session.end".into());
            }
            require_end_fields(event, ctx)?;
        }
        PreflightLifecycleEvent::TurnStart => {
            if ctx.session_id.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.sessionId required for turn.start".into());
            }
            if ctx.turn_id.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.turnId required for turn.start".into());
            }
            if ctx.user_prompt.is_none() {
                return Err("context.userPrompt required for turn.start".into());
            }
            if ctx.model.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.model required for turn.start".into());
            }
        }
        PreflightLifecycleEvent::TurnEnd => {
            if ctx.session_id.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.sessionId required for turn.end".into());
            }
            if ctx.turn_id.as_ref().is_none_or(|s| s.is_empty()) {
                return Err("context.turnId required for turn.end".into());
            }
            require_end_fields(event, ctx)?;
        }
    }
    Ok(())
}

fn forbid_prompt_fields(
    event: PreflightLifecycleEvent,
    ctx: &LifecycleEventContext,
) -> Result<(), String> {
    if ctx.prior_user_prompts.is_some() {
        return Err(format!(
            "event {} must not include priorUserPrompts",
            event.as_str()
        ));
    }
    Ok(())
}

fn require_end_fields(
    event: PreflightLifecycleEvent,
    ctx: &LifecycleEventContext,
) -> Result<(), String> {
    if ctx.outcome.is_none() {
        return Err(format!(
            "context.outcome required for {}",
            event.as_str()
        ));
    }
    if ctx.duration_ms.is_none() {
        return Err(format!(
            "context.durationMs required for {}",
            event.as_str()
        ));
    }
    if ctx.outcome == Some(PreflightOutcome::Ok) && ctx.error.is_some() {
        return Err(format!(
            "context.error must be absent when outcome=ok ({})",
            event.as_str()
        ));
    }
    Ok(())
}

/// Build a minimal worker-init context. Author: kejiqing
#[must_use]
pub fn build_worker_init_context(
    proj_id: i64,
    worker_id: &str,
    work_dir: &str,
    template_id: &str,
    sandbox_id: &str,
    worker_profile_mode: &str,
) -> LifecycleEventContext {
    LifecycleEventContext {
        proj_id: Some(proj_id),
        worker_id: Some(worker_id.to_string()),
        work_dir: Some(work_dir.to_string()),
        template_id: Some(template_id.to_string()),
        sandbox_id: Some(sandbox_id.to_string()),
        worker_profile_mode: Some(worker_profile_mode.to_string()),
        ..Default::default()
    }
}

/// Convert turn SPI context into lifecycle context for `turn.start`.
#[must_use]
pub fn lifecycle_context_from_turn(req: &PreflightRequestContext) -> LifecycleEventContext {
    LifecycleEventContext {
        work_dir: Some(req.work_dir.clone()),
        session_id: Some(req.session_id.clone()),
        turn_id: Some(req.turn_id.clone()),
        user_prompt: Some(req.user_prompt.clone()),
        prior_user_prompts: Some(req.prior_user_prompts.clone()),
        extra_session: Some(req.extra_session.clone()),
        model: Some(req.model.clone()),
        is_continuation: Some(req.is_continuation),
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightSpiRequest {
    #[serde(rename = "spiVersion")]
    pub spi_version: String,
    /// Firing lifecycle event. Author: kejiqing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<PreflightLifecycleEvent>,
    pub step: PreflightStep,
    pub context: PreflightRequestContext,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightResponseStatus {
    Ok,
    Skip,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PreflightEffect {
    LockLanguage {
        language: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    WriteSessionFile {
        #[serde(rename = "relPath")]
        rel_path: String,
        content: String,
    },
    AppendSystemPromptSection {
        markdown: String,
    },
    AppendTranscriptSummary {
        text: String,
    },
    InjectToolExchange {
        #[serde(rename = "toolName")]
        tool_name: String,
        input: String,
        output: String,
        #[serde(default, rename = "isError")]
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightSpiResponse {
    pub status: PreflightResponseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub effects: Vec<PreflightEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Value>,
}

/// Admin registry row shape (camelCase API).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightPluginRecord {
    pub plugin_id: String,
    pub display_name: String,
    pub spi_version: String,
    #[serde(default)]
    pub default_impl: Option<PreflightImpl>,
    #[serde(default)]
    pub config_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreflightFilterContext {
    pub is_continuation: bool,
    pub session_first_turn_satisfied: bool,
}

/// Returns step indices to run for a firing event (in order). Author: kejiqing
#[must_use]
pub fn filter_step_indices_for_event(
    steps: &[PreflightStep],
    event: PreflightLifecycleEvent,
    ctx: PreflightFilterContext,
) -> Vec<usize> {
    steps
        .iter()
        .enumerate()
        .filter(|(_, step)| should_run_for_event(step.resolved_event(), event, ctx))
        .map(|(i, _)| i)
        .collect()
}

/// Legacy: filter for solve-path `session.start` + `turn.start` only (excludes worker.*).
/// Prefer [`filter_step_indices_for_event`] for a single firing event.
#[must_use]
pub fn filter_step_indices(steps: &[PreflightStep], ctx: PreflightFilterContext) -> Vec<usize> {
    steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            let ev = step.resolved_event();
            matches!(
                ev,
                PreflightLifecycleEvent::SessionStart | PreflightLifecycleEvent::TurnStart
            ) && should_run_for_event(ev, ev, ctx)
        })
        .map(|(i, _)| i)
        .collect()
}

#[must_use]
pub fn should_run_for_event(
    step_event: PreflightLifecycleEvent,
    firing: PreflightLifecycleEvent,
    ctx: PreflightFilterContext,
) -> bool {
    if step_event != firing {
        return false;
    }
    match firing {
        PreflightLifecycleEvent::SessionStart => {
            !ctx.is_continuation && !ctx.session_first_turn_satisfied
        }
        PreflightLifecycleEvent::TurnStart
        | PreflightLifecycleEvent::TurnEnd
        | PreflightLifecycleEvent::SessionEnd
        | PreflightLifecycleEvent::WorkerInitStart
        | PreflightLifecycleEvent::WorkerInitEnd
        | PreflightLifecycleEvent::WorkerReuseStart
        | PreflightLifecycleEvent::WorkerReuseEnd => true,
    }
}

/// Legacy scope filter (compat). Author: kejiqing
#[must_use]
pub fn should_run_step(scope: PreflightScope, ctx: PreflightFilterContext) -> bool {
    let ev = event_from_scope(scope);
    should_run_for_event(ev, ev, ctx)
}

/// Session-turn effects are not valid on worker lifecycle events. Author: kejiqing
pub fn validate_effects_for_event(
    event: PreflightLifecycleEvent,
    effects: &[PreflightEffect],
) -> Result<(), String> {
    if event.is_pre_jail() {
        for effect in effects {
            if matches!(
                effect,
                PreflightEffect::LockLanguage { .. }
                    | PreflightEffect::InjectToolExchange { .. }
                    | PreflightEffect::AppendTranscriptSummary { .. }
            ) {
                return Err(format!(
                    "effect {:?} not allowed on event {}",
                    effect_type_name(effect),
                    event.as_str()
                ));
            }
        }
    }
    Ok(())
}

fn effect_type_name(effect: &PreflightEffect) -> &'static str {
    match effect {
        PreflightEffect::LockLanguage { .. } => "lockLanguage",
        PreflightEffect::WriteSessionFile { .. } => "writeSessionFile",
        PreflightEffect::AppendSystemPromptSection { .. } => "appendSystemPromptSection",
        PreflightEffect::AppendTranscriptSummary { .. } => "appendTranscriptSummary",
        PreflightEffect::InjectToolExchange { .. } => "injectToolExchange",
    }
}

/// Reject subprocess responses that declare builtin-only effects.
pub fn validate_subprocess_response(response: &PreflightSpiResponse) -> Result<(), String> {
    if response.status == PreflightResponseStatus::Error {
        return Err(response
            .message
            .clone()
            .unwrap_or_else(|| "preflight plugin returned error".to_string()));
    }
    for effect in &response.effects {
        if matches!(effect, PreflightEffect::InjectToolExchange { .. }) {
            return Err(String::from(
                "subprocess preflight must not declare injectToolExchange (builtin only)",
            ));
        }
    }
    Ok(())
}

pub fn validate_spi_request(request: &PreflightSpiRequest) -> Result<(), String> {
    if request.spi_version != SPI_VERSION {
        return Err(format!(
            "unsupported spiVersion {:?} (expected {SPI_VERSION})",
            request.spi_version
        ));
    }
    if request.step.plugin_id.trim().is_empty() {
        return Err(String::from("step.pluginId must be non-empty"));
    }
    let event = request
        .event
        .unwrap_or_else(|| request.step.resolved_event());
    if event == PreflightLifecycleEvent::TurnStart {
        let lc = lifecycle_context_from_turn(&request.context);
        validate_context_for_event(event, &lc)?;
    }
    Ok(())
}

fn normalize_kinds(raw: &[String]) -> Vec<String> {
    raw.iter()
        .map(|k| k.trim())
        .filter(|k| !k.is_empty() && *k != "none")
        .map(ToString::to_string)
        .collect()
}

fn default_turn_language_step() -> PreflightStep {
    PreflightStep {
        plugin_id: BUILTIN_TURN_LANGUAGE.to_string(),
        on: Some(PreflightLifecycleEvent::TurnStart),
        scope: Some(PreflightScope::EveryTurn),
        r#impl: Some(PreflightImpl::Builtin {
            handler: BUILTIN_TURN_LANGUAGE.to_string(),
        }),
        config: Value::Object(Map::default()),
    }
}

fn kind_to_step(kind: &str) -> Option<PreflightStep> {
    match kind {
        BUILTIN_SQLBOT_MCP_START => Some(PreflightStep {
            plugin_id: BUILTIN_SQLBOT_MCP_START.to_string(),
            on: Some(PreflightLifecycleEvent::SessionStart),
            scope: Some(PreflightScope::SessionFirstTurn),
            r#impl: Some(PreflightImpl::Builtin {
                handler: BUILTIN_SQLBOT_MCP_START.to_string(),
            }),
            config: Value::Object(Map::default()),
        }),
        BUILTIN_TURN_LANGUAGE => Some(PreflightStep {
            plugin_id: BUILTIN_TURN_LANGUAGE.to_string(),
            on: Some(PreflightLifecycleEvent::TurnStart),
            scope: Some(PreflightScope::EveryTurn),
            r#impl: Some(PreflightImpl::Builtin {
                handler: BUILTIN_TURN_LANGUAGE.to_string(),
            }),
            config: Value::Object(Map::default()),
        }),
        _ => None,
    }
}

/// Parse raw JSON (steps or legacy kinds/kind) into pipeline config.
pub fn parse_pipeline_value(value: &Value) -> Result<PreflightPipelineConfig, String> {
    if value.get("steps").is_some() {
        let cfg: PreflightPipelineConfig = serde_json::from_value(value.clone())
            .map_err(|e| format!("solvePreflightJson: {e}"))?;
        return Ok(cfg);
    }
    if value.get("kinds").is_some() {
        let cfg: PreflightPipelineConfig = serde_json::from_value(value.clone())
            .map_err(|e| format!("solvePreflightJson: {e}"))?;
        return Ok(cfg);
    }
    let legacy: LegacyPreflightKind =
        serde_json::from_value(value.clone()).map_err(|e| format!("solvePreflightJson: {e}"))?;
    Ok(PreflightPipelineConfig {
        steps: vec![],
        kinds: normalize_kinds(&[legacy.kind]),
    })
}

/// Normalize to executable `steps` (migrate legacy `kinds`; set `on` from scope).
#[must_use]
pub fn normalize_pipeline_steps(cfg: &PreflightPipelineConfig) -> Vec<PreflightStep> {
    let raw = if !cfg.steps.is_empty() {
        cfg.steps.clone()
    } else {
        let kinds = normalize_kinds(&cfg.kinds);
        if kinds.is_empty() {
            return vec![];
        }
        let mut steps = vec![default_turn_language_step()];
        for kind in kinds {
            if let Some(step) = kind_to_step(&kind) {
                if step.plugin_id == BUILTIN_TURN_LANGUAGE {
                    continue;
                }
                steps.push(step);
            }
        }
        steps
    };
    raw.into_iter().map(PreflightStep::normalized).collect()
}

/// Default pipeline when no project preflight file is mounted (language inference every turn).
#[must_use]
pub fn default_runtime_pipeline_steps() -> Vec<PreflightStep> {
    vec![default_turn_language_step()]
}

pub fn validate_pipeline_value(value: &Value) -> Result<(), String> {
    let cfg = parse_pipeline_value(value)?;
    let steps = executable_pipeline_steps(&cfg);
    for step in &steps {
        if step.plugin_id.trim().is_empty() {
            return Err(String::from(
                "solvePreflightJson steps[].pluginId must be non-empty",
            ));
        }
        let _ = step.resolved_event();
        if let Some(PreflightImpl::Builtin { handler }) = &step.r#impl {
            match handler.as_str() {
                BUILTIN_TURN_LANGUAGE | BUILTIN_SQLBOT_MCP_START => {}
                other => {
                    return Err(format!(
                        "solvePreflightJson unknown builtin handler {other:?}"
                    ));
                }
            }
        }
        if let Some(PreflightImpl::Subprocess { command }) = &step.r#impl {
            if command.is_empty() || command.iter().all(|c| c.trim().is_empty()) {
                return Err(String::from(
                    "solvePreflightJson subprocess impl requires non-empty command",
                ));
            }
        }
    }
    Ok(())
}

/// Steps to run after normalization; empty stored config defaults to `turn_language` only.
#[must_use]
pub fn executable_pipeline_steps(cfg: &PreflightPipelineConfig) -> Vec<PreflightStep> {
    let steps = normalize_pipeline_steps(cfg);
    if steps.is_empty() && cfg.steps.is_empty() && normalize_kinds(&cfg.kinds).is_empty() {
        return vec![];
    }
    if steps.is_empty() {
        return default_runtime_pipeline_steps();
    }
    steps
}

/// Materialize worker-readable `solve-preflight.json` (steps with `on`).
#[must_use]
pub fn materialize_pipeline_json(value: &Value) -> Value {
    let Ok(cfg) = parse_pipeline_value(value) else {
        return json!({ "steps": [] });
    };
    let steps = normalize_pipeline_steps(&cfg);
    if steps.is_empty() {
        return json!({ "steps": [] });
    }
    serde_json::to_value(&steps).map_or_else(
        |_| json!({ "steps": [] }),
        |steps| json!({ "steps": steps }),
    )
}

#[must_use]
pub fn has_enabled_pipeline(value: &Value) -> bool {
    parse_pipeline_value(value)
        .map(|cfg| {
            if !cfg.steps.is_empty() {
                return true;
            }
            !normalize_kinds(&cfg.kinds).is_empty()
        })
        .unwrap_or(false)
}

/// Merge `language_pipeline_json` fields into the `turn_language` step config.
#[must_use]
pub fn merge_language_pipeline_into_steps(
    mut steps: Vec<PreflightStep>,
    language_pipeline: &Value,
) -> Vec<PreflightStep> {
    if language_pipeline
        .as_object()
        .is_none_or(serde_json::Map::is_empty)
    {
        return steps;
    }
    for step in &mut steps {
        if step.plugin_id == BUILTIN_TURN_LANGUAGE {
            if let Some(obj) = language_pipeline.as_object() {
                let mut cfg = step.config.as_object().cloned().unwrap_or_default();
                for (k, v) in obj {
                    cfg.insert(k.clone(), v.clone());
                }
                step.config = Value::Object(cfg);
            }
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_matrix_session_first_turn() {
        let ctx_new = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: false,
        };
        assert!(should_run_step(PreflightScope::SessionFirstTurn, ctx_new));
        let ctx_cont = PreflightFilterContext {
            is_continuation: true,
            session_first_turn_satisfied: false,
        };
        assert!(!should_run_step(PreflightScope::SessionFirstTurn, ctx_cont));
        let ctx_sat = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: true,
        };
        assert!(!should_run_step(PreflightScope::SessionFirstTurn, ctx_sat));
        assert!(should_run_step(PreflightScope::EveryTurn, ctx_sat));
    }

    #[test]
    fn event_filter_matrix() {
        let ctx_new = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: false,
        };
        let ctx_cont = PreflightFilterContext {
            is_continuation: true,
            session_first_turn_satisfied: false,
        };
        assert!(should_run_for_event(
            PreflightLifecycleEvent::WorkerInitStart,
            PreflightLifecycleEvent::WorkerInitStart,
            ctx_new
        ));
        assert!(!should_run_for_event(
            PreflightLifecycleEvent::WorkerInitStart,
            PreflightLifecycleEvent::WorkerReuseStart,
            ctx_new
        ));
        assert!(should_run_for_event(
            PreflightLifecycleEvent::WorkerReuseStart,
            PreflightLifecycleEvent::WorkerReuseStart,
            ctx_new
        ));
        assert!(!should_run_for_event(
            PreflightLifecycleEvent::WorkerReuseStart,
            PreflightLifecycleEvent::WorkerInitStart,
            ctx_new
        ));
        assert!(should_run_for_event(
            PreflightLifecycleEvent::SessionStart,
            PreflightLifecycleEvent::SessionStart,
            ctx_new
        ));
        assert!(!should_run_for_event(
            PreflightLifecycleEvent::SessionStart,
            PreflightLifecycleEvent::SessionStart,
            ctx_cont
        ));
        assert!(should_run_for_event(
            PreflightLifecycleEvent::TurnStart,
            PreflightLifecycleEvent::TurnStart,
            ctx_cont
        ));
        assert!(!should_run_for_event(
            PreflightLifecycleEvent::TurnEnd,
            PreflightLifecycleEvent::TurnStart,
            ctx_new
        ));
        assert!(should_run_for_event(
            PreflightLifecycleEvent::TurnEnd,
            PreflightLifecycleEvent::TurnEnd,
            ctx_new
        ));
    }

    #[test]
    fn solve_path_excludes_worker_init_steps() {
        let steps = vec![
            PreflightStep {
                plugin_id: "apt".into(),
                on: Some(PreflightLifecycleEvent::WorkerInitStart),
                scope: None,
                r#impl: None,
                config: json!({}),
            },
            PreflightStep {
                plugin_id: "reuse".into(),
                on: Some(PreflightLifecycleEvent::WorkerReuseStart),
                scope: None,
                r#impl: None,
                config: json!({}),
            },
            PreflightStep {
                plugin_id: BUILTIN_SQLBOT_MCP_START.into(),
                on: Some(PreflightLifecycleEvent::SessionStart),
                scope: Some(PreflightScope::SessionFirstTurn),
                r#impl: None,
                config: json!({}),
            },
            PreflightStep {
                plugin_id: BUILTIN_TURN_LANGUAGE.into(),
                on: Some(PreflightLifecycleEvent::TurnStart),
                scope: Some(PreflightScope::EveryTurn),
                r#impl: None,
                config: json!({}),
            },
        ];
        let ctx = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: false,
        };
        let idxs = filter_step_indices(&steps, ctx);
        let plugins: Vec<_> = idxs.iter().map(|&i| steps[i].plugin_id.as_str()).collect();
        assert_eq!(plugins, vec![BUILTIN_SQLBOT_MCP_START, BUILTIN_TURN_LANGUAGE]);
        assert!(filter_step_indices_for_event(
            &steps,
            PreflightLifecycleEvent::WorkerInitStart,
            ctx
        )
        .contains(&0));
    }

    #[test]
    fn legacy_scope_maps_to_on() {
        let step = PreflightStep {
            plugin_id: "x".into(),
            on: None,
            scope: Some(PreflightScope::EveryTurn),
            r#impl: None,
            config: json!({}),
        };
        assert_eq!(step.resolved_event(), PreflightLifecycleEvent::TurnStart);
        let session = PreflightStep {
            plugin_id: "y".into(),
            on: None,
            scope: Some(PreflightScope::SessionFirstTurn),
            r#impl: None,
            config: json!({}),
        };
        assert_eq!(
            session.resolved_event(),
            PreflightLifecycleEvent::SessionStart
        );
        let on_wins = PreflightStep {
            plugin_id: "z".into(),
            on: Some(PreflightLifecycleEvent::WorkerInitStart),
            scope: Some(PreflightScope::EveryTurn),
            r#impl: None,
            config: json!({}),
        };
        assert_eq!(
            on_wins.resolved_event(),
            PreflightLifecycleEvent::WorkerInitStart
        );
    }

    #[test]
    fn migrate_kinds_to_steps_with_default_language() {
        let raw = json!({"kinds": ["sqlbot_mcp_start"]});
        let steps = normalize_pipeline_steps(&parse_pipeline_value(&raw).unwrap());
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].plugin_id, BUILTIN_TURN_LANGUAGE);
        assert_eq!(
            steps[0].resolved_event(),
            PreflightLifecycleEvent::TurnStart
        );
        assert_eq!(steps[0].on, Some(PreflightLifecycleEvent::TurnStart));
        assert_eq!(steps[1].plugin_id, BUILTIN_SQLBOT_MCP_START);
    }

    #[test]
    fn context_worker_init_forbids_prompt() {
        let mut ctx = build_worker_init_context(1, "w1", "/claw_ds", "tmpl", "sbx", "strict");
        assert!(validate_context_for_event(PreflightLifecycleEvent::WorkerInitStart, &ctx).is_ok());
        ctx.user_prompt = Some("hi".into());
        assert!(validate_context_for_event(PreflightLifecycleEvent::WorkerInitStart, &ctx).is_err());
    }

    #[test]
    fn context_turn_start_requires_prompt() {
        let ctx = LifecycleEventContext {
            work_dir: Some("/w".into()),
            session_id: Some("s".into()),
            turn_id: Some("t".into()),
            user_prompt: Some("q".into()),
            model: Some("m".into()),
            ..Default::default()
        };
        assert!(validate_context_for_event(PreflightLifecycleEvent::TurnStart, &ctx).is_ok());
        let mut bad = ctx;
        bad.user_prompt = None;
        assert!(validate_context_for_event(PreflightLifecycleEvent::TurnStart, &bad).is_err());
    }

    #[test]
    fn context_end_requires_outcome() {
        let ctx = LifecycleEventContext {
            work_dir: Some("/w".into()),
            session_id: Some("s".into()),
            turn_id: Some("t".into()),
            outcome: Some(PreflightOutcome::Ok),
            duration_ms: Some(12),
            ..Default::default()
        };
        assert!(validate_context_for_event(PreflightLifecycleEvent::TurnEnd, &ctx).is_ok());
        let mut bad = ctx;
        bad.outcome = None;
        assert!(validate_context_for_event(PreflightLifecycleEvent::TurnEnd, &bad).is_err());
    }

    #[test]
    fn worker_init_rejects_session_effects() {
        let effects = vec![PreflightEffect::LockLanguage {
            language: "Chinese".into(),
            reason: None,
        }];
        assert!(validate_effects_for_event(
            PreflightLifecycleEvent::WorkerInitStart,
            &effects
        )
        .is_err());
        assert!(validate_effects_for_event(PreflightLifecycleEvent::TurnStart, &effects).is_ok());
    }

    #[test]
    fn lifecycle_context_serde_roundtrip() {
        let ctx = build_worker_init_context(9, "w", "/claw_ds", "t", "s", "relaxed");
        let v = serde_json::to_value(&ctx).unwrap();
        assert!(v.get("userPrompt").is_none());
        assert_eq!(v.get("projId").and_then(Value::as_i64), Some(9));
        let back: LifecycleEventContext = serde_json::from_value(v).unwrap();
        assert_eq!(back.worker_id.as_deref(), Some("w"));
    }

    #[test]
    fn default_runtime_pipeline_is_turn_language_only() {
        let steps = default_runtime_pipeline_steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].plugin_id, BUILTIN_TURN_LANGUAGE);
    }

    #[test]
    fn explicit_kind_none_executable_steps_stays_empty() {
        let cfg = parse_pipeline_value(&json!({"kind": "none", "steps": []})).unwrap();
        assert!(executable_pipeline_steps(&cfg).is_empty());
        assert!(normalize_pipeline_steps(&cfg).is_empty());
    }

    #[test]
    fn explicit_kind_none_is_empty_pipeline() {
        let cfg = parse_pipeline_value(&json!({"kind": "none"})).unwrap();
        assert!(normalize_pipeline_steps(&cfg).is_empty());
        assert!(executable_pipeline_steps(&cfg).is_empty());
    }

    #[test]
    fn subprocess_rejects_inject_tool_exchange() {
        let resp = PreflightSpiResponse {
            status: PreflightResponseStatus::Ok,
            message: None,
            effects: vec![PreflightEffect::InjectToolExchange {
                tool_name: "mcp_start".into(),
                input: "{}".into(),
                output: "{}".into(),
                is_error: false,
            }],
            metrics: None,
        };
        assert!(validate_subprocess_response(&resp).is_err());
    }

    #[test]
    fn materialize_emits_steps_with_on() {
        let out = materialize_pipeline_json(&json!({"kind": "sqlbot_mcp_start"}));
        let steps = out.get("steps").and_then(Value::as_array).expect("steps");
        assert_eq!(steps.len(), 2);
        assert!(out.get("kinds").is_none());
        assert_eq!(
            steps[0].get("on").and_then(Value::as_str),
            Some("turn.start")
        );
    }

    #[test]
    fn parse_on_without_scope() {
        let raw = json!({
            "steps": [{
                "pluginId": "apt_tools",
                "on": "worker.init.start",
                "impl": { "type": "subprocess", "command": ["true"] }
            }]
        });
        let steps = normalize_pipeline_steps(&parse_pipeline_value(&raw).unwrap());
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].resolved_event(),
            PreflightLifecycleEvent::WorkerInitStart
        );
    }

    #[test]
    fn effect_serde_roundtrip() {
        let effect = PreflightEffect::LockLanguage {
            language: "Thai".into(),
            reason: Some("test".into()),
        };
        let v = serde_json::to_value(&effect).unwrap();
        assert_eq!(v.get("type").and_then(Value::as_str), Some("lockLanguage"));
        let back: PreflightEffect = serde_json::from_value(v).unwrap();
        assert_eq!(back, effect);
    }
}
