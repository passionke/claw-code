//! Project harness engine (`claw` | `opencode` | `appserver`), fixed at project creation.
//! The only gateway module that knows engines. Every per-engine rule lives behind
//! [`EngineStrategy`]: claw is the pass-through strategy (trait defaults), neuro engines share
//! [`NeuroEngine`] and differ only by their static spec. Hot paths call
//! `engine.strategy().<hook>()`. Contract: `docs/neuro-harness-contract.md`.
//! Author: kejiqing

use gateway_solve_turn::GatewaySolveTaskFile;
use serde::{Deserialize, Serialize};

use crate::gateway_e2b_worker_settings::{e2b_worker_build_id, E2bWorkerSettings};
use crate::gateway_global_settings::{get_gateway_global_settings, GatewayGlobalSettingsStore};
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

    /// Column default: claw projects never write `harness_engine`.
    #[must_use]
    pub fn is_claw(self) -> bool {
        self == Self::Claw
    }

    #[must_use]
    pub fn strategy(self) -> &'static dyn EngineStrategy {
        match self {
            Self::Claw => &ClawEngine,
            Self::Opencode => &OPENCODE,
            Self::Appserver => &APPSERVER,
        }
    }
}

/// e2b template for an engine worker (strict only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineWorkerTemplate {
    pub template_id: String,
    pub build_id: Option<String>,
    pub alias: String,
    /// `#profile=` segment of the worker contract key (claw keys unchanged).
    pub profile_label: String,
}

/// Per-engine policy. Defaults are claw's behavior, so the claw strategy is empty.
pub trait EngineStrategy: Sync {
    /// Bin for `exec_solve`.
    fn exec_bin<'a>(&self, claw_bin: &'a str) -> &'a str {
        claw_bin
    }

    /// Own e2b template from global settings; `None` keeps the claw template logic.
    fn worker_template(&self, _store: &GatewayGlobalSettingsStore) -> Option<EngineWorkerTemplate> {
        None
    }

    fn validate_role(&self, _role: &str) -> Result<(), String> {
        Ok(())
    }

    fn validate_worker_profile(
        &self,
        _worker_profile_json: &serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Request-level gate before any work.
    fn gate_solve_request(&self, _req: &SolveGate<'_>) -> Result<(), String> {
        Ok(())
    }

    /// Last edit of the task file before it is shipped to the worker.
    fn prepare_task(&self, _task: &mut GatewaySolveTaskFile) {}
}

/// Inputs of [`EngineStrategy::gate_solve_request`].
pub struct SolveGate<'a> {
    pub e2b_backend: bool,
    pub interaction_mode: Option<&'a str>,
    pub sealed_plan_id: Option<&'a str>,
    pub sealed_plan_markdown: Option<&'a str>,
}

pub struct ClawEngine;

impl EngineStrategy for ClawEngine {}

/// A neuro-harness engine: an ACP agent behind `neuro-<name>` in its own strict e2b template.
pub struct NeuroEngine {
    name: &'static str,
    worker_bin: &'static str,
    /// e2b template alias, also the template when PG has no `templateId`.
    template_alias: &'static str,
    template_settings: fn(&GatewayGlobalSettingsStore) -> &E2bWorkerSettings,
    /// Read-only paths the engine runtime needs inside the strict Landlock jail, appended to the
    /// resolved DSL (e.g. Bun aborts without `/proc` and `/dev/urandom`).
    landlock_runtime_ro: &'static [&'static str],
}

fn opencode_settings(s: &GatewayGlobalSettingsStore) -> &E2bWorkerSettings {
    &s.e2b_worker_opencode
}

fn appserver_settings(s: &GatewayGlobalSettingsStore) -> &E2bWorkerSettings {
    &s.e2b_worker_appserver
}

static OPENCODE: NeuroEngine = NeuroEngine {
    name: "opencode",
    worker_bin: "/usr/local/bin/neuro-opencode",
    template_alias: "claw-worker-opencode",
    template_settings: opencode_settings,
    landlock_runtime_ro: &["/proc", "/dev/urandom"],
};

