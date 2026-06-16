//! Gateway-wide settings (not per `proj_id`): PAT vault for Git push, LLM model, etc. Author: kejiqing
//!
//! **全局大模型（按 `CLAW_CLUSTER_ID` 隔离，独立 PG 表 + 密钥加密）**
//! 1. Admin 保存 → `gateway_llm_cluster_model`（API Key 以 clusterId 派生 AES 密钥加密）。
//! 2. `gateway_llm_cluster_state` 指向当前生效条目 → handler 调 `sync_llm_runtime_from_db`。
//! 3. claude-tap 从 PG 轮询 upstream；gateway 求解时经 pool Exec 注入 worker LLM env（不写 `.claw/*` 文件）。

use runtime::builtin_system_prompt_scaffold_default;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::cluster_identity::gateway_cluster_id_optional;
use crate::gateway_admin_mcp_token::{admin_mcp_tokens_public, AdminMcpTokenPublic};
use crate::gateway_claw_tap_settings::{ClawTapSettings, ClawTapSettingsPublic};
use crate::gateway_llm_cluster_store::{self, resolve_llm_cluster_id};
use crate::gateway_llm_model_apply::{self, LlmModelApplyOutcome};
use crate::gateway_llm_model_revision::{
    format_model_rev_local_ms, llm_api_key_slot, GLOBAL_LLM_MODEL_ID,
};
use crate::project_config_draft::normalize_revision_note;
use crate::session_db::{GatewayLlmModelRevisionRow, GatewaySessionDb};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPatEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmModelEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(rename = "currentRev", default, skip_serializing)]
    pub current_rev: String,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmModelPublic {
    pub id: String,
    pub name: String,
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(rename = "currentRev", skip_serializing)]
    pub current_rev: String,
    #[serde(rename = "apiKeySet")]
    pub api_key_set: bool,
    #[serde(rename = "active", default)]
    pub active: bool,
    #[serde(rename = "activeRev", skip_serializing)]
    pub active_rev: Option<String>,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LlmModelVersionPublic {
    #[serde(rename = "modelRev")]
    pub model_rev: String,
    pub name: String,
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(rename = "apiKeySet")]
    pub api_key_set: bool,
    pub active: bool,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LlmModelVersionsResponse {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "currentRev")]
    pub current_rev: String,
    #[serde(rename = "activeRev", skip_serializing_if = "Option::is_none")]
    pub active_rev: Option<String>,
    pub versions: Vec<LlmModelVersionPublic>,
}

/// Active LLM revision loaded from PostgreSQL (source of truth). Author: kejiqing
#[derive(Debug, Clone)]
pub struct ActiveLlmRuntime {
    pub model_id: String,
    pub model_rev: String,
    pub base_model_url: String,
    pub model_name: String,
    pub api_key: String,
    pub applied_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct LlmModelsStore {
    pub models: Vec<LlmModelEntry>,
    pub api_keys: BTreeMap<String, String>,
    pub active_id: String,
    pub active_rev: String,
    pub active_applied_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayGlobalSettingsPublic {
    #[serde(rename = "gitPats", default)]
    pub git_pats: Vec<GitPatPublic>,
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
    /// 当前生效 revision 的配置（全局变量视图，只读 DB active + revision）。Author: kejiqing
    #[serde(
        rename = "activeLlmConfig",
        default,
        skip_deserializing,
        skip_serializing_if = "Option::is_none"
    )]
    pub active_llm_config: Option<ActiveLlmConfigPublic>,
    #[serde(
        rename = "clawTap",
        default,
        skip_deserializing,
        skip_serializing_if = "Option::is_none"
    )]
    pub claw_tap: Option<ClawTapSettingsPublic>,
    #[serde(
        rename = "adminMcpTokens",
        default,
        skip_deserializing,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub admin_mcp_tokens: Vec<AdminMcpTokenPublic>,
    #[serde(
        rename = "clusterId",
        default,
        skip_deserializing,
        skip_serializing_if = "Option::is_none"
    )]
    pub cluster_id: Option<String>,
}

/// 当前全局生效的 LLM（`active_llm_model_*` + revision 行）。Author: kejiqing
#[derive(Debug, Clone, Serialize)]
pub struct ActiveLlmConfigPublic {
    #[serde(rename = "modelId")]
    pub model_id: String,
    pub name: String,
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(rename = "apiKeySet")]
    pub api_key_set: bool,
}

