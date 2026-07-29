//! Project-level LLM config (optional override of cluster global). Author: kejiqing
//!
//! Unconfigured / no active model → inherit global. Active project model → override mode.

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::{
    ActiveLlmConfigPublic, ActiveLlmRuntime, ApplyLlmModelResponse, LlmModelEntry, LlmModelPublic,
    LlmModelVersionsResponse, LlmModelsStore, PutLlmModelInput,
};
use crate::gateway_llm_cluster_store::resolve_llm_cluster_id;
use crate::gateway_llm_model_apply::LlmModelApplyOutcome;
use crate::gateway_llm_model_revision::{format_model_rev_local_ms, llm_api_key_slot};
use crate::gateway_llm_project_store::{load_project_llm_store, save_project_llm_store};
use crate::session_db::{
    GatewayLlmProjectObserveRow, GatewayLlmProjectRevisionRow, GatewaySessionDb,
};

/// Inference mode for a project. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectLlmMode {
    Inherit,
    Override,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectObservePublic {
    #[serde(rename = "configured")]
    pub configured: bool,
    #[serde(rename = "sandboxId", skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(rename = "proxyBaseUrl", skip_serializing_if = "Option::is_none")]
    pub proxy_base_url: Option<String>,
    #[serde(rename = "liveBaseUrl", skip_serializing_if = "Option::is_none")]
    pub live_base_url: Option<String>,
    #[serde(rename = "host", skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(rename = "proxyPort", skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<i32>,
    #[serde(rename = "livePort", skip_serializing_if = "Option::is_none")]
    pub live_port: Option<i32>,
    #[serde(rename = "updatedAtMs", skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
    #[serde(
        rename = "e2bObserveSandboxRunning",
        skip_serializing_if = "Option::is_none"
    )]
    pub e2b_observe_sandbox_running: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectInferenceSettingsResponse {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    pub mode: ProjectLlmMode,
    #[serde(rename = "clusterId", skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(rename = "llmModels", default)]
    pub llm_models: Vec<LlmModelPublic>,
    #[serde(rename = "activeLlmModelId", skip_serializing_if = "Option::is_none")]
    pub active_llm_model_id: Option<String>,
    #[serde(rename = "activeLlmModelRev", skip_serializing_if = "Option::is_none")]
    pub active_llm_model_rev: Option<String>,
    #[serde(
        rename = "activeLlmAppliedAtMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub active_llm_applied_at_ms: Option<i64>,
    #[serde(rename = "activeLlmConfig", skip_serializing_if = "Option::is_none")]
    pub active_llm_config: Option<ActiveLlmConfigPublic>,
    #[serde(rename = "observe")]
    pub observe: ProjectObservePublic,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn normalize_llm_id(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(s.to_string())
}

fn allocate_llm_id(existing: &[LlmModelEntry]) -> String {
    let base = format!("llm-{}", now_ms());
    if existing.iter().any(|m| m.id == base) {
        format!("{base}-2")
    } else {
        base
    }
}

fn prune_llm_api_keys_for_model(store: &mut LlmModelsStore, model_id: &str) {
    let at_prefix = format!("{model_id}@");
    let colon_prefix = format!("{model_id}:");
    store.api_keys.retain(|k, _| {
        k != model_id && !k.starts_with(&at_prefix) && !k.starts_with(&colon_prefix)
    });
}

fn llm_api_key_for(store: &LlmModelsStore, model_id: &str, model_rev: &str) -> Option<String> {
    let slot = llm_api_key_slot(model_id, model_rev);
    store
        .api_keys
        .get(&slot)
        .cloned()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| {
            store
                .api_keys
                .get(model_id)
                .cloned()
                .filter(|k| !k.trim().is_empty())
        })
}

fn resolve_llm_api_key_on_save(
    store: &LlmModelsStore,
    model_id: &str,
    current_rev: &str,
    input_key: Option<&str>,
    is_new: bool,
) -> Result<String, String> {
    if let Some(key) = input_key.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(key.to_string());
    }
    if is_new {
        return Err("apiKey is required".into());
    }
    llm_api_key_for(store, model_id, current_rev).ok_or_else(|| "apiKey is required".into())
}