static APPSERVER: NeuroEngine = NeuroEngine {
    name: "appserver",
    worker_bin: "/usr/local/bin/neuro-appserver",
    template_alias: "claw-worker-appserver",
    template_settings: appserver_settings,
    landlock_runtime_ro: &[],
};

impl NeuroEngine {
    fn unsupported(&self, what: &str) -> String {
        format!(
            "{UNSUPPORTED_BY_ENGINE}: {what} (harnessEngine={})",
            self.name
        )
    }
}

impl EngineStrategy for NeuroEngine {
    fn exec_bin<'a>(&self, _claw_bin: &'a str) -> &'a str {
        self.worker_bin
    }

    fn worker_template(&self, store: &GatewayGlobalSettingsStore) -> Option<EngineWorkerTemplate> {
        let settings = (self.template_settings)(store);
        Some(EngineWorkerTemplate {
            template_id: settings
                .template_id
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| self.template_alias.to_string()),
            build_id: e2b_worker_build_id(settings),
            alias: self.template_alias.to_string(),
            profile_label: format!("strict+{}", self.name),
        })
    }

    /// Engine projects stay `normal`.
    fn validate_role(&self, role: &str) -> Result<(), String> {
        if role == crate::master_observer::PROJECT_ROLE_NORMAL {
            Ok(())
        } else {
            Err(self.unsupported(&format!("projectRole={role}")))
        }
    }

    /// Engine workers run strict only.
    fn validate_worker_profile(
        &self,
        worker_profile_json: &serde_json::Value,
    ) -> Result<(), String> {
        if mode_from_json(worker_profile_json) == WorkerProfileMode::Strict {
            Ok(())
        } else {
            Err(self.unsupported("workerProfileJson.mode=relaxed"))
        }
    }

    /// Plan mode and sealed plans are claw-only; engine workers exist only as e2b templates.
    fn gate_solve_request(&self, req: &SolveGate<'_>) -> Result<(), String> {
        if !req.e2b_backend {
            return Err(self.unsupported("non-e2b worker backend"));
        }
        if gateway_solve_turn::InteractionMode::parse(req.interaction_mode).is_plan() {
            return Err(self.unsupported("interactionMode=plan"));
        }
        let set = |s: Option<&str>| s.is_some_and(|s| !s.trim().is_empty());
        if set(req.sealed_plan_id) || set(req.sealed_plan_markdown) {
            return Err(self.unsupported("sealedPlan"));
        }
        Ok(())
    }

    /// Plain agent turn, plus the runtime's read-only paths in the jail (visible in the task file).
    fn prepare_task(&self, task: &mut GatewaySolveTaskFile) {
        task.interaction_mode = Some("agent".to_string());
        task.ask_user_question_enabled = Some(false);
        task.force_single_turn = Some(true);
        task.sealed_plan_id = None;
        task.sealed_plan_markdown = None;
        if let Some(dsl) = task.landlock_dsl.as_mut() {
            for path in self.landlock_runtime_ro {
                if !dsl.ro.iter().any(|p| p == path) {
                    dsl.ro.push((*path).to_string());
                }
            }
        }
    }
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
    Ok(engine.strategy().worker_template(&store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gate(
        e2b: bool,
        mode: Option<&'static str>,
        sealed: Option<&'static str>,
    ) -> SolveGate<'static> {
        SolveGate {
            e2b_backend: e2b,
            interaction_mode: mode,
            sealed_plan_id: sealed,
            sealed_plan_markdown: None,
        }
    }

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
        let bin = |e: HarnessEngine| e.strategy().exec_bin("/usr/local/bin/claw").to_string();
        assert_eq!(bin(HarnessEngine::Claw), "/usr/local/bin/claw");
        assert_eq!(
            bin(HarnessEngine::Appserver),
            "/usr/local/bin/neuro-appserver"
        );
        assert_eq!(
            bin(HarnessEngine::Opencode),
            "/usr/local/bin/neuro-opencode"
        );
    }

    #[test]
    fn template_defaults_to_alias_and_pins_build() {
        let mut store = GatewayGlobalSettingsStore::default();
        let t = HarnessEngine::Opencode
            .strategy()
            .worker_template(&store)
            .unwrap();
        assert_eq!(t.template_id, "claw-worker-opencode");
        assert_eq!(t.alias, "claw-worker-opencode");
        assert_eq!(t.build_id, None);
        assert_eq!(t.profile_label, "strict+opencode");
        store.e2b_worker_appserver =
            serde_json::from_value(json!({"templateId":"tpl_x","buildId":"b1"})).unwrap();
        let t = HarnessEngine::Appserver
            .strategy()
            .worker_template(&store)
            .unwrap();
        assert_eq!(t.template_id, "tpl_x");
        assert_eq!(t.build_id.as_deref(), Some("b1"));
        assert_eq!(t.alias, "claw-worker-appserver");
        assert!(HarnessEngine::Claw
            .strategy()
            .worker_template(&store)
            .is_none());
    }

    #[test]
    fn role_and_profile_rules() {
        let oc = HarnessEngine::Opencode.strategy();
        assert!(oc.validate_role("normal").is_ok());
        assert!(oc.validate_role("steerable").is_err());
        assert!(HarnessEngine::Claw
            .strategy()
            .validate_role("master")
            .is_ok());
        let app = HarnessEngine::Appserver.strategy();
        assert!(app
            .validate_worker_profile(&json!({"mode":"strict"}))
            .is_ok());
        assert!(app
            .validate_worker_profile(&json!({"mode":"relaxed"}))
            .is_err());
        assert!(HarnessEngine::Claw
            .strategy()
            .validate_worker_profile(&json!({"mode":"relaxed"}))
            .is_ok());
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
    fn solve_gate_and_prepare_task() {
        let oc = HarnessEngine::Opencode.strategy();
        assert!(oc
            .gate_solve_request(&gate(true, Some("agent"), None))
            .is_ok());
        assert!(oc
            .gate_solve_request(&gate(true, Some("plan"), None))
            .unwrap_err()
            .contains("interactionMode=plan"));
        assert!(oc
            .gate_solve_request(&gate(true, None, Some("p1")))
            .is_err());
        assert!(oc.gate_solve_request(&gate(false, None, None)).is_err());
        assert!(HarnessEngine::Claw
            .strategy()
            .gate_solve_request(&gate(false, Some("plan"), None))
            .is_ok());

        let task_json = json!({
            "requestId":"r","userPrompt":"p","turnId":"t",
            "askUserQuestionEnabled":true,"forceSingleTurn":false,
            "landlockDsl": gateway_solve_turn::default_landlock_dsl(),
            "landlockDslSource":"systemDefault"
        });
        let mut task: GatewaySolveTaskFile = serde_json::from_value(task_json.clone()).unwrap();
        oc.prepare_task(&mut task);
        oc.prepare_task(&mut task);
        assert_eq!(task.interaction_mode.as_deref(), Some("agent"));
        assert_eq!(task.ask_user_question_enabled, Some(false));
        assert_eq!(task.force_single_turn, Some(true));
        let ro = &task.landlock_dsl.as_ref().unwrap().ro;
        for p in ["/proc", "/dev/urandom"] {
            assert_eq!(
                ro.iter().filter(|x| *x == p).count(),
                1,
                "{p} once in {ro:?}"
            );
        }
        gateway_solve_turn::validate_landlock_dsl(task.landlock_dsl.as_ref().unwrap()).unwrap();

        let mut claw_task: GatewaySolveTaskFile = serde_json::from_value(task_json).unwrap();
        let before = serde_json::to_value(&claw_task).unwrap();
        HarnessEngine::Claw.strategy().prepare_task(&mut claw_task);
        assert_eq!(serde_json::to_value(&claw_task).unwrap(), before);
    }
}
