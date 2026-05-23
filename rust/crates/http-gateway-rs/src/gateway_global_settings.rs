//! Gateway-wide settings (not per `ds_id`): PAT vault for Git push, LLM model, etc. Author: kejiqing
//!
//! **全局大模型（无版本）**
//! 1. Admin 保存 → 单行 `global` / `global` upsert 到 `gateway_llm_model_revision`。
//! 2. 同时写 `active_llm_model_id/rev`（恒为 `global`）→ handler 调 `sync_llm_runtime_from_db`。
//! 3. 落盘：`.env` + `.claw/claw-tap-upstream.json`（worker / claude-tap 读文件，非 Admin 缓存）。

use runtime::builtin_system_prompt_scaffold_default;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::gateway_llm_model_apply::{self, LlmModelApplyOutcome};
use crate::gateway_llm_model_revision::{
    format_model_rev_local_ms, llm_api_key_slot, GLOBAL_LLM_MODEL_ID, GLOBAL_LLM_REV,
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
    #[serde(skip_serializing)]
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
pub(crate) struct ActiveLlmRuntime {
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
}

/// 当前全局生效的 LLM（`active_llm_model_*` + revision 行）。Author: kejiqing
#[derive(Debug, Clone, Serialize)]
pub struct ActiveLlmConfigPublic {
    #[serde(rename = "modelId", skip_serializing)]
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

fn normalize_llm_id(raw: &str) -> Option<String> {
    normalize_pat_id(raw)
}

fn parse_llm_models_store(
    models_v: &serde_json::Value,
    keys_v: &serde_json::Value,
    active_id: &str,
    active_rev: &str,
    applied_at_ms: Option<i64>,
) -> LlmModelsStore {
    let models: Vec<LlmModelEntry> = serde_json::from_value(models_v.clone()).unwrap_or_default();
    let api_keys: BTreeMap<String, String> =
        serde_json::from_value(keys_v.clone()).unwrap_or_default();
    LlmModelsStore {
        models,
        api_keys,
        active_id: active_id.trim().to_string(),
        active_rev: active_rev.trim().to_string(),
        active_applied_at_ms: applied_at_ms,
    }
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
    store: &mut LlmModelsStore,
) -> Result<bool, sqlx::Error> {
    let mut changed = false;
    for entry in store.models.clone() {
        if !entry.current_rev.is_empty() {
            continue;
        }
        let rev = format_model_rev_local_ms(entry.updated_at_ms);
        let row = GatewayLlmModelRevisionRow {
            model_id: entry.id.clone(),
            model_rev: rev.clone(),
            created_at_ms: entry.updated_at_ms,
            name: entry.name.clone(),
            base_model_url: entry.base_model_url.clone(),
            model_name: entry.model_name.clone(),
            note: None,
        };
        db.insert_llm_model_revision(&row).await?;
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
        model_id: GLOBAL_LLM_MODEL_ID.to_string(),
        name,
        base_model_url: runtime.base_model_url,
        model_name: runtime.model_name,
        api_key_set: !runtime.api_key.is_empty(),
    }))
}

fn resolve_global_api_key(
    store: &LlmModelsStore,
    input_key: Option<&str>,
) -> Result<String, String> {
    if let Some(key) = input_key.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(key.to_string());
    }
    llm_api_key_for(store, GLOBAL_LLM_MODEL_ID, GLOBAL_LLM_REV)
        .or_else(|| store.api_keys.get(GLOBAL_LLM_MODEL_ID).cloned())
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| "apiKey is required".into())
}