#[derive(Debug, Deserialize)]
pub struct PutActiveLlmConfigInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPatPublic {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "tokenSet")]
    pub token_set: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayGlobalSettingsStore {
    #[serde(rename = "gitPats", default)]
    git_pats: Vec<GitPatEntry>,
    #[serde(rename = "adminMcpTokens", default)]
    pub(crate) admin_mcp_tokens: Vec<crate::gateway_admin_mcp_token::AdminMcpTokenEntry>,
    #[serde(rename = "clusterId", default)]
    pub(crate) cluster_id: String,
    #[serde(rename = "clawTap", default)]
    pub(crate) claw_tap: ClawTapSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitPatTokensStore {
    #[serde(default)]
    pub tokens: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct PutGitPatInput {
    /// Omit to create; must be unique when provided.
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub note: Option<String>,
    /// Omit on update to keep existing token.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PutLlmModelInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "baseModelUrl")]
    pub base_model_url: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GatewayGlobalSettingsResponse {
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "gitPats")]
    pub git_pats: Vec<GitPatPublic>,
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
    #[serde(rename = "clawTap", skip_serializing_if = "Option::is_none")]
    pub claw_tap: Option<ClawTapSettingsPublic>,
    #[serde(rename = "adminMcpTokens", default)]
    pub admin_mcp_tokens: Vec<AdminMcpTokenPublic>,
    #[serde(rename = "clusterId", skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
}

fn cluster_id_public() -> Option<String> {
    gateway_cluster_id_optional()
}

#[derive(Debug, Serialize)]
pub struct ApplyLlmModelResponse {
    #[serde(rename = "llmModel")]
    pub llm_model: LlmModelPublic,
    #[serde(rename = "activeLlmModelId")]
    pub active_llm_model_id: String,
    #[serde(rename = "activeLlmModelRev")]
    pub active_llm_model_rev: String,
    #[serde(rename = "activeLlmAppliedAtMs")]
    pub active_llm_applied_at_ms: i64,
    #[serde(flatten)]
    pub outcome: LlmModelApplyOutcome,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn normalize_pat_id(raw: &str) -> Option<String> {
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

fn allocate_pat_id(existing: &[GitPatEntry]) -> String {
    let base = format!("pat-{}", now_ms());
    if existing.iter().any(|p| p.id == base) {
        format!("{base}-2")
    } else {
        base
    }
}

fn allocate_llm_id(existing: &[LlmModelEntry]) -> String {
    let base = format!("llm-{}", now_ms());
    if existing.iter().any(|m| m.id == base) {
        format!("{base}-2")
    } else {
        base
    }
}

fn normalize_llm_id(raw: &str) -> Option<String> {
    normalize_pat_id(raw)
}

fn prune_llm_api_keys_for_model(store: &mut LlmModelsStore, model_id: &str) {
    let prefix = format!("{model_id}:");
    store
        .api_keys
        .retain(|k, _| k != model_id && !k.starts_with(&prefix));
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

async fn ensure_llm_model_versions_backfilled(
    db: &GatewaySessionDb,
    cluster_id: &str,
    store: &mut LlmModelsStore,
) -> Result<bool, sqlx::Error> {
    let mut changed = false;
    for entry in store.models.clone() {
        if !entry.current_rev.is_empty() {
            continue;
        }
        let rev = format_model_rev_local_ms(entry.updated_at_ms);
        let row = GatewayLlmModelRevisionRow {
            cluster_id: cluster_id.to_string(),
            model_id: entry.id.clone(),
            model_rev: rev.clone(),
            created_at_ms: entry.updated_at_ms,
            name: entry.name.clone(),
            base_model_url: entry.base_model_url.clone(),
            model_name: entry.model_name.clone(),
            note: None,
        };
        db.upsert_llm_cluster_revision(&row).await?;
        if let Some(k) = store.api_keys.remove(&entry.id) {
            store.api_keys.insert(llm_api_key_slot(&entry.id, &rev), k);
        }
        if let Some(idx) = store.models.iter().position(|m| m.id == entry.id) {
            store.models[idx].current_rev = rev.clone();
            if store.active_id == entry.id && store.active_rev.is_empty() {
                store.active_rev = rev;
            }
        }
        changed = true;
    }
    Ok(changed)
}

pub async fn load_active_llm_config_public(
    db: &GatewaySessionDb,
) -> Result<Option<ActiveLlmConfigPublic>, sqlx::Error> {
    let store = load_llm_models_store(db).await?;
    let Some(runtime) = load_active_llm_runtime(db).await? else {
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

/// Upsert one LLM model by `id` (create when `id` omitted). Does not change active unless none set. Author: kejiqing
pub async fn upsert_llm_model(
    db: &GatewaySessionDb,
    input: PutLlmModelInput,
) -> Result<LlmModelPublic, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let base = gateway_llm_model_apply::normalize_upstream_base_url(&input.base_model_url)
        .ok_or_else(|| "invalid baseModelUrl".to_string())?;
    let model =
        gateway_llm_model_apply::normalize_model_name_for_upstream(&input.model_name, &base)
            .ok_or_else(|| "invalid modelName".to_string())?;

    let cluster_id =
        resolve_llm_cluster_id().ok_or_else(|| "CLAW_CLUSTER_ID is not set".to_string())?;
    let mut store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
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
    let row = GatewayLlmModelRevisionRow {
        cluster_id: cluster_id.clone(),
        model_id: id.clone(),
        model_rev: rev.clone(),
        created_at_ms: now,
        name: name.to_string(),
        base_model_url: base.clone(),
        model_name: model.clone(),
        note: normalize_revision_note(input.note),
    };
    db.upsert_llm_cluster_revision(&row)
        .await
        .map_err(|e| e.to_string())?;

    prune_llm_api_keys_for_model(&mut store, &id);
    store.api_keys.insert(llm_api_key_slot(&id, &rev), api_key);

    if let Some(idx) = store.models.iter().position(|m| m.id == id) {
        let entry = &mut store.models[idx];
        entry.name = name.to_string();
        entry.base_model_url = base.clone();
        entry.model_name = model.clone();
        entry.current_rev = rev.clone();
        entry.updated_at_ms = now;
    } else {
        store.models.push(LlmModelEntry {
            id: id.clone(),
            name: name.to_string(),
            base_model_url: base,
            model_name: model,
            current_rev: rev,
            created_at_ms: now,
            updated_at_ms: now,
        });
    }

    if store.active_id.is_empty() {
        store.active_id = id.clone();
        store.active_rev = store
            .models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.current_rev.clone())
            .unwrap_or_default();
        store.active_applied_at_ms = Some(now);
    }

    save_llm_models_store(db, &store, now)
        .await
        .map_err(|e| e.to_string())?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "llm model missing after save".to_string())?;
    llm_entry_to_public(db, entry, &store)
        .await
        .map_err(|e| e.to_string())
}

/// 兼容旧客户端：更新当前/首条模型并设为 active。Author: kejiqing
pub async fn put_active_llm_config(
    db: &GatewaySessionDb,
    input: PutActiveLlmConfigInput,
) -> Result<ActiveLlmConfigPublic, String> {
    let store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    let target_id = if !store.active_id.is_empty() {
        store.active_id.clone()
    } else if store.models.iter().any(|m| m.id == GLOBAL_LLM_MODEL_ID) {
        GLOBAL_LLM_MODEL_ID.to_string()
    } else {
        store
            .models
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_default()
    };
    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("默认");
    let saved = upsert_llm_model(
        db,
        PutLlmModelInput {
            id: if target_id.is_empty() {
                None
            } else {
                Some(target_id)
            },
            name: name.to_string(),
            base_model_url: input.base_model_url,
            model_name: input.model_name,
            api_key: input.api_key,
            note: input.note,
        },
    )
    .await?;
    apply_llm_model_by_id(db, &saved.id, None).await?;
    load_active_llm_config_public(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "active LLM missing after save".into())
}

/// Load one model's current revision + API key for admin probe (does not require active). Author: kejiqing
pub async fn load_llm_runtime_for_model_id(
    db: &GatewaySessionDb,
    model_id: &str,
) -> Result<ActiveLlmRuntime, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let cluster_id =
        resolve_llm_cluster_id().ok_or_else(|| "CLAW_CLUSTER_ID is not set".to_string())?;
    let store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
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
        .get_llm_cluster_revision(&cluster_id, &id, &rev)
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
        applied_at_ms: None,
    })
}

