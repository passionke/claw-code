//! Per-`CLAW_CLUSTER_ID` LLM model storage + API key encryption (key = clusterId). Author: kejiqing

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};

use crate::cluster_identity;
use crate::gateway_global_settings::{LlmModelEntry, LlmModelsStore};
use crate::gateway_llm_model_revision::llm_api_key_slot;
use crate::session_db::{GatewayLlmClusterModelRow, GatewaySessionDb};

const NONCE_LEN: usize = 12;

fn derive_aes_key(cluster_id: &str) -> [u8; 32] {
    Sha256::digest(cluster_id.trim().as_bytes()).into()
}

/// Encrypt plaintext API key; stored as hex(nonce || ciphertext). Author: kejiqing
pub fn encrypt_llm_api_key(cluster_id: &str, plaintext: &str) -> Result<String, String> {
    let key = derive_aes_key(cluster_id);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("llm api key cipher init: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| format!("llm api key nonce: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("llm api key encrypt: {e}"))?;
    Ok(format!(
        "{}{}",
        hex::encode(nonce_bytes),
        hex::encode(ciphertext)
    ))
}

/// Decrypt API key ciphertext produced by [`encrypt_llm_api_key`]. Author: kejiqing
pub fn decrypt_llm_api_key(cluster_id: &str, stored: &str) -> Result<String, String> {
    let stored = stored.trim();
    if stored.is_empty() {
        return Err("empty llm api key ciphertext".into());
    }
    let bytes = hex::decode(stored).map_err(|e| format!("llm api key hex decode: {e}"))?;
    if bytes.len() <= NONCE_LEN {
        return Err("llm api key ciphertext too short".into());
    }
    let (nonce_bytes, ct) = bytes.split_at(NONCE_LEN);
    let key = derive_aes_key(cluster_id);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("llm api key cipher init: {e}"))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|e| format!("llm api key decrypt: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| format!("llm api key utf8: {e}"))
}

pub fn resolve_llm_cluster_id() -> Option<String> {
    cluster_identity::gateway_cluster_id_optional()
}

pub async fn migrate_legacy_llm_if_needed(
    db: &GatewaySessionDb,
    cluster_id: &str,
) -> Result<(), sqlx::Error> {
    if db.count_llm_cluster_models(cluster_id).await? > 0 {
        return Ok(());
    }
    let (models_v, keys_v, active_id, active_rev, applied) =
        db.get_gateway_llm_models_raw().await?;
    let models: Vec<LlmModelEntry> = serde_json::from_value(models_v).unwrap_or_default();
    if models.is_empty() {
        return Ok(());
    }
    let api_keys: std::collections::BTreeMap<String, String> =
        serde_json::from_value(keys_v).unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));

    for entry in &models {
        let rev = if entry.current_rev.is_empty() {
            crate::gateway_llm_model_revision::format_model_rev_local_ms(entry.updated_at_ms)
        } else {
            entry.current_rev.clone()
        };
        let slot = llm_api_key_slot(&entry.id, &rev);
        let api_key = api_keys
            .get(&slot)
            .or_else(|| api_keys.get(&entry.id))
            .cloned()
            .unwrap_or_default();
        let ciphertext = if api_key.trim().is_empty() {
            String::new()
        } else {
            encrypt_llm_api_key(cluster_id, api_key.trim()).map_err(sqlx::Error::Protocol)?
        };
        db.upsert_llm_cluster_model(&GatewayLlmClusterModelRow {
            cluster_id: cluster_id.to_string(),
            model_id: entry.id.clone(),
            name: entry.name.clone(),
            base_model_url: entry.base_model_url.clone(),
            model_name: entry.model_name.clone(),
            current_rev: rev.clone(),
            api_key_ciphertext: ciphertext,
            created_at_ms: entry.created_at_ms,
            updated_at_ms: entry.updated_at_ms,
        })
        .await?;
        if let Some(row) = db.get_llm_model_revision(&entry.id, &rev).await? {
            db.upsert_llm_cluster_revision(&row.with_cluster_id(cluster_id))
                .await?;
        }
    }

    db.save_llm_cluster_state(
        cluster_id,
        active_id.trim(),
        active_rev.trim(),
        applied,
        now,
    )
    .await?;
    Ok(())
}

pub async fn load_cluster_llm_store(
    db: &GatewaySessionDb,
    cluster_id: &str,
) -> Result<LlmModelsStore, sqlx::Error> {
    migrate_legacy_llm_if_needed(db, cluster_id).await?;
    let state = db.get_llm_cluster_state(cluster_id).await?;
    let rows = db.list_llm_cluster_models(cluster_id).await?;
    let mut store = LlmModelsStore {
        models: rows
            .iter()
            .map(|r| LlmModelEntry {
                id: r.model_id.clone(),
                name: r.name.clone(),
                base_model_url: r.base_model_url.clone(),
                model_name: r.model_name.clone(),
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

pub async fn save_cluster_llm_store(
    db: &GatewaySessionDb,
    cluster_id: &str,
    store: &LlmModelsStore,
    updated_at_ms: i64,
) -> Result<(), sqlx::Error> {
    let existing = db.list_llm_cluster_models(cluster_id).await?;
    let keep: std::collections::HashSet<&str> =
        store.models.iter().map(|m| m.id.as_str()).collect();
    for row in existing {
        if !keep.contains(row.model_id.as_str()) {
            db.delete_llm_cluster_model(cluster_id, &row.model_id)
                .await?;
            db.delete_llm_cluster_revisions(cluster_id, &row.model_id)
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
        db.upsert_llm_cluster_model(&GatewayLlmClusterModelRow {
            cluster_id: cluster_id.to_string(),
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
    db.save_llm_cluster_state(
        cluster_id,
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
    use super::*;

    #[test]
    fn encrypt_roundtrip() {
        let plain = "sk-test-key-123";
        let enc = encrypt_llm_api_key("local-dev", plain).expect("encrypt");
        assert_ne!(enc, plain);
        let dec = decrypt_llm_api_key("local-dev", &enc).expect("decrypt");
        assert_eq!(dec, plain);
    }

    #[test]
    fn decrypt_wrong_cluster_fails() {
        let enc = encrypt_llm_api_key("cluster-a", "sk-x").expect("encrypt");
        assert!(decrypt_llm_api_key("cluster-b", &enc).is_err());
    }
}