/// 保存全局大模型（无版本）：upsert `global` 行 + 设为 active。Author: kejiqing
pub async fn save_global_llm(
    db: &GatewaySessionDb,
    name: &str,
    base_model_url: &str,
    model_name: &str,
    api_key: Option<String>,
    note: Option<String>,
) -> Result<LlmModelPublic, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let base = gateway_llm_model_apply::normalize_upstream_base_url(base_model_url)
        .ok_or_else(|| "invalid baseModelUrl".to_string())?;
    let model = gateway_llm_model_apply::normalize_model_name_for_upstream(model_name, &base)
        .ok_or_else(|| "invalid modelName".to_string())?;
    let mut store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    let api_key = resolve_global_api_key(&store, api_key.as_deref())?;
    let now = now_ms();
    let row = GatewayLlmModelRevisionRow {
        model_id: GLOBAL_LLM_MODEL_ID.to_string(),
        model_rev: GLOBAL_LLM_REV.to_string(),
        created_at_ms: now,
        name: name.to_string(),
        base_model_url: base.clone(),
        model_name: model.clone(),
        note: normalize_revision_note(note),
    };
    db.upsert_llm_model_revision(&row)
        .await
        .map_err(|e| e.to_string())?;
    store.models = vec![LlmModelEntry {
        id: GLOBAL_LLM_MODEL_ID.to_string(),
        name: name.to_string(),
        base_model_url: base,
        model_name: model,
        current_rev: GLOBAL_LLM_REV.to_string(),
        created_at_ms: store.models.first().map(|m| m.created_at_ms).unwrap_or(now),
        updated_at_ms: now,
    }];
    store.api_keys.clear();
    store.api_keys.insert(
        llm_api_key_slot(GLOBAL_LLM_MODEL_ID, GLOBAL_LLM_REV),
        api_key,
    );
    store.active_id = GLOBAL_LLM_MODEL_ID.to_string();
    store.active_rev = GLOBAL_LLM_REV.to_string();
    store.active_applied_at_ms = Some(now);
    save_llm_models_store(db, &store, now)
        .await
        .map_err(|e| e.to_string())?;
    let entry = store
        .models
        .first()
        .ok_or_else(|| "global llm missing after save".to_string())?;
    llm_entry_to_public(db, entry, &store)
        .await
        .map_err(|e| e.to_string())
}

/// 保存全局大模型；handler 负责 `sync_llm_runtime_from_db`。Author: kejiqing
pub async fn put_active_llm_config(
    db: &GatewaySessionDb,
    input: PutActiveLlmConfigInput,
) -> Result<ActiveLlmConfigPublic, String> {
    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("默认");
    save_global_llm(
        db,
        name,
        &input.base_model_url,
        &input.model_name,
        input.api_key,
        input.note,
    )
    .await?;
    load_active_llm_config_public(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "global LLM missing after save".into())
}