pub(crate) async fn load_active_llm_runtime(
    db: &GatewaySessionDb,
) -> Result<Option<ActiveLlmRuntime>, sqlx::Error> {
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return Ok(None);
    };
    let store = load_llm_models_store(db).await?;
    if store.active_id.is_empty() || store.active_rev.is_empty() {
        return Ok(None);
    }
    let Some(row) = db
        .get_llm_cluster_revision(&cluster_id, &store.active_id, &store.active_rev)
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
        applied_at_ms: store.active_applied_at_ms,
    }))
}

async fn load_llm_models_store(db: &GatewaySessionDb) -> Result<LlmModelsStore, sqlx::Error> {
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return Ok(LlmModelsStore::default());
    };
    let mut store = gateway_llm_cluster_store::load_cluster_llm_store(db, &cluster_id).await?;
    let changed = ensure_llm_model_versions_backfilled(db, &cluster_id, &mut store).await?;
    if changed {
        save_llm_models_store(db, &store, now_ms()).await?;
    }
    Ok(store)
}

async fn save_llm_models_store(
    db: &GatewaySessionDb,
    store: &LlmModelsStore,
    updated_at_ms: i64,
) -> Result<(), sqlx::Error> {
    let cluster_id = resolve_llm_cluster_id()
        .ok_or_else(|| sqlx::Error::Configuration("CLAW_CLUSTER_ID is not set".into()))?;
    gateway_llm_cluster_store::save_cluster_llm_store(db, &cluster_id, store, updated_at_ms).await
}

