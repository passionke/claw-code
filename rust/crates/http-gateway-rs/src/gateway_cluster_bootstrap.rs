//! First-run cluster bootstrap: LLM from env + e2b template readiness + core singleton ensure.
//! Author: kejiqing

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::claw_tap_cluster_state::{
    self, ClawTapClusterHandle, ClawTapClusterSnapshot, TapConsistency,
};
use crate::cluster_identity::gateway_cluster_id;
use crate::gateway_bootstrap_publish::{self, BootstrapPublishJob};
use crate::gateway_e2b_core_readiness::{load_core_readiness_snapshot, observe_component_ready};
use crate::gateway_e2b_nas_api_settings::E2bNasApiSettings;
use crate::gateway_e2b_observe_settings::E2bObserveSettings;
use crate::gateway_e2b_singleton_api::{self, E2bSingletonsStatusResponse};
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
    #[serde(rename = "imageRef", skip_serializing_if = "Option::is_none")]
    pub image_ref: Option<String>,
    #[serde(rename = "imageDigest", skip_serializing_if = "Option::is_none")]
    pub image_digest: Option<String>,
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

#[derive(Debug, Clone, Serialize, ToSchema)]
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
    /// Suggested ACR/CI tag for Admin publish (other pipelines produce the image). Author: kejiqing
    #[serde(
        rename = "suggestedCiImageTag",
        skip_serializing_if = "Option::is_none"
    )]
    pub suggested_ci_image_tag: Option<String>,
    #[serde(rename = "publishJob", skip_serializing_if = "Option::is_none")]
    pub publish_job: Option<BootstrapPublishJob>,
    #[serde(rename = "completedAtMs", skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub singletons: Option<E2bSingletonsStatusResponse>,
    #[serde(rename = "clawTap", skip_serializing_if = "Option::is_none")]
    pub claw_tap: Option<ClawTapClusterSnapshot>,
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
    worker_opencode: &E2bWorkerSettings,
    worker_appserver: &E2bWorkerSettings,
) -> Vec<BootstrapTemplateEntry> {
    vec![
        BootstrapTemplateEntry {
            key: "e2bObserve".into(),
            alias: "claw-observe".into(),
            build_id: observe.build_id.clone(),
            image_ref: observe.image_ref.clone(),
            image_digest: observe.image_digest.clone(),
            ready: build_id_ready(observe.build_id.as_ref()),
        },
        BootstrapTemplateEntry {
            key: "e2bNasApi".into(),
            alias: "claw-nas-api".into(),
            build_id: nas_api.build_id.clone(),
            image_ref: nas_api.image_ref.clone(),
            image_digest: nas_api.image_digest.clone(),
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
            image_ref: worker.image_ref.clone(),
            image_digest: worker.image_digest.clone(),
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
            image_ref: worker_relaxed.image_ref.clone(),
            image_digest: worker_relaxed.image_digest.clone(),
            ready: build_id_ready(worker_relaxed.build_id.as_ref()),
        },
        BootstrapTemplateEntry {
            key: "e2bWorkerOpencode".into(),
            alias: worker_opencode
                .alias
                .clone()
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| "claw-worker-opencode".into()),
            build_id: worker_opencode.build_id.clone(),
            image_ref: worker_opencode.image_ref.clone(),
            image_digest: worker_opencode.image_digest.clone(),
            ready: build_id_ready(worker_opencode.build_id.as_ref()),
        },
        BootstrapTemplateEntry {
            key: "e2bWorkerAppserver".into(),
            alias: worker_appserver
                .alias
                .clone()
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| "claw-worker-appserver".into()),
            build_id: worker_appserver.build_id.clone(),
            image_ref: worker_appserver.image_ref.clone(),
            image_digest: worker_appserver.image_digest.clone(),
            ready: build_id_ready(worker_appserver.build_id.as_ref()),
        },
    ]
}

/// While a scoped publish job runs, blank only in-scope rows so the other step stays visible.
/// Author: kejiqing
fn blank_template_entries_for_publish_job(
    entries: Vec<BootstrapTemplateEntry>,
    job: &BootstrapPublishJob,
) -> Vec<BootstrapTemplateEntry> {
    if job.phase != gateway_bootstrap_publish::BootstrapPublishPhase::Running {
        return entries;
    }
    let Some(scope) = job.scope else {
        // Legacy / unknown scope: keep prior conservative behavior (blank all).
        return entries
            .into_iter()
            .map(|mut e| {
                e.build_id = None;
                e.ready = false;
                e
            })
            .collect();
    };
    entries
        .into_iter()
        .map(|mut e| {
            if scope.affects_key(&e.key) {
                e.build_id = None;
                e.ready = false;
            }
            e
        })
        .collect()
}