pub(crate) async fn load_active_llm_runtime(
    db: &GatewaySessionDb,
) -> Result<Option<ActiveLlmRuntime>, sqlx::Error> {
    let store = load_llm_models_store(db).await?;
    if store.active_id.is_empty() || store.active_rev.is_empty() {
        return Ok(None);
    }
    let Some(row) = db
        .get_llm_model_revision(&store.active_id, &store.active_rev)
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

async fn ensure_global_llm_singleton(
    db: &GatewaySessionDb,
    store: &mut LlmModelsStore,
) -> Result<bool, sqlx::Error> {
    let normalized = store.models.len() == 1
        && store
            .models
            .first()
            .is_some_and(|m| m.id == GLOBAL_LLM_MODEL_ID)
        && store.active_id == GLOBAL_LLM_MODEL_ID
        && store.active_rev == GLOBAL_LLM_REV;
    if normalized {
        if db
            .get_llm_model_revision(GLOBAL_LLM_MODEL_ID, GLOBAL_LLM_REV)
            .await?
            .is_none()
        {
            if let Some(m) = store.models.first() {
                let row = GatewayLlmModelRevisionRow {
                    model_id: GLOBAL_LLM_MODEL_ID.to_string(),
                    model_rev: GLOBAL_LLM_REV.to_string(),
                    created_at_ms: m.updated_at_ms,
                    name: m.name.clone(),
                    base_model_url: m.base_model_url.clone(),
                    model_name: m.model_name.clone(),
                    note: None,
                };
                db.upsert_llm_model_revision(&row).await?;
            }
        }
        return Ok(false);
    }

    let (name, base_model_url, model_name, created_at_ms, api_key) =
        if !store.active_id.is_empty() && !store.active_rev.is_empty() {
            let mut name = "默认".to_string();
            let mut base_model_url = String::new();
            let mut model_name = String::new();
            let mut created_at_ms = now_ms();
            if let Some(row) = db
                .get_llm_model_revision(&store.active_id, &store.active_rev)
                .await?
            {
                name = row.name;
                base_model_url = row.base_model_url;
                model_name = row.model_name;
                created_at_ms = row.created_at_ms;
            }
            let api_key = llm_api_key_for(store, &store.active_id, &store.active_rev);
            (name, base_model_url, model_name, created_at_ms, api_key)
        } else if let Some(m) = store.models.first() {
            let mut name = m.name.clone();
            let mut base_model_url = m.base_model_url.clone();
            let mut model_name = m.model_name.clone();
            let mut created_at_ms = m.created_at_ms;
            let rev = if m.current_rev.is_empty() {
                GLOBAL_LLM_REV.to_string()
            } else {
                m.current_rev.clone()
            };
            let api_key = llm_api_key_for(store, &m.id, &rev);
            if let Some(row) = db.get_llm_model_revision(&m.id, &rev).await? {
                name = row.name;
                base_model_url = row.base_model_url;
                model_name = row.model_name;
                created_at_ms = row.created_at_ms;
            }
            (name, base_model_url, model_name, created_at_ms, api_key)
        } else {
            return Ok(false);
        };

    for old in &store.models {
        if old.id != GLOBAL_LLM_MODEL_ID {
            db.delete_llm_model_revisions(&old.id).await?;
        }
    }
    db.delete_llm_model_revisions(GLOBAL_LLM_MODEL_ID).await?;

    let now = now_ms();
    let row = GatewayLlmModelRevisionRow {
        model_id: GLOBAL_LLM_MODEL_ID.to_string(),
        model_rev: GLOBAL_LLM_REV.to_string(),
        created_at_ms,
        name: name.clone(),
        base_model_url: base_model_url.clone(),
        model_name: model_name.clone(),
        note: None,
    };
    db.upsert_llm_model_revision(&row).await?;

    store.api_keys.clear();
    if let Some(k) = api_key.filter(|k| !k.trim().is_empty()) {
        store
            .api_keys
            .insert(llm_api_key_slot(GLOBAL_LLM_MODEL_ID, GLOBAL_LLM_REV), k);
    }

    store.models = vec![LlmModelEntry {
        id: GLOBAL_LLM_MODEL_ID.to_string(),
        name,
        base_model_url,
        model_name,
        current_rev: GLOBAL_LLM_REV.to_string(),
        created_at_ms,
        updated_at_ms: now,
    }];
    store.active_id = GLOBAL_LLM_MODEL_ID.to_string();
    store.active_rev = GLOBAL_LLM_REV.to_string();
    if store.active_applied_at_ms.is_none() {
        store.active_applied_at_ms = Some(now);
    }
    Ok(true)
}

async fn load_llm_models_store(db: &GatewaySessionDb) -> Result<LlmModelsStore, sqlx::Error> {
    let (models_v, keys_v, active_id, active_rev, applied) =
        db.get_gateway_llm_models_raw().await?;
    let mut store = parse_llm_models_store(&models_v, &keys_v, &active_id, &active_rev, applied);
    let mut changed = ensure_llm_model_versions_backfilled(db, &mut store).await?;
    if ensure_global_llm_singleton(db, &mut store).await? {
        changed = true;
    }
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
    let models_v = serde_json::to_value(&store.models).unwrap_or_else(|_| serde_json::json!([]));
    let keys_v = serde_json::to_value(&store.api_keys).unwrap_or_else(|_| serde_json::json!({}));
    db.save_gateway_llm_models_raw(
        &models_v,
        &keys_v,
        &store.active_id,
        &store.active_rev,
        store.active_applied_at_ms,
        updated_at_ms,
    )
    .await
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
    let (name, base_model_url, model_name) =
        match db.get_llm_model_revision(&entry.id, &current_rev).await? {
            Some(row) => (row.name, row.base_model_url, row.model_name),
            None => (
                entry.name.clone(),
                entry.base_model_url.clone(),
                entry.model_name.clone(),
            ),
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
    let settings_v =
        serde_json::to_value(settings).unwrap_or_else(|_| serde_json::json!({"gitPats":[]}));
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
    })
}