fn normalize_revision_note(note: Option<String>) -> Option<String> {
    note.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// True when project has an active custom LLM (override mode). Author: kejiqing
pub fn project_has_active_llm(store: &LlmModelsStore) -> bool {
    !store.active_id.trim().is_empty() && !store.active_rev.trim().is_empty()
}

pub fn project_llm_mode(store: &LlmModelsStore) -> ProjectLlmMode {
    if project_has_active_llm(store) {
        ProjectLlmMode::Override
    } else {
        ProjectLlmMode::Inherit
    }
}

async fn load_store(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<(String, LlmModelsStore), String> {
    let cluster_id =
        resolve_llm_cluster_id().ok_or_else(|| "CLAW_CLUSTER_ID is not set".to_string())?;
    let store = load_project_llm_store(db, &cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok((cluster_id, store))
}

async fn llm_entry_to_public(
    db: &GatewaySessionDb,
    cluster_id: &str,
    proj_id: i64,
    entry: &LlmModelEntry,
    store: &LlmModelsStore,
) -> Result<LlmModelPublic, sqlx::Error> {
    let current_rev = if entry.current_rev.is_empty() {
        format_model_rev_local_ms(entry.updated_at_ms)
    } else {
        entry.current_rev.clone()
    };
    let (name, base_model_url, model_name, supports_vision) = match db
        .get_llm_project_revision(cluster_id, proj_id, &entry.id, &current_rev)
        .await?
    {
        Some(row) => (
            row.name,
            row.base_model_url,
            row.model_name,
            row.supports_vision,
        ),
        None => (
            entry.name.clone(),
            entry.base_model_url.clone(),
            entry.model_name.clone(),
            entry.supports_vision,
        ),
    };
    let is_active_model = !store.active_id.is_empty() && store.active_id == entry.id;
    Ok(LlmModelPublic {
        id: entry.id.clone(),
        name,
        base_model_url,
        model_name,
        supports_vision,
        current_rev: current_rev.clone(),
        api_key_set: llm_api_key_for(store, &entry.id, &current_rev).is_some(),
        active: is_active_model,
        active_rev: is_active_model
            .then(|| store.active_rev.clone())
            .filter(|r| !r.is_empty()),
        created_at_ms: entry.created_at_ms,
        updated_at_ms: entry.updated_at_ms,
    })
}

async fn llm_models_to_public(
    db: &GatewaySessionDb,
    cluster_id: &str,
    proj_id: i64,
    store: &LlmModelsStore,
) -> Result<Vec<LlmModelPublic>, sqlx::Error> {
    let mut out = Vec::with_capacity(store.models.len());
    for entry in &store.models {
        out.push(llm_entry_to_public(db, cluster_id, proj_id, entry, store).await?);
    }
    Ok(out)
}

pub async fn load_active_project_llm_runtime(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<Option<ActiveLlmRuntime>, sqlx::Error> {
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return Ok(None);
    };
    let store = load_project_llm_store(db, &cluster_id, proj_id).await?;
    if !project_has_active_llm(&store) {
        return Ok(None);
    }
    let Some(row) = db
        .get_llm_project_revision(&cluster_id, proj_id, &store.active_id, &store.active_rev)
        .await?
    else {
        return Ok(None);
    };
    let Some(api_key) = llm_api_key_for(&store, &store.active_id, &store.active_rev) else {
        return Ok(None);
    };
    Ok(Some(ActiveLlmRuntime {
        model_id: store.active_id,
        model_rev: store.active_rev,
        base_model_url: row.base_model_url,
        model_name: row.model_name,
        api_key,
        supports_vision: row.supports_vision,
        applied_at_ms: store.active_applied_at_ms,
    }))
}

/// Resolve the runtime used by a project: project override first, cluster global otherwise.
/// Author: kejiqing
pub async fn load_effective_llm_runtime(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<Option<ActiveLlmRuntime>, sqlx::Error> {
    if let Some(runtime) = load_active_project_llm_runtime(db, proj_id).await? {
        return Ok(Some(runtime));
    }
    crate::gateway_global_settings::load_active_llm_runtime(db).await
}

pub async fn load_active_project_llm_config_public(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<Option<ActiveLlmConfigPublic>, sqlx::Error> {
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return Ok(None);
    };
    let store = load_project_llm_store(db, &cluster_id, proj_id).await?;
    let Some(runtime) = load_active_project_llm_runtime(db, proj_id).await? else {
        return Ok(None);
    };
    let name = store
        .models
        .iter()
        .find(|m| m.id == runtime.model_id)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| runtime.model_id.clone());
    Ok(Some(ActiveLlmConfigPublic {
        model_id: runtime.model_id,
        name,
        base_model_url: runtime.base_model_url,
        model_name: runtime.model_name,
        api_key_set: !runtime.api_key.is_empty(),
    }))
}

pub async fn load_llm_runtime_for_project_model_id(
    db: &GatewaySessionDb,
    proj_id: i64,
    model_id: &str,
) -> Result<ActiveLlmRuntime, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let (cluster_id, store) = load_store(db, proj_id).await?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("llm model {id} not found"))?;
    let rev = if entry.current_rev.is_empty() {
        format_model_rev_local_ms(entry.updated_at_ms)
    } else {
        entry.current_rev.clone()
    };
    let Some(row) = db
        .get_llm_project_revision(&cluster_id, proj_id, &id, &rev)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Err(format!("llm model {id} revision {rev} not found"));
    };
    let api_key =
        llm_api_key_for(&store, &id, &rev).ok_or_else(|| "apiKey is not configured".to_string())?;
    Ok(ActiveLlmRuntime {
        model_id: id,
        model_rev: rev,
        base_model_url: row.base_model_url,
        model_name: row.model_name,
        api_key,
        supports_vision: row.supports_vision,
        applied_at_ms: None,
    })
}

