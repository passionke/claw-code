//! Project harness engine (`claw` | `opencode` | `appserver`), fixed at project creation.
//! The only gateway module that maps an engine to its e2b template, worker bin and capability
//! rules; hot paths call in through one-line hooks. Contract: `docs/neuro-harness-contract.md`.
//! Author: kejiqing

use gateway_solve_turn::GatewaySolveTaskFile;
use serde::{Deserialize, Serialize};

use crate::gateway_e2b_worker_settings::{e2b_worker_build_id, E2bWorkerSettings};
use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

use super::worker_profile::{mode_from_json, WorkerProfileMode};

/// Error prefix shared with the worker-side gate (`neuro-harness` `task.rs`).
pub const UNSUPPORTED_BY_ENGINE: &str = "unsupported_by_engine";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum HarnessEngine {
    #[default]
    Claw,
    Opencode,
    Appserver,
}

impl HarnessEngine {
    /// Missing / blank → `claw`.
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).unwrap_or_default() {
            "" | "claw" => Ok(Self::Claw),
            "opencode" => Ok(Self::Opencode),
            "appserver" => Ok(Self::Appserver),
            other => Err(format!(
                "harnessEngine must be claw | opencode | appserver; got {other:?}"
            )),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claw => "claw",
            Self::Opencode => "opencode",
            Self::Appserver => "appserver",
        }
    }

    #[must_use]
    pub fn is_claw(self) -> bool {
        self == Self::Claw
    }

    /// Worker CLI inside the engine image; `None` keeps the claw bin.
    fn worker_bin(self) -> Option<&'static str> {
        match self {
            Self::Claw => None,
            Self::Opencode => Some("/usr/local/bin/neuro-opencode"),
            Self::Appserver => Some("/usr/local/bin/neuro-appserver"),
        }
    }

    /// e2b template alias (also the default when PG has no `templateId`).
    fn template_alias(self) -> Option<&'static str> {
        match self {
            Self::Claw => None,
            Self::Opencode => Some("claw-worker-opencode"),
            Self::Appserver => Some("claw-worker-appserver"),
        }
    }
}

/// Bin for `exec_solve`: the engine CLI, or the existing claw bin.
#[must_use]
pub fn exec_bin(engine: HarnessEngine, claw_bin: &str) -> &str {
    engine.worker_bin().unwrap_or(claw_bin)
}

pub async fn load_project_harness_engine(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<HarnessEngine, String> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT harness_engine FROM project_config WHERE cluster_id = $1 AND proj_id = $2",
    )
    .bind(db.cluster_id())
    .bind(proj_id)
    .fetch_optional(db.pg_pool())
    .await
    .map_err(|e| format!("load harness_engine for proj {proj_id}: {e}"))?;
    HarnessEngine::parse(raw.as_deref())
}

/// Written once, right after the project row is inserted. No other code path updates the column.
pub async fn set_harness_engine_on_create(
    db: &GatewaySessionDb,
    proj_id: i64,
    engine: HarnessEngine,
) -> Result<(), String> {
    if engine.is_claw() {
        return Ok(());
    }
    let r = sqlx::query(
        "UPDATE project_config SET harness_engine = $3 WHERE cluster_id = $1 AND proj_id = $2",
    )
    .bind(db.cluster_id())
    .bind(proj_id)
    .bind(engine.as_str())
    .execute(db.pg_pool())
    .await
    .map_err(|e| format!("set harness_engine for proj {proj_id}: {e}"))?;
    if r.rows_affected() == 0 {
        return Err(format!(
            "project {proj_id} not found when setting harness_engine"
        ));
    }
    Ok(())
}

/// e2b template for an engine worker (strict only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineWorkerTemplate {
    pub template_id: String,
    pub build_id: Option<String>,
    pub alias: String,
}

fn engine_settings(
    store: &crate::gateway_global_settings::GatewayGlobalSettingsStore,
    engine: HarnessEngine,
) -> Option<&E2bWorkerSettings> {
    match engine {
        HarnessEngine::Claw => None,
        HarnessEngine::Opencode => Some(&store.e2b_worker_opencode),
        HarnessEngine::Appserver => Some(&store.e2b_worker_appserver),
    }
}

fn template_from_settings(
    engine: HarnessEngine,
    settings: &E2bWorkerSettings,
) -> Option<EngineWorkerTemplate> {
    let alias = engine.template_alias()?.to_string();
    Some(EngineWorkerTemplate {
        template_id: settings
            .template_id
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| alias.clone()),
        build_id: e2b_worker_build_id(settings),
        alias,
    })
}

/// PG `e2bWorkerOpencode` / `e2bWorkerAppserver`; `None` for claw (existing template logic).
pub async fn engine_worker_template(
    db: &GatewaySessionDb,
    engine: HarnessEngine,
) -> Result<Option<EngineWorkerTemplate>, String> {
    if engine.is_claw() {
        return Ok(None);
    }
    let (store, _, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| format!("load global settings: {e}"))?;
    Ok(engine_settings(&store, engine).and_then(|s| template_from_settings(engine, s)))
}