async fn llm_entry_to_public(
    db: &GatewaySessionDb,
    entry: &LlmModelEntry,
    store: &LlmModelsStore,
) -> Result<LlmModelPublic, sqlx::Error> {
    let current_rev = if entry.current_rev.is_empty() {
        format_model_rev_local_ms(entry.updated_at_ms)
    } else {
        entry.current_rev.clone()
    };
    let (name, base_model_url, model_name) = if let Some(cluster_id) = resolve_llm_cluster_id() {
        match db
            .get_llm_cluster_revision(&cluster_id, &entry.id, &current_rev)
            .await?
        {
            Some(row) => (row.name, row.base_model_url, row.model_name),
            None => (
                entry.name.clone(),
                entry.base_model_url.clone(),
                entry.model_name.clone(),
            ),
        }
    } else {
        (
            entry.name.clone(),
            entry.base_model_url.clone(),
            entry.model_name.clone(),
        )
    };
    let is_active_model = !store.active_id.is_empty() && store.active_id == entry.id;
    let api_key_set = llm_api_key_for(store, &entry.id, &current_rev).is_some();
    Ok(LlmModelPublic {
        id: entry.id.clone(),
        name,
        base_model_url,
        model_name,
        current_rev,
        api_key_set,
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
    store: &LlmModelsStore,
) -> Result<Vec<LlmModelPublic>, sqlx::Error> {
    let mut out = Vec::with_capacity(store.models.len());
    for entry in &store.models {
        out.push(llm_entry_to_public(db, entry, store).await?);
    }
    Ok(out)
}

fn active_llm_model_id_public(store: &LlmModelsStore) -> Option<String> {
    if store.active_id.is_empty() {
        None
    } else {
        Some(store.active_id.clone())
    }
}

fn active_llm_model_rev_public(store: &LlmModelsStore) -> Option<String> {
    if store.active_rev.is_empty() {
        None
    } else {
        Some(store.active_rev.clone())
    }
}

fn parse_settings_store(v: &serde_json::Value) -> GatewayGlobalSettingsStore {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

fn parse_tokens_store(v: &serde_json::Value) -> GitPatTokensStore {
    if v.is_object() && v.get("tokens").is_none() {
        GitPatTokensStore {
            tokens: serde_json::from_value(v.clone()).unwrap_or_default(),
        }
    } else {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

fn tokens_to_json(store: &GitPatTokensStore) -> serde_json::Value {
    serde_json::to_value(&store.tokens).unwrap_or_else(|_| serde_json::json!({}))
}

pub async fn get_gateway_global_settings(
    db: &GatewaySessionDb,
) -> Result<(GatewayGlobalSettingsStore, GitPatTokensStore, i64), sqlx::Error> {
    let (settings_v, tokens_v, updated_at_ms) = db.get_gateway_global_settings_raw().await?;
    Ok((
        parse_settings_store(&settings_v),
        parse_tokens_store(&tokens_v),
        updated_at_ms,
    ))
}

pub async fn save_gateway_global_settings(
    db: &GatewaySessionDb,
    settings: &GatewayGlobalSettingsStore,
    tokens: &GitPatTokensStore,
    updated_at_ms: i64,
) -> Result<(), sqlx::Error> {
    let mut settings = settings.clone();
    settings.cluster_id.clear();
    let settings_v =
        serde_json::to_value(&settings).unwrap_or_else(|_| serde_json::json!({"gitPats":[]}));
    db.save_gateway_global_settings_raw(&settings_v, &tokens_to_json(tokens), updated_at_ms)
        .await
}

pub async fn load_public(
    db: &GatewaySessionDb,
) -> Result<GatewayGlobalSettingsPublic, sqlx::Error> {
    let (settings, tokens, _) = get_gateway_global_settings(db).await?;
    let llm = load_llm_models_store(db).await?;
    Ok(GatewayGlobalSettingsPublic {
        git_pats: to_public(&settings, &tokens).git_pats,
        llm_models: llm_models_to_public(db, &llm).await?,
        active_llm_model_id: active_llm_model_id_public(&llm),
        active_llm_model_rev: active_llm_model_rev_public(&llm),
        active_llm_applied_at_ms: llm.active_applied_at_ms,
        active_llm_config: load_active_llm_config_public(db).await?,
        claw_tap: Some(ClawTapSettingsPublic::from(&settings.claw_tap)),
        admin_mcp_tokens: admin_mcp_tokens_public(&settings),
        cluster_id: cluster_id_public(),
    })
}

pub async fn load_response(
    db: &GatewaySessionDb,
) -> Result<GatewayGlobalSettingsResponse, sqlx::Error> {
    let (settings, tokens, updated_at_ms) = get_gateway_global_settings(db).await?;
    let llm = load_llm_models_store(db).await?;
    Ok(GatewayGlobalSettingsResponse {
        updated_at_ms,
        git_pats: to_public(&settings, &tokens).git_pats,
        llm_models: llm_models_to_public(db, &llm).await?,
        active_llm_model_id: active_llm_model_id_public(&llm),
        active_llm_model_rev: active_llm_model_rev_public(&llm),
        active_llm_applied_at_ms: llm.active_applied_at_ms,
        active_llm_config: load_active_llm_config_public(db).await?,
        claw_tap: Some(ClawTapSettingsPublic::from(&settings.claw_tap)),
        admin_mcp_tokens: admin_mcp_tokens_public(&settings),
        cluster_id: cluster_id_public(),
    })
}

pub async fn list_llm_model_versions(
    db: &GatewaySessionDb,
    model_id: &str,
) -> Result<LlmModelVersionsResponse, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("llm model {id} not found"))?;
    Ok(LlmModelVersionsResponse {
        model_id: id,
        current_rev: entry.current_rev.clone(),
        active_rev: active_llm_model_rev_public(&store),
        versions: vec![],
    })
}

pub async fn delete_llm_model(db: &GatewaySessionDb, model_id: &str) -> Result<bool, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let cluster_id =
        resolve_llm_cluster_id().ok_or_else(|| "CLAW_CLUSTER_ID is not set".to_string())?;
    let mut store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    if !store.models.iter().any(|m| m.id == id) {
        return Ok(false);
    }
    db.delete_llm_cluster_revisions(&cluster_id, &id)
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
    save_llm_models_store(db, &store, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 将指定模型设为当前生效并触发 runtime 同步（由 handler 调 sync）。Author: kejiqing
pub async fn apply_llm_model_by_id(
    db: &GatewaySessionDb,
    model_id: &str,
    _model_rev: Option<&str>,
) -> Result<ApplyLlmModelResponse, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    let mut store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
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
    save_llm_models_store(db, &store, applied_at_ms)
        .await
        .map_err(|e| e.to_string())?;
    let public = llm_entry_to_public(db, &entry, &store)
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
            message: Some("active LLM updated; claude-tap polls PG for upstream".into()),
        },
    })
}