fn observe_public_from_row(row: Option<&GatewayLlmProjectObserveRow>) -> ProjectObservePublic {
    match row {
        Some(r) if !r.sandbox_id.trim().is_empty() || !r.proxy_base_url.trim().is_empty() => {
            ProjectObservePublic {
                configured: true,
                sandbox_id: Some(r.sandbox_id.clone()).filter(|s| !s.is_empty()),
                proxy_base_url: Some(r.proxy_base_url.clone()).filter(|s| !s.is_empty()),
                live_base_url: Some(r.live_base_url.clone()).filter(|s| !s.is_empty()),
                host: Some(r.host.clone()).filter(|s| !s.is_empty()),
                proxy_port: Some(r.proxy_port),
                live_port: Some(r.live_port),
                updated_at_ms: Some(r.updated_at_ms).filter(|&ms| ms > 0),
                e2b_observe_sandbox_running: None,
            }
        }
        _ => ProjectObservePublic {
            configured: false,
            sandbox_id: None,
            proxy_base_url: None,
            live_base_url: None,
            host: None,
            proxy_port: None,
            live_port: None,
            updated_at_ms: None,
            e2b_observe_sandbox_running: None,
        },
    }
}

pub async fn load_project_inference_settings(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<ProjectInferenceSettingsResponse, String> {
    let (cluster_id, store) = load_store(db, proj_id).await?;
    let mode = project_llm_mode(&store);
    let observe_row = db
        .get_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ProjectInferenceSettingsResponse {
        proj_id,
        mode,
        cluster_id: Some(cluster_id.clone()),
        llm_models: llm_models_to_public(db, &cluster_id, proj_id, &store)
            .await
            .map_err(|e| e.to_string())?,
        active_llm_model_id: if store.active_id.is_empty() {
            None
        } else {
            Some(store.active_id.clone())
        },
        active_llm_model_rev: if store.active_rev.is_empty() {
            None
        } else {
            Some(store.active_rev.clone())
        },
        active_llm_applied_at_ms: store.active_applied_at_ms,
        active_llm_config: load_active_project_llm_config_public(db, proj_id)
            .await
            .map_err(|e| e.to_string())?,
        observe: observe_public_from_row(observe_row.as_ref()),
    })
}

