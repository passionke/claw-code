//! Per-project language pipeline config (`project_config.language_pipeline_json` → `home/.claw/language-pipeline.json`).
//! Author: kejiqing

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Relative to `ds_*` root and session worker root.
pub const LANGUAGE_PIPELINE_CONFIG_REL: &str = "home/.claw/language-pipeline.json";

pub const PLACEHOLDER_PRIOR_USER_PROMPTS: &str = "{prior_user_prompts}";
pub const PLACEHOLDER_CURRENT_USER_PROMPT: &str = "{current_user_prompt}";

/// Built-in inference template when project config omits `languageInferencePrompt`. Author: kejiqing
pub const DEFAULT_LANGUAGE_INFERENCE_PROMPT: &str = r#"Determine the language for the assistant's user-visible output this turn.

Rules:
- The CURRENT turn user message is authoritative.
- Prior turns are context only; explicit instructions in the CURRENT turn override (e.g. "answer in English").
- Output JSON only, no markdown: {"language":"<English|Chinese|Thai|Japanese|...>","reason":"<brief>"}

## Prior user messages (oldest first)
{prior_user_prompts}

## Current turn user message (authoritative)
{current_user_prompt}"#;

fn default_prior_turns() -> usize {
    5
}

fn default_prior_max_chars() -> usize {
    3000
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LanguagePipelineConfig {
    #[serde(default = "default_inference_prompt")]
    pub language_inference_prompt: String,
    #[serde(default = "default_prior_turns")]
    pub language_inference_prior_turns: usize,
    #[serde(default = "default_prior_max_chars")]
    pub language_inference_prior_max_chars: usize,
}

fn default_inference_prompt() -> String {
    DEFAULT_LANGUAGE_INFERENCE_PROMPT.to_string()
}

impl Default for LanguagePipelineConfig {
    fn default() -> Self {
        Self {
            language_inference_prompt: default_inference_prompt(),
            language_inference_prior_turns: default_prior_turns(),
            language_inference_prior_max_chars: default_prior_max_chars(),
        }
    }
}

/// Validate `project_config.language_pipeline_json` before DB write.
pub fn validate_language_pipeline_json(value: &Value) -> Result<(), String> {
    let cfg: LanguagePipelineConfig =
        serde_json::from_value(value.clone()).map_err(|e| format!("languagePipelineJson: {e}"))?;
    if cfg.language_inference_prior_turns == 0 {
        return Err(String::from(
            "languagePipelineJson.languageInferencePriorTurns must be >= 1",
        ));
    }
    if cfg.language_inference_prior_max_chars == 0 {
        return Err(String::from(
            "languagePipelineJson.languageInferencePriorMaxChars must be >= 1",
        ));
    }
    if !cfg
        .language_inference_prompt
        .contains(PLACEHOLDER_CURRENT_USER_PROMPT)
    {
        return Err(format!(
            "languagePipelineJson.languageInferencePrompt must contain {PLACEHOLDER_CURRENT_USER_PROMPT}"
        ));
    }
    Ok(())
}

#[must_use]
pub fn materialize_language_pipeline_json(value: &Value) -> Value {
    serde_json::to_value(LanguagePipelineConfig::default())
        .ok()
        .and_then(|default| {
            if value.is_null()
                || (value.is_object() && value.as_object().is_some_and(|m| m.is_empty()))
            {
                return Some(default);
            }
            serde_json::from_value::<LanguagePipelineConfig>(value.clone())
                .ok()
                .and_then(|cfg| serde_json::to_value(cfg).ok())
        })
        .unwrap_or_else(|| value.clone())
}

fn parse_language_pipeline_file(path: &Path) -> Option<LanguagePipelineConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    serde_json::from_value(materialize_language_pipeline_json(&value)).ok()
}

/// Resolve language pipeline config for a worker session (pool ro mount or ds tree).
#[must_use]
pub fn resolve_language_pipeline_config(session_home: &Path) -> LanguagePipelineConfig {
    if let Some(cfg) =
        parse_language_pipeline_file(&session_home.join(LANGUAGE_PIPELINE_CONFIG_REL))
    {
        return cfg;
    }
    let config_root = runtime::gateway_project_config_root(session_home);
    parse_language_pipeline_file(&config_root.join(LANGUAGE_PIPELINE_CONFIG_REL))
        .unwrap_or_default()
}

/// Substitute `{prior_user_prompts}` and `{current_user_prompt}` into the inference template.
#[must_use]
pub fn render_language_inference_prompt(
    template: &str,
    prior_user_prompts: &str,
    current_user_prompt: &str,
) -> String {
    template
        .replace(PLACEHOLDER_PRIOR_USER_PROMPTS, prior_user_prompts)
        .replace(PLACEHOLDER_CURRENT_USER_PROMPT, current_user_prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_current_placeholder() {
        let v = serde_json::json!({"languageInferencePrompt": "no placeholder"});
        assert!(validate_language_pipeline_json(&v).is_err());
    }

    #[test]
    fn validate_accepts_default_shape() {
        let v = serde_json::json!({});
        validate_language_pipeline_json(&v).unwrap();
    }

    #[test]
    fn render_substitutes_placeholders() {
        let out = render_language_inference_prompt(
            "prior={prior_user_prompts}\ncurrent={current_user_prompt}",
            "old",
            "new",
        );
        assert!(out.contains("prior=old"));
        assert!(out.contains("current=new"));
    }
}
