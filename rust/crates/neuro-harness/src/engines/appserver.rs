//! `neuro-appserver`: codex-acp (bundled codex app-server) with `CODEX_HOME` under the session root.
//! Contract: `docs/neuro-harness-contract.md` §8. Author: kejiqing

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use agent_client_protocol::schema::v1::SessionUpdate;
use serde_json::{json, Value};

use crate::profile::{AgentLaunch, EngineProfile, PrepareContext, TurnSignal};
use crate::projection::{link_skills, write_file};
use crate::HarnessError;

pub const DEFAULT_BIN: &str = "/usr/local/lib/neuro-engines/codex-acp/node_modules/.bin/codex-acp";
pub const BIN_ENV: &str = "NEURO_CODEX_ACP_BIN";
const PROVIDER: &str = "neurogate";

/// Base instructions of the pinned codex (`codex-rs/models-manager/prompt.md`, rust-v0.159.3).
const CODEX_BASE_INSTRUCTIONS: &str = include_str!("../../assets/codex/rust-v0.159.3/prompt.md");
/// Codex fallback metadata default (`model_info_from_slug`, rust-v0.159.3).
const CODEX_DEFAULT_CONTEXT_WINDOW: u64 = 272_000;

pub struct AppserverProfile;

impl EngineProfile for AppserverProfile {
    fn engine(&self) -> &'static str {
        "appserver"
    }

    fn prepare(&self, ctx: &PrepareContext<'_>) -> Result<AgentLaunch, HarnessError> {
        let root = ctx.session_root;
        let codex_home = root.join(".codex");
        let catalog_path = codex_home.join("model-catalog.json");
        let catalog = serde_json::to_vec_pretty(&model_catalog(ctx.model, context_window()))
            .map_err(|e| HarnessError::internal(format!("serialize model catalog: {e}")))?;
        write_file(&catalog_path, &catalog)?;
        write_file(
            &codex_home.join("config.toml"),
            config_toml(ctx.model, ctx.openai_base_url, &catalog_path).as_bytes(),
        )?;
        let instructions = std::fs::read(ctx.instructions_path).map_err(|e| {
            HarnessError::internal(format!("read {}: {e}", ctx.instructions_path.display()))
        })?;
        write_file(&codex_home.join("AGENTS.md"), &instructions)?;
        link_skills(ctx.skills_dir, &root.join(".agents/skills"), |_| true)?;

        let mut env = BTreeMap::new();
        env.insert("HOME".into(), root.to_string_lossy().to_string());
        env.insert(
            "CODEX_HOME".into(),
            codex_home.to_string_lossy().to_string(),
        );
        // Isolation is landlock's job; codex's own sandbox and approvals stay off.
        env.insert("INITIAL_AGENT_MODE".into(), "agent-full-access".into());
        Ok(AgentLaunch {
            command: super::engine_bin(BIN_ENV, DEFAULT_BIN),
            args: Vec::new(),
            env,
        })
    }

    /// codex-acp reports a dead turn as `_meta.codex.threadStatus.type = "systemError"`, then
    /// streams the error text as a normal message and answers the prompt with `end_turn`.
    fn turn_signal(&self, update: &SessionUpdate) -> Option<TurnSignal> {
        let SessionUpdate::SessionInfoUpdate(info) = update else {
            return None;
        };
        let codex = info.meta.as_ref()?.get("codex")?;
        if codex.pointer("/threadStatus/type").and_then(Value::as_str) == Some("systemError") {
            return Some(TurnSignal::Failed);
        }
        let error = codex.get("error")?;
        ["additionalDetails", "message"]
            .iter()
            .find_map(|k| error.get(*k).and_then(Value::as_str))
            .filter(|s| !s.trim().is_empty())
            .map(|s| TurnSignal::ErrorDetail(s.to_string()))
    }
}