pub async fn upsert_project_llm_model(
    db: &GatewaySessionDb,
    proj_id: i64,
    input: PutLlmModelInput,
) -> Result<LlmModelPublic, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let base = crate::gateway_llm_model_apply::normalize_upstream_base_url(&input.base_model_url)
        .ok_or_else(|| "invalid baseModelUrl".to_string())?;
    let model =
        crate::gateway_llm_model_apply::normalize_model_name_for_upstream(&input.model_name, &base)
            .ok_or_else(|| "invalid modelName".to_string())?;

    let (cluster_id, mut store) = load_store(db, proj_id).await?;
    let id = if let Some(raw) = input.id.as_deref() {
        normalize_llm_id(raw).ok_or_else(|| "invalid llm model id".to_string())?
    } else {
        allocate_llm_id(&store.models)
    };

    let is_new = !store.models.iter().any(|m| m.id == id);
    let prev_rev = store
        .models
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.current_rev.clone())
        .unwrap_or_default();
    let api_key =
        resolve_llm_api_key_on_save(&store, &id, &prev_rev, input.api_key.as_deref(), is_new)?;

    let now = now_ms();
    let rev = format_model_rev_local_ms(now);
    let row = GatewayLlmProjectRevisionRow {
        cluster_id: cluster_id.clone(),
        proj_id,
        model_id: id.clone(),
        model_rev: rev.clone(),
        created_at_ms: now,
        name: name.to_string(),
        base_model_url: base.clone(),
        model_name: model.clone(),
        supports_vision: input.supports_vision,
        note: normalize_revision_note(input.note),
    };
    db.upsert_llm_project_revision(&row)
        .await
        .map_err(|e| e.to_string())?;

    prune_llm_api_keys_for_model(&mut store, &id);
    store.api_keys.insert(llm_api_key_slot(&id, &rev), api_key);

    if let Some(idx) = store.models.iter().position(|m| m.id == id) {
        let entry = &mut store.models[idx];
        entry.name = name.to_string();
        entry.base_model_url = base.clone();
        entry.model_name = model.clone();
        entry.supports_vision = input.supports_vision;
        entry.current_rev = rev.clone();
        entry.updated_at_ms = now;
    } else {
        store.models.push(LlmModelEntry {
            id: id.clone(),
            name: name.to_string(),
            base_model_url: base,
            model_name: model,
            supports_vision: input.supports_vision,
            current_rev: rev.clone(),
            created_at_ms: now,
            updated_at_ms: now,
        });
    }

    if store.active_id.is_empty() || store.active_id == id {
        store.active_id = id.clone();
        store.active_rev = rev.clone();
        store.active_applied_at_ms = Some(now);
    }

    save_project_llm_store(db, &cluster_id, proj_id, &store, now)
        .await
        .map_err(|e| e.to_string())?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "llm model missing after save".to_string())?;
    llm_entry_to_public(db, &cluster_id, proj_id, entry, &store)
        .await
        .map_err(|e| e.to_string())
}

pub async fn delete_project_llm_model(
    db: &GatewaySessionDb,
    proj_id: i64,
    model_id: &str,
) -> Result<(bool, bool), String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let (cluster_id, mut store) = load_store(db, proj_id).await?;
    if !store.models.iter().any(|m| m.id == id) {
        return Ok((false, !project_has_active_llm(&store)));
    }
    db.delete_llm_project_revisions(&cluster_id, proj_id, &id)
        .await
        .map_err(|e| e.to_string())?;
    store.models.retain(|m| m.id != id);
    prune_llm_api_keys_for_model(&mut store, &id);
    if store.active_id == id {
        if let Some(next) = store.models.first() {
            store.active_id = next.id.clone();
            store.active_rev = next.current_rev.clone();
        } else {
            store.active_id.clear();
            store.active_rev.clear();
            store.active_applied_at_ms = None;
        }
    }
    save_project_llm_store(db, &cluster_id, proj_id, &store, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    let inherit_now = !project_has_active_llm(&store);
    Ok((true, inherit_now))
}