pub async fn validate_git_sync_json_with_global(
    db: &GatewaySessionDb,
    v: &serde_json::Value,
) -> Result<(), String> {
    let sync = crate::project_git_sync::parse_git_sync_json(v);
    let tokens = load_git_pat_tokens(db).await.map_err(|e| e.to_string())?;
    let resolved = crate::project_git_sync::resolve_git_sync_credentials(&sync, &tokens.tokens);
    crate::project_git_sync::validate_git_sync_resolved(&resolved)
}

pub async fn upsert_git_pat(
    db: &GatewaySessionDb,
    input: PutGitPatInput,
) -> Result<GitPatPublic, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let note = input
        .note
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut settings, mut tokens, _updated_at_ms) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let id = if let Some(raw) = input.id.as_deref() {
        normalize_pat_id(raw).ok_or_else(|| "invalid pat id".to_string())?
    } else {
        allocate_pat_id(&settings.git_pats)
    };
    let now = now_ms();
    if let Some(idx) = settings.git_pats.iter().position(|p| p.id == id) {
        let entry = &mut settings.git_pats[idx];
        entry.name = name.to_string();
        entry.note = note;
        entry.updated_at_ms = now;
        if let Some(tok) = input
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            tokens.tokens.insert(id.clone(), tok.to_string());
        }
    } else {
        let tok = input
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "token is required for new PAT".to_string())?;
        settings.git_pats.push(GitPatEntry {
            id: id.clone(),
            name: name.to_string(),
            note,
            created_at_ms: now,
            updated_at_ms: now,
        });
        tokens.tokens.insert(id.clone(), tok.to_string());
    }
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    let public = to_public(&settings, &tokens);
    public
        .git_pats
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "pat missing after save".to_string())
}