#[must_use]
pub fn template_build_commands(cluster_id: &str) -> Vec<BootstrapCommand> {
    let cid = cluster_id.trim();
    vec![BootstrapCommand {
        label: "Admin：选用 ACR/CI 镜像 tag 发布 e2b 模板（worker 系 + nas-api）".into(),
        command: "POST /v1/gateway/bootstrap/publish-templates {\"imageTag\":\"release-vX.Y.Z\"}"
            .into(),
        hint: Some(format!(
            "在引导页填写 CI/ACR 已有 tag（如 release-v1.8.11），点「发布模板」。\
             Gateway 会调 e2b Template.build（worker / relaxed / nas-api / opencode / appserver）；\
             observe 用第一步 claw-tap tag 单独发布。\
             CLAW_CLUSTER_ID={cid}。镜像构建由其他链路负责。"
        )),
    }]
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
        &settings.e2b_worker_opencode,
        &settings.e2b_worker_appserver,
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
            "e2b template buildId missing (observe / nas-api / worker / relaxed / opencode / appserver)"
                .into()
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

/// Lightweight bootstrap status (no e2b HTTP). Used for Admin/UI — **not** the startup ensure gate.
/// Author: kejiqing
pub async fn cluster_needs_bootstrap(db: &GatewaySessionDb) -> Result<bool, sqlx::Error> {
    if !interactive_backend_is_e2b() {
        return Ok(false);
    }
    let snap = cluster_bootstrap_status(db, None, None).await?;
    Ok(snap.needs_bootstrap)
}

/// Cluster init marker: `true` = first-run / reset init state (Admin wizard not acked).
/// This is the single source of truth for "serve without ensure/reconcile/warm" — init is the
/// highest priority recovery path and must never be blocked by worker lifecycle. Author: kejiqing
pub async fn cluster_init_pending(db: &GatewaySessionDb) -> Result<bool, String> {
    if !interactive_backend_is_e2b() {
        return Ok(false);
    }
    let (settings, _, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| format!("load global settings for init gate: {e}"))?;
    Ok(settings.cluster_bootstrap.completed_at_ms.is_none())
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
            suggested_ci_image_tag: gateway_bootstrap_publish::suggested_ci_image_tag(),
            publish_job: Some(gateway_bootstrap_publish::current_publish_job()),
            completed_at_ms: None,
            singletons: None,
            claw_tap: None,
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
        &settings.e2b_worker_opencode,
        &settings.e2b_worker_appserver,
    );
    let publish_job = gateway_bootstrap_publish::current_publish_job();
    // While Admin publish is running, blank only in-scope rows so the other step stays visible.
    // Author: kejiqing
    let template_entries = blank_template_entries_for_publish_job(template_entries, &publish_job);
    let templates_ok = template_entries.iter().all(|e| e.ready)
        && publish_job.phase != gateway_bootstrap_publish::BootstrapPublishPhase::Running;

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

    let singletons = if let Some(c) = client {
        gateway_e2b_singleton_api::load_e2b_singletons_status(db, Some(c))
            .await
            .ok()
    } else {
        gateway_e2b_singleton_api::load_e2b_singletons_status(db, None)
            .await
            .ok()
    };
    let claw_tap = if let Some(handle) = claw_tap_cluster {
        Some(claw_tap_cluster_state::snapshot_from_handle(handle).await)
    } else {
        None
    };

    Ok(ClusterBootstrapSnapshot {
        needs_bootstrap,
        cluster_id: cluster_id.clone(),
        phases,
        blocking_reason,
        env_llm_available: env_llm_available(),
        template_commands: template_build_commands(&cluster_id),
        template_entries,
        suggested_ci_image_tag: gateway_bootstrap_publish::suggested_ci_image_tag(),
        publish_job: Some(publish_job),
        completed_at_ms: bootstrap_meta.completed_at_ms,
        singletons,
        claw_tap,
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
        return Err(
            "e2b templates not ready — publish from ACR/CI tag in Admin bootstrap step 3".into(),
        );
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
    // Do NOT mark wizard complete here — Admin must keep the 验收/可选 steps until the
    // user explicitly POST /bootstrap/complete. Author: kejiqing
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

/// User finished the Admin wizard (验收/可选). Requires infra phases complete. Author: kejiqing
pub async fn complete_cluster_bootstrap_wizard(
    db: &GatewaySessionDb,
    client: Option<&E2bSandboxClient>,
    claw_tap_cluster: Option<&ClawTapClusterHandle>,
) -> Result<BootstrapEnsureCoreResponse, String> {
    let snap = cluster_bootstrap_status(db, client, claw_tap_cluster)
        .await
        .map_err(|e| e.to_string())?;
    if snap.needs_bootstrap {
        return Err(snap
            .blocking_reason
            .unwrap_or_else(|| "bootstrap phases incomplete".into()));
    }
    mark_cluster_bootstrap_completed(db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(BootstrapEnsureCoreResponse {
        ok: true,
        message: None,
        needs_bootstrap: false,
    })
}

/// Clear wizard ack so Admin shows the guide again (ops / after accidental skip). Author: kejiqing
pub async fn reopen_cluster_bootstrap_wizard(db: &GatewaySessionDb) -> Result<(), String> {
    db.merge_gateway_global_settings_json(
        &["clusterBootstrap", "completedAtMs"],
        &serde_json::Value::Null,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Clear PG template pins + singleton runtime after e2b endpoint move. Author: kejiqing
pub async fn invalidate_e2b_template_pins(db: &GatewaySessionDb) -> Result<(), String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    // Field-level clears — do not replace whole e2bWorker (keeps poolSize). Author: kejiqing
    for (section, fields) in [
        ("e2bWorker", &["templateId", "buildId", "alias"][..]),
        ("e2bWorkerRelaxed", &["templateId", "buildId", "alias"][..]),
        ("e2bWorkerOpencode", &["templateId", "buildId", "alias"][..]),
        (
            "e2bWorkerAppserver",
            &["templateId", "buildId", "alias"][..],
        ),
        (
            "e2bNasApi",
            &[
                "templateId",
                "buildId",
                "appliedBuildId",
                "baseUrl",
                "sandboxId",
            ][..],
        ),
        (
            "e2bObserve",
            &[
                "templateId",
                "buildId",
                "appliedBuildId",
                "baseUrl",
                "sandboxId",
            ][..],
        ),
    ] {
        for field in fields {
            db.merge_gateway_global_settings_json(&[section, field], &serde_json::Value::Null)
                .await
                .map_err(|e| format!("clear {section}.{field}: {e}"))?;
        }
        db.merge_gateway_global_settings_json(
            &[section, "updatedAtMs"],
            &serde_json::json!(now_ms),
        )
        .await
        .map_err(|e| format!("touch {section}.updatedAtMs: {e}"))?;
    }
    // Option URL fields: JSON null is fine. `host` is a String — write "" (null poisons
    // whole settings_json parse via unwrap_or_default). Author: kejiqing
    for field in [
        "e2bObserveSandboxId",
        "proxyBaseUrl",
        "liveBaseUrl",
        "liveSessionUrlTemplate",
    ] {
        db.merge_gateway_global_settings_json(&["clawTap", field], &serde_json::Value::Null)
            .await
            .map_err(|e| format!("clear clawTap.{field}: {e}"))?;
    }
    db.merge_gateway_global_settings_json(&["clawTap", "host"], &serde_json::json!(""))
        .await
        .map_err(|e| format!("clear clawTap.host: {e}"))?;
    info!(
        target: "claw_gateway_bootstrap",
        "invalidated e2b template pins + singleton runtime (endpoint change or reset)"
    );
    Ok(())
}

/// Drop wizard ack + template pins + active LLM so Admin init can be run again end-to-end.
/// Keeps LLM model catalog/keys; only clears the active pointer. Author: kejiqing
pub async fn reset_bootstrap_for_rerun(db: &GatewaySessionDb) -> Result<(), String> {
    invalidate_e2b_template_pins(db).await?;
    clear_active_llm_for_bootstrap_reset(db).await?;
    reopen_cluster_bootstrap_wizard(db).await?;
    Ok(())
}

async fn clear_active_llm_for_bootstrap_reset(db: &GatewaySessionDb) -> Result<(), String> {
    let cluster_id = gateway_cluster_id().map_err(|e| e.clone())?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    db.save_llm_cluster_state(&cluster_id, "", "", None, now_ms)
        .await
        .map_err(|e| format!("clear active LLM: {e}"))?;
    info!(
        target: "claw_gateway_bootstrap",
        "cleared active LLM pointer for bootstrap reset (catalog kept)"
    );
    Ok(())
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
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].command.contains("publish-templates"));
        assert!(cmds[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("CLAW_CLUSTER_ID=workbox-20260828")));
    }

    #[test]
    fn build_id_ready_rejects_empty() {
        assert!(!build_id_ready(Some(&String::new())));
        assert!(!build_id_ready(Some(&"  ".into())));
        assert!(build_id_ready(Some(&"uuid-1".into())));
    }

    #[test]
    fn template_entries_include_opencode_and_appserver() {
        let mut observe = E2bObserveSettings::default();
        observe.build_id = Some("obs-1".into());
        let mut nas = E2bNasApiSettings::default();
        nas.build_id = Some("nas-1".into());
        let mut worker = E2bWorkerSettings::default();
        worker.build_id = Some("w-1".into());
        worker.image_ref = Some("reg/claw-gateway-worker:t".into());
        worker.image_digest = Some("sha256:abc".into());
        let mut relaxed = E2bWorkerSettings::default();
        relaxed.build_id = Some("r-1".into());
        let mut opencode = E2bWorkerSettings::default();
        opencode.build_id = Some("o-1".into());
        let mut appserver = E2bWorkerSettings::default();
        appserver.build_id = Some("a-1".into());
        let entries = template_entries_from_settings(
            &observe, &nas, &worker, &relaxed, &opencode, &appserver,
        );
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[0].key, "e2bObserve");
        assert_eq!(entries[4].key, "e2bWorkerOpencode");
        assert_eq!(entries[4].alias, "claw-worker-opencode");
        assert_eq!(entries[5].key, "e2bWorkerAppserver");
        assert_eq!(entries[5].alias, "claw-worker-appserver");
        assert!(entries.iter().all(|e| e.ready));
        assert_eq!(
            entries[2].image_ref.as_deref(),
            Some("reg/claw-gateway-worker:t")
        );
    }

    #[test]
    fn blank_entries_only_clears_observe_scope() {
        let mut observe = E2bObserveSettings::default();
        observe.build_id = Some("obs-1".into());
        observe.image_ref = Some("tap:v1".into());
        let mut nas = E2bNasApiSettings::default();
        nas.build_id = Some("nas-1".into());
        nas.image_ref = Some("debian".into());
        let worker = E2bWorkerSettings {
            build_id: Some("w-1".into()),
            image_ref: Some("worker:t".into()),
            ..Default::default()
        };
        let relaxed = E2bWorkerSettings {
            build_id: Some("r-1".into()),
            ..Default::default()
        };
        let opencode = E2bWorkerSettings {
            build_id: Some("o-1".into()),
            ..Default::default()
        };
        let appserver = E2bWorkerSettings {
            build_id: Some("a-1".into()),
            ..Default::default()
        };
        let entries = template_entries_from_settings(
            &observe, &nas, &worker, &relaxed, &opencode, &appserver,
        );
        let job = BootstrapPublishJob {
            phase: gateway_bootstrap_publish::BootstrapPublishPhase::Running,
            scope: Some(gateway_bootstrap_publish::BootstrapPublishScope::Observe),
            ..Default::default()
        };
        let blanked = blank_template_entries_for_publish_job(entries, &job);
        let obs = blanked.iter().find(|e| e.key == "e2bObserve").unwrap();
        assert!(!obs.ready);
        assert!(obs.build_id.is_none());
        // image_ref kept on blank so UI can still show source while pending
        assert_eq!(obs.image_ref.as_deref(), Some("tap:v1"));
        let worker_row = blanked.iter().find(|e| e.key == "e2bWorker").unwrap();
        assert!(worker_row.ready);
        assert_eq!(worker_row.build_id.as_deref(), Some("w-1"));
        assert_eq!(worker_row.image_ref.as_deref(), Some("worker:t"));
    }

    #[test]
    fn blank_entries_worker_set_keeps_observe() {
        let observe = E2bObserveSettings {
            build_id: Some("obs-1".into()),
            image_ref: Some("tap:v1".into()),
            ..Default::default()
        };
        let nas = E2bNasApiSettings {
            build_id: Some("nas-1".into()),
            ..Default::default()
        };
        let worker = E2bWorkerSettings {
            build_id: Some("w-1".into()),
            ..Default::default()
        };
        let relaxed = E2bWorkerSettings::default();
        let opencode = E2bWorkerSettings::default();
        let appserver = E2bWorkerSettings::default();
        let entries = template_entries_from_settings(
            &observe, &nas, &worker, &relaxed, &opencode, &appserver,
        );
        let job = BootstrapPublishJob {
            phase: gateway_bootstrap_publish::BootstrapPublishPhase::Running,
            scope: Some(gateway_bootstrap_publish::BootstrapPublishScope::WorkerSet),
            ..Default::default()
        };
        let blanked = blank_template_entries_for_publish_job(entries, &job);
        let obs = blanked.iter().find(|e| e.key == "e2bObserve").unwrap();
        assert!(obs.ready);
        assert_eq!(obs.build_id.as_deref(), Some("obs-1"));
        let nas_row = blanked.iter().find(|e| e.key == "e2bNasApi").unwrap();
        assert!(!nas_row.ready);
        assert!(nas_row.build_id.is_none());
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