pub async fn apply_project_llm_model_by_id(
    db: &GatewaySessionDb,
    proj_id: i64,
    model_id: &str,
    _model_rev: Option<&str>,
) -> Result<ApplyLlmModelResponse, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let (cluster_id, mut store) = load_store(db, proj_id).await?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("llm model {id} not found"))?
        .clone();
    let rev = if entry.current_rev.is_empty() {
        format_model_rev_local_ms(entry.updated_at_ms)
    } else {
        entry.current_rev.clone()
    };
    if llm_api_key_for(&store, &id, &rev).is_none() {
        return Err("apiKey is not configured".into());
    }
    let applied_at_ms = now_ms();
    store.active_id = id.clone();
    store.active_rev = rev.clone();
    store.active_applied_at_ms = Some(applied_at_ms);
    save_project_llm_store(db, &cluster_id, proj_id, &store, applied_at_ms)
        .await
        .map_err(|e| e.to_string())?;
    let public = llm_entry_to_public(db, &cluster_id, proj_id, &entry, &store)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ApplyLlmModelResponse {
        active_llm_model_id: id,
        active_llm_model_rev: rev,
        active_llm_applied_at_ms: applied_at_ms,
        llm_model: public,
        outcome: LlmModelApplyOutcome {
            env_file: String::new(),
            applied_at_ms,
            tap_chain_refreshed: false,
            tap_restarted: false,
            message: Some(
                "project active LLM updated; project observe polls PG for upstream".into(),
            ),
        },
    })
}

pub async fn list_project_llm_model_versions(
    db: &GatewaySessionDb,
    proj_id: i64,
    model_id: &str,
) -> Result<LlmModelVersionsResponse, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let (_cluster_id, store) = load_store(db, proj_id).await?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("llm model {id} not found"))?;
    Ok(LlmModelVersionsResponse {
        model_id: id,
        current_rev: entry.current_rev.clone(),
        active_rev: if store.active_rev.is_empty() {
            None
        } else {
            Some(store.active_rev.clone())
        },
        versions: vec![],
    })
}

/// Load project observe proxy URL when override is active. Author: kejiqing
pub async fn load_project_observe_proxy_base_url(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<Option<String>, String> {
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return Ok(None);
    };
    let row = db
        .get_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| {
        let u = r.proxy_base_url.trim().trim_end_matches('/').to_string();
        if u.is_empty() {
            None
        } else {
            Some(u)
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherit_when_no_active() {
        let store = LlmModelsStore::default();
        assert_eq!(project_llm_mode(&store), ProjectLlmMode::Inherit);
        assert!(!project_has_active_llm(&store));
    }

    #[test]
    fn override_when_active_set() {
        let store = LlmModelsStore {
            active_id: "llm-1".into(),
            active_rev: "2026-07-28_12-00-00".into(),
            ..Default::default()
        };
        assert_eq!(project_llm_mode(&store), ProjectLlmMode::Override);
        assert!(project_has_active_llm(&store));
    }

    #[test]
    fn prune_removes_at_slots() {
        let mut store = LlmModelsStore::default();
        store.api_keys.insert("llm-1@old".into(), "sk-old".into());
        store.api_keys.insert("llm-1@new".into(), "sk-new".into());
        store.api_keys.insert("llm-2@x".into(), "sk-2".into());
        prune_llm_api_keys_for_model(&mut store, "llm-1");
        assert!(!store.api_keys.contains_key("llm-1@old"));
        assert!(!store.api_keys.contains_key("llm-1@new"));
        assert!(store.api_keys.contains_key("llm-2@x"));
    }
}