pub async fn delete_git_pat(db: &GatewaySessionDb, pat_id: &str) -> Result<bool, String> {
    let id = normalize_pat_id(pat_id).ok_or_else(|| "invalid pat id".to_string())?;
    let (mut settings, mut tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let before = settings.git_pats.len();
    settings.git_pats.retain(|p| p.id != id);
    tokens.tokens.remove(&id);
    if settings.git_pats.len() == before {
        return Ok(false);
    }
    let now = now_ms();
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[must_use]
pub fn resolve_git_pat_token(pat_id: Option<&str>, tokens: &GitPatTokensStore) -> Option<String> {
    let id = pat_id?.trim();
    if id.is_empty() {
        return None;
    }
    tokens.tokens.get(id).cloned()
}

pub async fn load_git_pat_tokens(db: &GatewaySessionDb) -> Result<GitPatTokensStore, sqlx::Error> {
    let (_, tokens, _) = get_gateway_global_settings(db).await?;
    Ok(tokens)
}

/// Builtin system prompt scaffold from PG (not exposed on Admin API). Author: kejiqing
pub async fn load_system_prompt_default(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let (text, _) = db.get_gateway_system_prompt_default().await?;
    Ok(if text.trim().is_empty() {
        builtin_system_prompt_scaffold_default()
    } else {
        text
    })
}

#[must_use]
pub fn to_public(
    settings: &GatewayGlobalSettingsStore,
    tokens: &GitPatTokensStore,
) -> GatewayGlobalSettingsPublic {
    GatewayGlobalSettingsPublic {
        git_pats: settings
            .git_pats
            .iter()
            .map(|p| GitPatPublic {
                id: p.id.clone(),
                name: p.name.clone(),
                note: p.note.clone(),
                created_at_ms: p.created_at_ms,
                updated_at_ms: p.updated_at_ms,
                token_set: tokens.tokens.contains_key(&p.id),
            })
            .collect(),
        llm_models: vec![],
        active_llm_model_id: None,
        active_llm_model_rev: None,
        active_llm_applied_at_ms: None,
        active_llm_config: None,
        claw_tap: Some(ClawTapSettingsPublic::from(&settings.claw_tap)),
        admin_mcp_tokens: admin_mcp_tokens_public(settings),
        cluster_id: cluster_id_public(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pat_id_accepts_slug() {
        assert_eq!(
            normalize_pat_id("pat-github-1").as_deref(),
            Some("pat-github-1")
        );
    }

    #[test]
    fn normalize_llm_id_accepts_slug() {
        assert_eq!(
            normalize_llm_id("llm-deepseek-1").as_deref(),
            Some("llm-deepseek-1")
        );
    }

    #[test]
    fn put_llm_model_input_deserializes_camel_case_api_key() {
        let input: PutLlmModelInput = serde_json::from_value(serde_json::json!({
            "name": "DeepSeek",
            "baseModelUrl": "https://api.deepseek.com",
            "modelName": "deepseek-chat",
            "apiKey": "sk-test"
        }))
        .expect("deserialize");
        assert_eq!(input.api_key.as_deref(), Some("sk-test"));
    }
}
