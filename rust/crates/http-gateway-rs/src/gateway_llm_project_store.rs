//! Per-`(CLAW_CLUSTER_ID, proj_id)` LLM model storage (reuses cluster AES key). Author: kejiqing

use crate::gateway_global_settings::{LlmModelEntry, LlmModelsStore};
use crate::gateway_llm_cluster_store::{decrypt_llm_api_key, encrypt_llm_api_key};
use crate::gateway_llm_model_revision::llm_api_key_slot;
use crate::session_db::{GatewayLlmProjectModelRow, GatewaySessionDb};

pub async fn load_project_llm_store(
    db: &GatewaySessionDb,
    cluster_id: &str,
    proj_id: i64,
) -> Result<LlmModelsStore, sqlx::Error> {
    let state = db.get_llm_project_state(cluster_id, proj_id).await?;
    let rows = db.list_llm_project_models(cluster_id, proj_id).await?;
    let mut store = LlmModelsStore {
        models: rows
            .iter()
            .map(|r| LlmModelEntry {
                id: r.model_id.clone(),
                name: r.name.clone(),
                base_model_url: r.base_model_url.clone(),
                model_name: r.model_name.clone(),
                supports_vision: false,
                supports_video: false,
                supports_audio: false,
                current_rev: r.current_rev.clone(),
                created_at_ms: r.created_at_ms,
                updated_at_ms: r.updated_at_ms,
            })
            .collect(),
        api_keys: std::collections::BTreeMap::new(),
        active_id: state
            .as_ref()
            .map(|s| s.active_model_id.clone())
            .unwrap_or_default(),
        active_rev: state
            .as_ref()
            .map(|s| s.active_model_rev.clone())
            .unwrap_or_default(),
        active_applied_at_ms: state.as_ref().and_then(|s| s.active_applied_at_ms),
    };
    for row in &rows {
        if row.api_key_ciphertext.trim().is_empty() {
            continue;
        }
        let rev = if row.current_rev.is_empty() {
            crate::gateway_llm_model_revision::format_model_rev_local_ms(row.updated_at_ms)
        } else {
            row.current_rev.clone()
        };
        if let Ok(key) = decrypt_llm_api_key(cluster_id, &row.api_key_ciphertext) {
            if !key.trim().is_empty() {
                store
                    .api_keys
                    .insert(llm_api_key_slot(&row.model_id, &rev), key);
            }
        }
    }
    Ok(store)
}

pub async fn save_project_llm_store(
    db: &GatewaySessionDb,
    cluster_id: &str,
    proj_id: i64,
    store: &LlmModelsStore,
    updated_at_ms: i64,
) -> Result<(), sqlx::Error> {
    let existing = db.list_llm_project_models(cluster_id, proj_id).await?;
    let keep: std::collections::HashSet<&str> =
        store.models.iter().map(|m| m.id.as_str()).collect();
    for row in existing {
        if !keep.contains(row.model_id.as_str()) {
            db.delete_llm_project_model(cluster_id, proj_id, &row.model_id)
                .await?;
            db.delete_llm_project_revisions(cluster_id, proj_id, &row.model_id)
                .await?;
        }
    }
    for entry in &store.models {
        let rev = if entry.current_rev.is_empty() {
            crate::gateway_llm_model_revision::format_model_rev_local_ms(entry.updated_at_ms)
        } else {
            entry.current_rev.clone()
        };
        let api_key = store
            .api_keys
            .get(&llm_api_key_slot(&entry.id, &rev))
            .or_else(|| store.api_keys.get(&entry.id))
            .cloned()
            .unwrap_or_default();
        let ciphertext = if api_key.trim().is_empty() {
            String::new()
        } else {
            encrypt_llm_api_key(cluster_id, api_key.trim()).map_err(sqlx::Error::Protocol)?
        };
        db.upsert_llm_project_model(&GatewayLlmProjectModelRow {
            cluster_id: cluster_id.to_string(),
            proj_id,
            model_id: entry.id.clone(),
            name: entry.name.clone(),
            base_model_url: entry.base_model_url.clone(),
            model_name: entry.model_name.clone(),
            current_rev: rev,
            api_key_ciphertext: ciphertext,
            created_at_ms: entry.created_at_ms,
            updated_at_ms: entry.updated_at_ms,
        })
        .await?;
    }
    db.save_llm_project_state(
        cluster_id,
        proj_id,
        &store.active_id,
        &store.active_rev,
        store.active_applied_at_ms,
        updated_at_ms,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::gateway_llm_cluster_store::{decrypt_llm_api_key, encrypt_llm_api_key};

    #[test]
    fn project_llm_key_uses_cluster_aes() {
        let enc = encrypt_llm_api_key("local-dev", "sk-proj").expect("encrypt");
        let dec = decrypt_llm_api_key("local-dev", &enc).expect("decrypt");
        assert_eq!(dec, "sk-proj");
    }
}