/// `#profile=` segment of the worker contract key for engine workers (claw keys unchanged).
#[must_use]
pub fn contract_profile_label(engine: HarnessEngine) -> String {
    format!("strict+{}", engine.as_str())
}

fn unsupported(what: &str, engine: HarnessEngine) -> String {
    format!(
        "{UNSUPPORTED_BY_ENGINE}: {what} (harnessEngine={})",
        engine.as_str()
    )
}

/// appserver (Codex) speaks only the Responses API: the effective upstream must be `/responses`.
pub fn validate_llm_upstream(engine: HarnessEngine, base_model_url: &str) -> Result<(), String> {
    if engine != HarnessEngine::Appserver {
        return Ok(());
    }
    if crate::gateway_tap_client::base_model_url_is_responses(base_model_url) {
        Ok(())
    } else {
        Err(unsupported(
            &format!("LLM baseModelUrl must end with /responses, got {base_model_url:?}"),
            engine,
        ))
    }
}

/// [`validate_llm_upstream`] against the project's effective LLM (project override, else cluster).
pub async fn check_project_llm_upstream(
    db: &GatewaySessionDb,
    engine: HarnessEngine,
    proj_id: i64,
) -> Result<(), String> {
    if engine != HarnessEngine::Appserver {
        return Ok(());
    }
    let runtime = crate::gateway_project_llm::load_effective_llm_runtime(db, proj_id)
        .await
        .map_err(|e| format!("load effective LLM for proj {proj_id}: {e}"))?
        .ok_or_else(|| format!("no active LLM for proj {proj_id}"))?;
    validate_llm_upstream(engine, &runtime.base_model_url)
}

/// Non-claw projects stay `normal`.
pub fn validate_role(engine: HarnessEngine, role: &str) -> Result<(), String> {
    if engine.is_claw() || role == crate::master_observer::PROJECT_ROLE_NORMAL {
        Ok(())
    } else {
        Err(unsupported(&format!("projectRole={role}"), engine))
    }
}

/// Non-claw projects run strict workers only.
pub fn validate_worker_profile(
    engine: HarnessEngine,
    worker_profile_json: &serde_json::Value,
) -> Result<(), String> {
    if engine.is_claw() || mode_from_json(worker_profile_json) == WorkerProfileMode::Strict {
        Ok(())
    } else {
        Err(unsupported("workerProfileJson.mode=relaxed", engine))
    }
}

/// Request-level gate before any work: plan mode and sealed plans are claw-only; engine workers
/// exist only as e2b templates.
pub fn gate_solve_request(
    engine: HarnessEngine,
    e2b_backend: bool,
    interaction_mode: Option<&str>,
    sealed_plan_id: Option<&str>,
    sealed_plan_markdown: Option<&str>,
) -> Result<(), String> {
    if engine.is_claw() {
        return Ok(());
    }
    if !e2b_backend {
        return Err(unsupported("non-e2b worker backend", engine));
    }
    if gateway_solve_turn::InteractionMode::parse(interaction_mode).is_plan() {
        return Err(unsupported("interactionMode=plan", engine));
    }
    if sealed_plan_id.is_some_and(|s| !s.trim().is_empty())
        || sealed_plan_markdown.is_some_and(|s| !s.trim().is_empty())
    {
        return Err(unsupported("sealedPlan", engine));
    }
    Ok(())
}

