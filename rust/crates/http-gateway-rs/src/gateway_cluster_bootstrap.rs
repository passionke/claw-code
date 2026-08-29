//! First-run cluster bootstrap: LLM from env + e2b template readiness + core singleton ensure.
//! Author: kejiqing

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::claw_tap_cluster_state::{self, ClawTapClusterHandle, TapConsistency};
use crate::cluster_identity::gateway_cluster_id;
use crate::gateway_e2b_core_readiness::{load_core_readiness_snapshot, observe_component_ready};
use crate::gateway_e2b_nas_api_settings::E2bNasApiSettings;
use crate::gateway_e2b_observe_settings::E2bObserveSettings;
use crate::gateway_e2b_worker_settings::E2bWorkerSettings;
use crate::gateway_global_settings::{
    self, get_gateway_global_settings, put_active_llm_config, PutActiveLlmConfigInput,
};
use crate::gateway_llm_config_sync::LlmRuntimeHandle;
use crate::pool::interactive_backend::interactive_backend_is_e2b;
use crate::pool::PoolClients;
use crate::session_db::GatewaySessionDb;
use claw_e2b_sandbox_client::E2bSandboxClient;

const BOOTSTRAP_POLL_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapPhaseId {
    ClusterIdentity,
    LlmConfig,
    E2bTemplates,
    E2bSingletons,
    ClawTapStrict,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapPhaseStatus {
    pub phase: BootstrapPhaseId,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapTemplateEntry {
    pub key: String,
    pub alias: String,
    #[serde(rename = "buildId", skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapCommand {
    pub label: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterBootstrapSettings {
    #[serde(rename = "completedAtMs", default)]
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClusterBootstrapSnapshot {
    #[serde(rename = "needsBootstrap")]
    pub needs_bootstrap: bool,
    #[serde(rename = "clusterId")]
    pub cluster_id: String,
    pub phases: Vec<BootstrapPhaseStatus>,
    #[serde(rename = "blockingReason", skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
    #[serde(rename = "envLlmAvailable")]
    pub env_llm_available: bool,
    #[serde(rename = "templateCommands")]
    pub template_commands: Vec<BootstrapCommand>,
    #[serde(rename = "templateEntries")]
    pub template_entries: Vec<BootstrapTemplateEntry>,
    #[serde(rename = "completedAtMs", skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapApplyLlmResponse {
    pub applied: bool,
    #[serde(rename = "modelName", skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapEnsureCoreResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "needsBootstrap")]
    pub needs_bootstrap: bool,
}

#[derive(Debug, Clone)]
struct EnvLlmBootstrapInput {
    api_key: String,
    base_url: String,
    model_name: String,
    name: String,
}

fn trim_non_empty(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[must_use]
pub fn env_llm_available() -> bool {
    env_llm_bootstrap_input().is_some()
}

fn env_llm_bootstrap_input() -> Option<EnvLlmBootstrapInput> {
    let api_key = trim_non_empty(std::env::var("CLAW_BOOTSTRAP_LLM_API_KEY").ok())
        .or_else(|| trim_non_empty(std::env::var("OPENAI_API_KEY").ok()))?;
    let base_url = trim_non_empty(std::env::var("CLAW_BOOTSTRAP_LLM_BASE_URL").ok())
        .or_else(|| trim_non_empty(std::env::var("UPSTREAM_OPENAI_BASE_URL").ok()))
        .or_else(|| trim_non_empty(std::env::var("OPENAI_BASE_URL").ok()))?;
    let model_name = trim_non_empty(std::env::var("CLAW_BOOTSTRAP_LLM_MODEL_NAME").ok())
        .or_else(|| trim_non_empty(std::env::var("OPENAI_MODEL").ok()))
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let name = trim_non_empty(std::env::var("CLAW_BOOTSTRAP_LLM_NAME").ok())
        .unwrap_or_else(|| "ci-bootstrap".to_string());
    Some(EnvLlmBootstrapInput {
        api_key,
        base_url,
        model_name,
        name,
    })
}

#[must_use]
fn build_id_ready(build_id: Option<&String>) -> bool {
    build_id.is_some_and(|id| !id.trim().is_empty())
}

fn template_entries_from_settings(
    observe: &E2bObserveSettings,
    nas_api: &E2bNasApiSettings,
    worker: &E2bWorkerSettings,
    worker_relaxed: &E2bWorkerSettings,
) -> Vec<BootstrapTemplateEntry> {
    vec![
        BootstrapTemplateEntry {
            key: "e2bObserve".into(),
            alias: "claw-observe".into(),
            build_id: observe.build_id.clone(),
            ready: build_id_ready(observe.build_id.as_ref()),
        },
        BootstrapTemplateEntry {
            key: "e2bNasApi".into(),
            alias: "claw-nas-api".into(),
            build_id: nas_api.build_id.clone(),
            ready: build_id_ready(nas_api.build_id.as_ref()),
        },
        BootstrapTemplateEntry {
            key: "e2bWorker".into(),
            alias: worker
                .alias
                .clone()
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| "claw-worker".into()),
            build_id: worker.build_id.clone(),
            ready: build_id_ready(worker.build_id.as_ref()),
        },
        BootstrapTemplateEntry {
            key: "e2bWorkerRelaxed".into(),
            alias: worker_relaxed
                .alias
                .clone()
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| "claw-worker-relaxed".into()),
            build_id: worker_relaxed.build_id.clone(),
            ready: build_id_ready(worker_relaxed.build_id.as_ref()),
        },
    ]
}

#[must_use]
pub fn template_build_commands(cluster_id: &str) -> Vec<BootstrapCommand> {
    let cid = cluster_id.trim();
    vec![
        BootstrapCommand {
            label: "全量核心模板（observe + nas-api + worker strict/relaxed）".into(),
            command: format!(
                "export CLAW_CLUSTER_ID={cid}\nexport CLAW_GATEWAY_DATABASE_URL='<workbox-pg-url>'\n./deploy/e2b/build-selfhosted-templates.sh"
            ),
            hint: Some(
                "在开发机 claw-code 仓库根目录执行；PG URL 与 workbox Gateway 相同。见 deploy/e2b/WORKER-BUILD.md"
                    .into(),
            ),
        },
        BootstrapCommand {
            label: "Worker strict + relaxed（日常增量）".into(),
            command: format!(
                "export CLAW_CLUSTER_ID={cid}\nexport CLAW_GATEWAY_DATABASE_URL='<workbox-pg-url>'\n./deploy/stack/gateway.sh e2b-worker-deploy"
            ),
            hint: Some(
                "Mac arm64 可加 --from-ci-image release-vX.Y.Z。构建完成后 PG 出现 e2bWorker / e2bWorkerRelaxed buildId"
                    .into(),
            ),
        },
    ]
}

async fn llm_phase_complete(db: &GatewaySessionDb) -> Result<bool, sqlx::Error> {
    let active = gateway_global_settings::load_active_llm_runtime(db).await?;
    Ok(active.is_some_and(|a| {
        !a.api_key.trim().is_empty()
            && !a.base_model_url.trim().is_empty()
            && !a.model_name.trim().is_empty()
    }))
}

async fn templates_phase_complete(db: &GatewaySessionDb) -> Result<bool, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    let entries = template_entries_from_settings(
        &settings.e2b_observe,
        &settings.e2b_nas_api,
        &settings.e2b_worker,
        &settings.e2b_worker_relaxed,
    );
    Ok(entries.iter().all(|e| e.ready))
}

fn first_incomplete_phase(phases: &[BootstrapPhaseStatus]) -> Option<String> {
    phases.iter().find(|p| !p.complete).map(|p| match p.phase {
        BootstrapPhaseId::ClusterIdentity => "cluster identity not ready".into(),
        BootstrapPhaseId::LlmConfig => p
            .detail
            .clone()
            .unwrap_or_else(|| "active LLM not configured".into()),
        BootstrapPhaseId::E2bTemplates => p.detail.clone().unwrap_or_else(|| {
            "e2b template buildId missing (observe / nas-api / worker / worker-relaxed)".into()
        }),
        BootstrapPhaseId::E2bSingletons => p
            .detail
            .clone()
            .unwrap_or_else(|| "e2b core singletons not online".into()),
        BootstrapPhaseId::ClawTapStrict => p
            .detail
            .clone()
            .unwrap_or_else(|| "clawTap cluster not strict".into()),
    })
}

/// Lightweight bootstrap status (no e2b HTTP). Used at process startup gate.
pub async fn cluster_needs_bootstrap(db: &GatewaySessionDb) -> Result<bool, sqlx::Error> {
    if !interactive_backend_is_e2b() {
        return Ok(false);
    }
    let snap = cluster_bootstrap_status(db, None, None).await?;
    Ok(snap.needs_bootstrap)
}

pub async fn cluster_bootstrap_status(
    db: &GatewaySessionDb,
    client: Option<&E2bSandboxClient>,
    claw_tap_cluster: Option<&ClawTapClusterHandle>,
) -> Result<ClusterBootstrapSnapshot, sqlx::Error> {
    if !interactive_backend_is_e2b() {
        let cluster_id = gateway_cluster_id().unwrap_or_else(|_| "unset".into());
        return Ok(ClusterBootstrapSnapshot {
            needs_bootstrap: false,
            cluster_id,
            phases: vec![],
            blocking_reason: None,
            env_llm_available: env_llm_available(),
            template_commands: vec![],
            template_entries: vec![],
            completed_at_ms: None,
        });
    }

    let cluster_id = gateway_cluster_id()
        .map_err(|e| sqlx::Error::Configuration(format!("CLAW_CLUSTER_ID: {e}").into()))?;
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    let bootstrap_meta = settings.cluster_bootstrap.clone();

    let cluster_identity = !cluster_id.trim().is_empty();
    let llm_ok = llm_phase_complete(db).await?;
    let template_entries = template_entries_from_settings(
        &settings.e2b_observe,
        &settings.e2b_nas_api,
        &settings.e2b_worker,
        &settings.e2b_worker_relaxed,
    );
    let templates_ok = template_entries.iter().all(|e| e.ready);

    let mut singletons_ok = false;
    let mut singletons_detail: Option<String> = None;
    if templates_ok && llm_ok {
        if let Some(c) = client {
            let empty_tap = Arc::new(tokio::sync::RwLock::new(None));
            let tap_handle = claw_tap_cluster.unwrap_or(&empty_tap);
            let core = load_core_readiness_snapshot(db, Some(c), tap_handle).await?;
            singletons_ok = observe_component_ready(&core.observe)
                && crate::gateway_e2b_core_readiness::nas_api_component_ready(&core.nas_api);
            if !singletons_ok {
                singletons_detail = core.reason;
            }
        } else {
            singletons_detail = Some(
                "e2b sandbox client unavailable; call ensure-core after templates ready".into(),
            );
        }
    } else if !templates_ok {
        let missing: Vec<_> = template_entries
            .iter()
            .filter(|e| !e.ready)
            .map(|e| e.key.as_str())
            .collect();
        singletons_detail = Some(format!(
            "waiting for template buildId: {}",
            missing.join(", ")
        ));
    } else {
        singletons_detail = Some("waiting for active LLM".into());
    }

    let mut claw_tap_ok = false;
    let mut claw_tap_detail: Option<String> = None;
    if singletons_ok {
        if let Some(handle) = claw_tap_cluster {
            let snap = claw_tap_cluster_state::snapshot_from_handle(handle).await;
            claw_tap_ok = snap.consistency == TapConsistency::Strict;
            if !claw_tap_ok {
                claw_tap_detail = snap
                    .reason
                    .or_else(|| Some(format!("clawTap consistency {:?}", snap.consistency)));
            }
        } else {
            claw_tap_detail = Some("clawTap cluster state not refreshed yet".into());
        }
    } else {
        claw_tap_detail = Some("waiting for e2b singletons".into());
    }

    let phases = vec![
        BootstrapPhaseStatus {
            phase: BootstrapPhaseId::ClusterIdentity,
            complete: cluster_identity,
            detail: if cluster_identity {
                None
            } else {
                Some("CLAW_CLUSTER_ID invalid".into())
            },
        },
        BootstrapPhaseStatus {
            phase: BootstrapPhaseId::LlmConfig,
            complete: llm_ok,
            detail: if llm_ok {
                None
            } else {
                Some("no active LLM in PG (Admin Apply or env bootstrap)".into())
            },
        },
        BootstrapPhaseStatus {
            phase: BootstrapPhaseId::E2bTemplates,
            complete: templates_ok,
            detail: if templates_ok {
                None
            } else {
                let missing: Vec<_> = template_entries
                    .iter()
                    .filter(|e| !e.ready)
                    .map(|e| format!("{} ({})", e.key, e.alias))
                    .collect();
                Some(format!("missing buildId: {}", missing.join(", ")))
            },
        },
        BootstrapPhaseStatus {
            phase: BootstrapPhaseId::E2bSingletons,
            complete: singletons_ok,
            detail: singletons_detail,
        },
        BootstrapPhaseStatus {
            phase: BootstrapPhaseId::ClawTapStrict,
            complete: claw_tap_ok,
            detail: claw_tap_detail,
        },
    ];

    let all_complete = phases.iter().all(|p| p.complete);
    let needs_bootstrap = !all_complete;
    let blocking_reason = if all_complete {
        None
    } else {
        first_incomplete_phase(&phases)
    };

    Ok(ClusterBootstrapSnapshot {
        needs_bootstrap,
        cluster_id: cluster_id.clone(),
        phases,
        blocking_reason,
        env_llm_available: env_llm_available(),
        template_commands: template_build_commands(&cluster_id),
        template_entries,
        completed_at_ms: bootstrap_meta.completed_at_ms,
    })
}

pub async fn apply_llm_from_env(
    db: &GatewaySessionDb,
    llm_handle: &LlmRuntimeHandle,
) -> Result<BootstrapApplyLlmResponse, String> {
    let Some(input) = env_llm_bootstrap_input() else {
        return Ok(BootstrapApplyLlmResponse {
            applied: false,
            model_name: None,
            message: Some(
                "set CLAW_BOOTSTRAP_LLM_API_KEY + CLAW_BOOTSTRAP_LLM_BASE_URL (or OPENAI_* ) in .env"
                    .into(),
            ),
        });
    };
    if llm_phase_complete(db).await.map_err(|e| e.to_string())? {
        return Ok(BootstrapApplyLlmResponse {
            applied: false,
            model_name: Some(input.model_name.clone()),
            message: Some("active LLM already configured in PG".into()),
        });
    }
    put_active_llm_config(
        db,
        PutActiveLlmConfigInput {
            name: Some(input.name),
            base_model_url: input.base_url,
            model_name: input.model_name.clone(),
            api_key: Some(input.api_key),
            note: Some("cluster bootstrap from env".into()),
        },
    )
    .await?;
    crate::gateway_llm_config_sync::sync_llm_runtime_from_db(db, llm_handle).await?;
    info!(
        target: "claw_gateway_bootstrap",
        model = %input.model_name,
        "active LLM applied from env (bootstrap)"
    );
    Ok(BootstrapApplyLlmResponse {
        applied: true,
        model_name: Some(input.model_name),
        message: None,
    })
}

pub async fn ensure_bootstrap_core(
    db: &GatewaySessionDb,
    pool_clients: &PoolClients,
    llm_handle: &LlmRuntimeHandle,
    claw_tap_cluster: &ClawTapClusterHandle,
) -> Result<BootstrapEnsureCoreResponse, String> {
    if !interactive_backend_is_e2b() {
        return Ok(BootstrapEnsureCoreResponse {
            ok: true,
            message: Some("bootstrap not applicable (non-e2b backend)".into()),
            needs_bootstrap: false,
        });
    }
    if !templates_phase_complete(db)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("e2b templates not ready — build on dev machine first".into());
    }
    if !llm_phase_complete(db).await.map_err(|e| e.to_string())? {
        return Err("active LLM not configured — apply from env or Admin".into());
    }
    let client = pool_clients
        .e2b_sandbox_client()
        .ok_or_else(|| "e2b client not configured".to_string())?;
    crate::gateway_e2b_singleton_lifecycle::ensure_e2b_singletons_on_startup_strict(
        db,
        client.as_ref(),
    )
    .await?;
    if let Ok(Some(cluster)) =
        claw_tap_cluster_state::refresh_claw_tap_cluster_state(db, llm_handle).await
    {
        *claw_tap_cluster.write().await = Some(cluster);
    }
    let snap = cluster_bootstrap_status(db, Some(client.as_ref()), Some(claw_tap_cluster))
        .await
        .map_err(|e| e.to_string())?;
    if !snap.needs_bootstrap {
        mark_cluster_bootstrap_completed(db)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(BootstrapEnsureCoreResponse {
        ok: !snap.needs_bootstrap,
        message: snap.blocking_reason,
        needs_bootstrap: snap.needs_bootstrap,
    })
}

pub async fn mark_cluster_bootstrap_completed(db: &GatewaySessionDb) -> Result<(), sqlx::Error> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    db.merge_gateway_global_settings_json(
        &["clusterBootstrap"],
        &serde_json::json!({ "completedAtMs": now_ms }),
    )
    .await
}

pub fn spawn_bootstrap_reconcile_loop(
    db: Arc<GatewaySessionDb>,
    pool_clients: PoolClients,
    llm_handle: LlmRuntimeHandle,
    claw_tap_cluster: ClawTapClusterHandle,
) {
    tokio::spawn(async move {
        let start =
            tokio::time::Instant::now() + std::time::Duration::from_secs(BOOTSTRAP_POLL_SECS);
        let mut ticker =
            tokio::time::interval_at(start, std::time::Duration::from_secs(BOOTSTRAP_POLL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            match cluster_needs_bootstrap(db.as_ref()).await {
                Ok(false) => continue,
                Ok(true) => {}
                Err(e) => {
                    warn!(
                        target: "claw_gateway_bootstrap",
                        error = %e,
                        "bootstrap reconcile: status check failed"
                    );
                    continue;
                }
            }
            if !templates_phase_complete(db.as_ref()).await.unwrap_or(false) {
                continue;
            }
            if !llm_phase_complete(db.as_ref()).await.unwrap_or(false) {
                continue;
            }
            match ensure_bootstrap_core(db.as_ref(), &pool_clients, &llm_handle, &claw_tap_cluster)
                .await
            {
                Ok(resp) if !resp.needs_bootstrap => {
                    info!(
                        target: "claw_gateway_bootstrap",
                        "cluster bootstrap reconcile complete"
                    );
                    let poll_db = Arc::clone(&db);
                    pool_clients.spawn_singleton_health_reconcile_loop(poll_db);
                    return;
                }
                Ok(resp) => {
                    warn!(
                        target: "claw_gateway_bootstrap",
                        message = ?resp.message,
                        "bootstrap reconcile: core not fully ready"
                    );
                }
                Err(e) => {
                    warn!(
                        target: "claw_gateway_bootstrap",
                        error = %e,
                        "bootstrap reconcile: ensure-core failed"
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_llm_parses_bootstrap_vars() {
        std::env::set_var("CLAW_BOOTSTRAP_LLM_API_KEY", "sk-test");
        std::env::set_var("CLAW_BOOTSTRAP_LLM_BASE_URL", "https://api.example.com/v1");
        std::env::set_var("CLAW_BOOTSTRAP_LLM_MODEL_NAME", "mock-model");
        let input = env_llm_bootstrap_input().expect("env llm");
        assert_eq!(input.api_key, "sk-test");
        assert_eq!(input.base_url, "https://api.example.com/v1");
        assert_eq!(input.model_name, "mock-model");
        std::env::remove_var("CLAW_BOOTSTRAP_LLM_API_KEY");
        std::env::remove_var("CLAW_BOOTSTRAP_LLM_BASE_URL");
        std::env::remove_var("CLAW_BOOTSTRAP_LLM_MODEL_NAME");
    }

    #[test]
    fn template_commands_include_cluster_id() {
        let cmds = template_build_commands("workbox-20260828");
        assert_eq!(cmds.len(), 2);
        assert!(cmds[0].command.contains("CLAW_CLUSTER_ID=workbox-20260828"));
    }

    #[test]
    fn build_id_ready_rejects_empty() {
        assert!(!build_id_ready(Some(&String::new())));
        assert!(!build_id_ready(Some(&"  ".into())));
        assert!(build_id_ready(Some(&"uuid-1".into())));
    }

    #[test]
    fn first_incomplete_phase_returns_llm() {
        let phases = vec![
            BootstrapPhaseStatus {
                phase: BootstrapPhaseId::ClusterIdentity,
                complete: true,
                detail: None,
            },
            BootstrapPhaseStatus {
                phase: BootstrapPhaseId::LlmConfig,
                complete: false,
                detail: None,
            },
        ];
        let reason = first_incomplete_phase(&phases).unwrap_or_default();
        assert!(reason.contains("LLM"));
    }
}