pub async fn list_llm_model_versions(
    db: &GatewaySessionDb,
    model_id: &str,
) -> Result<LlmModelVersionsResponse, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    if id != GLOBAL_LLM_MODEL_ID {
        return Err("global LLM has no version history".into());
    }
    let store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    if !store.models.iter().any(|m| m.id == GLOBAL_LLM_MODEL_ID) {
        return Err("global LLM not configured".into());
    }
    Ok(LlmModelVersionsResponse {
        model_id: GLOBAL_LLM_MODEL_ID.to_string(),
        current_rev: GLOBAL_LLM_REV.to_string(),
        active_rev: active_llm_model_rev_public(&store),
        versions: vec![],
    })
}

pub async fn upsert_llm_model(
    db: &GatewaySessionDb,
    input: PutLlmModelInput,
) -> Result<LlmModelPublic, String> {
    save_global_llm(
        db,
        &input.name,
        &input.base_model_url,
        &input.model_name,
        input.api_key,
        input.note,
    )
    .await
}

pub async fn delete_llm_model(db: &GatewaySessionDb, _model_id: &str) -> Result<bool, String> {
    let mut store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    if store.models.is_empty() {
        return Ok(false);
    }
    db.delete_llm_model_revisions(GLOBAL_LLM_MODEL_ID)
        .await
        .map_err(|e| e.to_string())?;
    store.models.clear();
    store.api_keys.clear();
    store.active_id.clear();
    store.active_rev.clear();
    store.active_applied_at_ms = None;
    save_llm_models_store(db, &store, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 兼容旧路由：全局 LLM 无「生效」步骤，保存即 active。Author: kejiqing
pub async fn apply_llm_model_by_id(
    db: &GatewaySessionDb,
    model_id: &str,
    _model_rev: Option<&str>,
) -> Result<ApplyLlmModelResponse, String> {
    let id = normalize_llm_id(model_id).ok_or_else(|| "invalid llm model id".to_string())?;
    if id != GLOBAL_LLM_MODEL_ID {
        return Err("global LLM has no versions; save via PUT .../active-llm-config".into());
    }
    let store = load_llm_models_store(db).await.map_err(|e| e.to_string())?;
    let entry = store
        .models
        .iter()
        .find(|m| m.id == GLOBAL_LLM_MODEL_ID)
        .ok_or_else(|| "global LLM not configured".to_string())?;
    if llm_api_key_for(&store, GLOBAL_LLM_MODEL_ID, GLOBAL_LLM_REV).is_none() {
        return Err("apiKey is not configured".into());
    }
    let applied_at_ms = store.active_applied_at_ms.unwrap_or_else(now_ms);
    let public = llm_entry_to_public(db, entry, &store)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ApplyLlmModelResponse {
        active_llm_model_id: GLOBAL_LLM_MODEL_ID.to_string(),
        active_llm_model_rev: GLOBAL_LLM_REV.to_string(),
        active_llm_applied_at_ms: applied_at_ms,
        llm_model: public,
        outcome: LlmModelApplyOutcome {
            env_file: String::new(),
            applied_at_ms,
            tap_chain_refreshed: false,
            tap_restarted: false,
            message: Some("global LLM already active; gateway sync refreshes files".into()),
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