/// Engine tasks always run as a plain agent turn.
pub fn trim_task(engine: HarnessEngine, task: &mut GatewaySolveTaskFile) {
    if engine.is_claw() {
        return;
    }
    task.interaction_mode = Some("agent".to_string());
    task.ask_user_question_enabled = Some(false);
    task.force_single_turn = Some(true);
    task.sealed_plan_id = None;
    task.sealed_plan_markdown = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_and_bins() {
        assert_eq!(HarnessEngine::parse(None).unwrap(), HarnessEngine::Claw);
        assert_eq!(
            HarnessEngine::parse(Some(" ")).unwrap(),
            HarnessEngine::Claw
        );
        assert_eq!(
            HarnessEngine::parse(Some("opencode")).unwrap(),
            HarnessEngine::Opencode
        );
        assert!(HarnessEngine::parse(Some("codex")).is_err());
        assert_eq!(
            exec_bin(HarnessEngine::Claw, "/usr/local/bin/claw"),
            "/usr/local/bin/claw"
        );
        assert_eq!(
            exec_bin(HarnessEngine::Appserver, "/usr/local/bin/claw"),
            "/usr/local/bin/neuro-appserver"
        );
        assert_eq!(
            exec_bin(HarnessEngine::Opencode, "claw"),
            "/usr/local/bin/neuro-opencode"
        );
    }

    #[test]
    fn template_defaults_to_alias_and_pins_build() {
        let t =
            template_from_settings(HarnessEngine::Opencode, &E2bWorkerSettings::default()).unwrap();
        assert_eq!(t.template_id, "claw-worker-opencode");
        assert_eq!(t.alias, "claw-worker-opencode");
        assert_eq!(t.build_id, None);
        let s: E2bWorkerSettings =
            serde_json::from_value(json!({"templateId":"tpl_x","buildId":"b1"})).unwrap();
        let t = template_from_settings(HarnessEngine::Appserver, &s).unwrap();
        assert_eq!(t.template_id, "tpl_x");
        assert_eq!(t.build_id.as_deref(), Some("b1"));
        assert_eq!(t.alias, "claw-worker-appserver");
        assert!(template_from_settings(HarnessEngine::Claw, &s).is_none());
        assert_eq!(
            contract_profile_label(HarnessEngine::Opencode),
            "strict+opencode"
        );
    }

    #[test]
    fn appserver_requires_responses_upstream() {
        let e = HarnessEngine::Appserver;
        assert!(validate_llm_upstream(e, "https://api.openai.com/v1/responses").is_ok());
        assert!(validate_llm_upstream(e, "https://api.openai.com/v1/responses/").is_ok());
        assert!(validate_llm_upstream(e, "https://h/v1/RESPONSES?x=1").is_ok());
        let err = validate_llm_upstream(e, "https://api.x.ai/v1/chat/completions").unwrap_err();
        assert!(err.starts_with("unsupported_by_engine:"), "{err}");
        assert!(validate_llm_upstream(e, "https://api.x.ai/v1").is_err());
        assert!(validate_llm_upstream(HarnessEngine::Opencode, "https://x/v1").is_ok());
        assert!(validate_llm_upstream(HarnessEngine::Claw, "https://x/v1").is_ok());
    }

    #[test]
    fn role_and_profile_rules() {
        assert!(validate_role(HarnessEngine::Opencode, "normal").is_ok());
        assert!(validate_role(HarnessEngine::Opencode, "steerable").is_err());
        assert!(validate_role(HarnessEngine::Claw, "master").is_ok());
        assert!(
            validate_worker_profile(HarnessEngine::Appserver, &json!({"mode":"strict"})).is_ok()
        );
        assert!(
            validate_worker_profile(HarnessEngine::Appserver, &json!({"mode":"relaxed"})).is_err()
        );
        assert!(validate_worker_profile(HarnessEngine::Claw, &json!({"mode":"relaxed"})).is_ok());
    }

    /// Engine workers append the same `runtime::Session` JSONL as claw (user msg, then the
    /// mapper's assistant/tool messages); the gateway must split it into one group per turn.
    #[test]
    fn engine_transcript_splits_per_turn() {
        use runtime::{ContentBlock, ConversationMessage, Session};
        let dir = tempfile::tempdir().unwrap();
        let path = gateway_solve_turn::gateway_solve_session_persistence_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut session = Session::new().with_persistence_path(&path);
        let text = |t: &str| ContentBlock::Text { text: t.into() };
        for m in [
            ConversationMessage::user_text("turn one"),
            ConversationMessage::assistant(vec![text("hello")]),
        ] {
            session.push_message(m).unwrap();
        }
        let mut session = Session::load_from_path(&path)
            .unwrap()
            .with_persistence_path(&path);
        for m in [
            ConversationMessage::user_text("turn two"),
            ConversationMessage::assistant(vec![ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "mcp__demo__echo".into(),
                input: "{\"x\":1}".into(),
            }]),
            ConversationMessage::tool_result("call_1", "mcp__demo__echo", "ok", false),
            ConversationMessage::assistant(vec![text("done")]),
        ] {
            session.push_message(m).unwrap();
        }
        let contents = std::fs::read_to_string(&path).unwrap();
        let groups =
            crate::persistence::transcript::turn_message_groups_from_jsonl_contents(&contents);
        let roles: Vec<Vec<&str>> = groups
            .iter()
            .map(|g| g.iter().map(|m| m.role.as_str()).collect())
            .collect();
        assert_eq!(
            roles,
            vec![
                vec!["user", "assistant"],
                vec!["user", "assistant", "tool", "assistant"]
            ]
        );
    }

    #[test]
    fn solve_gate_and_trim() {
        let e = HarnessEngine::Opencode;
        assert!(gate_solve_request(e, true, Some("agent"), None, None).is_ok());
        assert!(gate_solve_request(e, true, Some("plan"), None, None)
            .unwrap_err()
            .contains("interactionMode=plan"));
        assert!(gate_solve_request(e, true, None, Some("p1"), None).is_err());
        assert!(gate_solve_request(e, false, None, None, None).is_err());
        assert!(gate_solve_request(HarnessEngine::Claw, false, Some("plan"), None, None).is_ok());

        let mut task: GatewaySolveTaskFile = serde_json::from_value(json!({
            "requestId":"r","userPrompt":"p","turnId":"t",
            "askUserQuestionEnabled":true,"forceSingleTurn":false
        }))
        .unwrap();
        trim_task(e, &mut task);
        assert_eq!(task.interaction_mode.as_deref(), Some("agent"));
        assert_eq!(task.ask_user_question_enabled, Some(false));
        assert_eq!(task.force_single_turn, Some(true));
    }
}
