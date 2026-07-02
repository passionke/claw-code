//! Preflight SPI v1 types and validation (shared by gateway-solve-turn and http-gateway-rs). Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Deserialize)]
struct LegacyPreflightKind {
    kind: String,
}

pub const SPI_VERSION: &str = "1";

/// Known builtin handler ids (registry may extend via Admin).
pub const BUILTIN_TURN_LANGUAGE: &str = "turn_language";
pub const BUILTIN_SQLBOT_MCP_START: &str = "sqlbot_mcp_start";

/// When a session-first-turn step is considered done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightScope {
    EveryTurn,
    SessionFirstTurn,
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
    pub scope: PreflightScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#impl: Option<PreflightImpl>,
    #[serde(default)]
    pub config: Value,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightSpiRequest {
    #[serde(rename = "spiVersion")]
    pub spi_version: String,
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

/// Returns step indices to run in order.
#[must_use]
pub fn filter_step_indices(steps: &[PreflightStep], ctx: PreflightFilterContext) -> Vec<usize> {
    steps
        .iter()
        .enumerate()
        .filter(|(_, step)| should_run_step(step.scope, ctx))
        .map(|(i, _)| i)
        .collect()
}

#[must_use]
pub fn should_run_step(scope: PreflightScope, ctx: PreflightFilterContext) -> bool {
    match scope {
        PreflightScope::EveryTurn => true,
        PreflightScope::SessionFirstTurn => {
            !ctx.is_continuation && !ctx.session_first_turn_satisfied
        }
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
    if request.context.user_prompt.is_empty() && request.context.is_continuation {
        // continuation may have empty user_prompt in edge cases; allow
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
        scope: PreflightScope::EveryTurn,
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
            scope: PreflightScope::SessionFirstTurn,
            r#impl: Some(PreflightImpl::Builtin {
                handler: BUILTIN_SQLBOT_MCP_START.to_string(),
            }),
            config: Value::Object(Map::default()),
        }),
        BUILTIN_TURN_LANGUAGE => Some(PreflightStep {
            plugin_id: BUILTIN_TURN_LANGUAGE.to_string(),
            scope: PreflightScope::EveryTurn,
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

/// Normalize to executable `steps` (migrate legacy `kinds`; explicit empty kinds → no steps).
#[must_use]
pub fn normalize_pipeline_steps(cfg: &PreflightPipelineConfig) -> Vec<PreflightStep> {
    if !cfg.steps.is_empty() {
        return cfg.steps.clone();
    }
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
        // Explicit tombstone (`kind:none`, `steps:[]`) — no runtime steps.
        return vec![];
    }
    if steps.is_empty() {
        return default_runtime_pipeline_steps();
    }
    steps
}

/// Materialize worker-readable `solve-preflight.json` (steps only).
#[must_use]
pub fn materialize_pipeline_json(value: &Value) -> Value {
    let Ok(cfg) = parse_pipeline_value(value) else {
        return serde_json::json!({ "steps": [] });
    };
    let steps = normalize_pipeline_steps(&cfg);
    if steps.is_empty() {
        return serde_json::json!({ "steps": [] });
    }
    serde_json::to_value(&steps).map_or_else(
        |_| serde_json::json!({ "steps": [] }),
        |steps| serde_json::json!({ "steps": steps }),
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
    use serde_json::json;

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
    fn migrate_kinds_to_steps_with_default_language() {
        let raw = json!({"kinds": ["sqlbot_mcp_start"]});
        let steps = normalize_pipeline_steps(&parse_pipeline_value(&raw).unwrap());
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].plugin_id, BUILTIN_TURN_LANGUAGE);
        assert_eq!(steps[0].scope, PreflightScope::EveryTurn);
        assert_eq!(steps[1].plugin_id, BUILTIN_SQLBOT_MCP_START);
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
    fn materialize_emits_steps_only() {
        let out = materialize_pipeline_json(&json!({"kind": "sqlbot_mcp_start"}));
        let steps = out.get("steps").and_then(Value::as_array).expect("steps");
        assert_eq!(steps.len(), 2);
        assert!(out.get("kinds").is_none());
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