/// `CLAW_CONTEXT_WINDOW_TOKENS` (gateway-injected) when set, else codex's own default.
fn context_window() -> u64 {
    std::env::var("CLAW_CONTEXT_WINDOW_TOKENS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(CODEX_DEFAULT_CONTEXT_WINDOW)
}

fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn config_toml(model: &str, base_url: &str, catalog_path: &Path) -> String {
    format!(
        "model = {model}\n\
         model_provider = \"{PROVIDER}\"\n\
         model_catalog_json = {catalog}\n\
         approval_policy = \"never\"\n\
         sandbox_mode = \"danger-full-access\"\n\
         check_for_update_on_startup = false\n\
         \n\
         [analytics]\n\
         enabled = false\n\
         \n\
         [model_providers.{PROVIDER}]\n\
         name = \"{PROVIDER}\"\n\
         base_url = {base_url}\n\
         env_key = \"OPENAI_API_KEY\"\n\
         wire_api = \"responses\"\n",
        model = toml_str(model),
        catalog = toml_str(&catalog_path.to_string_lossy()),
        base_url = toml_str(base_url),
    )
}

/// One-entry catalog equal to codex's fallback metadata, so codex does not prepend the
/// "Model metadata not found" warning to the reply. `visibility` is `list` (fallback: `none`)
/// because codex-acp `session/new` fails when `model/list` comes back empty.
fn model_catalog(model: &str, context_window: u64) -> Value {
    let mut entry = json!({
        "slug": model,
        "display_name": model,
        "description": null,
        "default_reasoning_level": null,
        "supported_reasoning_levels": [],
        "shell_type": "unified_exec",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 99,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "default_service_tier": null,
        "availability_nux": null,
        "upgrade": null,
        "include_skills_usage_instructions": false,
        "include_plugin_usage_instructions": false,
        "include_apps_usage_instructions": false,
        "supports_reasoning_summary_parameter": true,
        "default_reasoning_summary": "auto",
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "web_search_tool_type": "text",
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_image_detail_original": false,
    });
    let rest = json!({
        "context_window": context_window,
        "max_context_window": context_window,
        "auto_compact_token_limit": null,
        "comp_hash": null,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": false,
        "supports_experimental_context": false,
        "use_responses_lite": false,
        "supports_reasoning_effort_updates": false,
        "guardian": null,
        "node_repl_auto_review_required": false,
        "node_repl_disabled": false,
        "auto_review_model_override": null,
        "model_specialty": null,
        "tool_mode": null,
        "multi_agent_version": null,
        "multi_agent_reasoning_effort": null,
        "model_messages": {"instructions_template": CODEX_BASE_INSTRUCTIONS},
    });
    if let (Some(entry), Value::Object(rest)) = (entry.as_object_mut(), rest) {
        entry.extend(rest);
    }
    json!({"models": [entry]})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(meta: &Value) -> SessionUpdate {
        serde_json::from_value(json!({"sessionUpdate": "session_info_update", "_meta": meta}))
            .unwrap()
    }

    #[test]
    fn codex_system_error_is_a_failed_turn() {
        let p = AppserverProfile;
        let retry = info(&json!({"codex": {"error": {
            "message": "Reconnecting... 5/5",
            "codexErrorInfo": {"responseStreamDisconnected": {"httpStatusCode": 404}},
            "additionalDetails": "unexpected status 404 Not Found: Unknown error, url: http://tap/responses"
        }}}));
        assert_eq!(
            p.turn_signal(&retry),
            Some(TurnSignal::ErrorDetail(
                "unexpected status 404 Not Found: Unknown error, url: http://tap/responses".into()
            ))
        );
        let dead = info(&json!({"codex": {"threadStatus": {"type": "systemError"}}}));
        assert_eq!(p.turn_signal(&dead), Some(TurnSignal::Failed));
        let active =
            info(&json!({"codex": {"threadStatus": {"type": "active", "activeFlags": []}}}));
        assert_eq!(p.turn_signal(&active), None);
    }

    #[test]
    fn toml_strings_are_escaped() {
        assert_eq!(toml_str(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn prepare_writes_codex_home() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sess");
        let pcr = dir.path().join("pcr");
        std::fs::create_dir_all(pcr.join(".claw/skills/My_Skill")).unwrap();
        std::fs::write(pcr.join(".claw/skills/My_Skill/SKILL.md"), "x").unwrap();
        let instructions = root.join(".neuro-harness/instructions.md");
        write_file(&instructions, b"be terse").unwrap();
        let launch = AppserverProfile
            .prepare(&PrepareContext {
                session_root: &root,
                project_config_root: &pcr,
                model: "gpt-x",
                openai_base_url: "http://tap:8080",
                instructions_path: &instructions,
                skills_dir: &pcr.join(".claw/skills"),
            })
            .unwrap();
        assert_eq!(launch.env["INITIAL_AGENT_MODE"], "agent-full-access");
        let toml = std::fs::read_to_string(root.join(".codex/config.toml")).unwrap();
        assert!(toml.contains("model = \"gpt-x\""), "{toml}");
        assert!(toml.contains("base_url = \"http://tap:8080\""), "{toml}");
        assert!(toml.contains("wire_api = \"responses\""), "{toml}");
        assert_eq!(
            std::fs::read_to_string(root.join(".codex/AGENTS.md")).unwrap(),
            "be terse"
        );
        let catalog: Value =
            serde_json::from_slice(&std::fs::read(root.join(".codex/model-catalog.json")).unwrap())
                .unwrap();
        assert_eq!(catalog["models"][0]["slug"], "gpt-x");
        assert!(
            catalog["models"][0]["model_messages"]["instructions_template"]
                .as_str()
                .unwrap()
                .starts_with("You are a coding agent running in the Codex CLI")
        );
        assert!(root.join(".agents/skills/My_Skill/SKILL.md").is_file());
    }
}
