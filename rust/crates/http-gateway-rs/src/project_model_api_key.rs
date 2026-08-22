//! Project-bound model API keys for OpenAI-compatible ingress. Author: kejiqing
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::session_db::GatewaySessionDb;

const TOKEN_PREFIX: &str = "ngmk_";

#[derive(Debug, Clone)]
pub struct ProjectModelApiKeyRow {
    pub id: String,
    pub cluster_id: String,
    pub proj_id: i64,
    pub model_alias: String,
    pub name: String,
    pub note: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub status: String,
    pub created_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectModelApiKeyPublic {
    pub id: String,
    pub proj_id: i64,
    pub model_alias: String,
    pub name: String,
    pub note: String,
    pub token_prefix: String,
    pub status: String,
    pub created_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueProjectModelApiKeyResponse {
    pub entry: ProjectModelApiKeyPublic,
    /// Plaintext returned only on create/rotate.
    pub token: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hash_token(plain: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plain.as_bytes());
    hex::encode(hasher.finalize())
}

fn random_secret_hex() -> String {
    let mut buf = [0u8; 24];
    getrandom::getrandom(&mut buf).expect("getrandom");
    hex::encode(buf)
}

fn allocate_id() -> String {
    format!("pmk-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

fn to_public(row: &ProjectModelApiKeyRow) -> ProjectModelApiKeyPublic {
    ProjectModelApiKeyPublic {
        id: row.id.clone(),
        proj_id: row.proj_id,
        model_alias: row.model_alias.clone(),
        name: row.name.clone(),
        note: row.note.clone(),
        token_prefix: row.token_prefix.clone(),
        status: row.status.clone(),
        created_at_ms: row.created_at_ms,
        revoked_at_ms: row.revoked_at_ms,
        last_used_at_ms: row.last_used_at_ms,
    }
}

fn row_from_sqlx(r: &sqlx::postgres::PgRow) -> ProjectModelApiKeyRow {
    ProjectModelApiKeyRow {
        id: r.get("id"),
        cluster_id: r.get("cluster_id"),
        proj_id: r.get("proj_id"),
        model_alias: r.get("model_alias"),
        name: r.get("name"),
        note: r.get("note"),
        token_hash: r.get("token_hash"),
        token_prefix: r.get("token_prefix"),
        status: r.get("status"),
        created_at_ms: r.get("created_at_ms"),
        revoked_at_ms: r.get("revoked_at_ms"),
        last_used_at_ms: r.get("last_used_at_ms"),
    }
}

impl GatewaySessionDb {
    pub async fn issue_project_model_api_key(
        &self,
        proj_id: i64,
        model_alias: &str,
        name: &str,
        note: &str,
    ) -> Result<IssueProjectModelApiKeyResponse, String> {
        if proj_id < 1 {
            return Err("projId must be >= 1".into());
        }
        let alias = model_alias.trim();
        let alias = if alias.is_empty() { "agent" } else { alias };
        let id = allocate_id();
        let secret = random_secret_hex();
        let plain = format!("{TOKEN_PREFIX}{id}_{secret}");
        let token_hash = hash_token(&plain);
        let token_prefix = plain.chars().take(12).collect::<String>();
        let now = now_ms();
        let cluster_id = self.cluster_id().to_string();
        sqlx::query(
            r#"INSERT INTO gateway_project_model_api_key (
                id, cluster_id, proj_id, model_alias, name, note, token_hash, token_prefix,
                status, created_at_ms, revoked_at_ms, last_used_at_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,NULL,NULL)"#,
        )
        .bind(&id)
        .bind(&cluster_id)
        .bind(proj_id)
        .bind(alias)
        .bind(name.trim())
        .bind(note.trim())
        .bind(&token_hash)
        .bind(&token_prefix)
        .bind(now)
        .execute(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        let entry = ProjectModelApiKeyRow {
            id,
            cluster_id,
            proj_id,
            model_alias: alias.to_string(),
            name: name.trim().to_string(),
            note: note.trim().to_string(),
            token_hash,
            token_prefix,
            status: "active".into(),
            created_at_ms: now,
            revoked_at_ms: None,
            last_used_at_ms: None,
        };
        Ok(IssueProjectModelApiKeyResponse {
            entry: to_public(&entry),
            token: plain,
        })
    }

    pub async fn list_project_model_api_keys(
        &self,
        proj_id: i64,
    ) -> Result<Vec<ProjectModelApiKeyPublic>, String> {
        let rows = sqlx::query(
            r#"SELECT id, cluster_id, proj_id, model_alias, name, note, token_hash, token_prefix,
                      status, created_at_ms, revoked_at_ms, last_used_at_ms
               FROM gateway_project_model_api_key
               WHERE proj_id = $1 AND cluster_id = $2
               ORDER BY created_at_ms DESC"#,
        )
        .bind(proj_id)
        .bind(self.cluster_id())
        .fetch_all(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows.iter().map(|r| to_public(&row_from_sqlx(r))).collect())
    }

    pub async fn revoke_project_model_api_key(&self, token_id: &str) -> Result<bool, String> {
        let now = now_ms();
        let res = sqlx::query(
            r#"UPDATE gateway_project_model_api_key
               SET status = 'revoked', revoked_at_ms = $1
               WHERE id = $2 AND cluster_id = $3 AND status = 'active'"#,
        )
        .bind(now)
        .bind(token_id.trim())
        .bind(self.cluster_id())
        .execute(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn verify_project_model_api_key(
        &self,
        plain: &str,
    ) -> Result<ProjectModelApiKeyRow, String> {
        let plain = plain.trim();
        if plain.is_empty() {
            return Err("missing token".into());
        }
        if !plain.starts_with(TOKEN_PREFIX) {
            return Err("invalid project model API key".into());
        }
        let token_hash = hash_token(plain);
        let row = sqlx::query(
            r#"SELECT id, cluster_id, proj_id, model_alias, name, note, token_hash, token_prefix,
                      status, created_at_ms, revoked_at_ms, last_used_at_ms
               FROM gateway_project_model_api_key
               WHERE token_hash = $1
               LIMIT 1"#,
        )
        .bind(&token_hash)
        .fetch_optional(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "invalid project model API key".to_string())?;
        let mut entry = row_from_sqlx(&row);
        if entry.status != "active" || entry.revoked_at_ms.is_some() {
            return Err("project model API key revoked".into());
        }
        let now = now_ms();
        let _ = sqlx::query(
            r#"UPDATE gateway_project_model_api_key SET last_used_at_ms = $1 WHERE id = $2"#,
        )
        .bind(now)
        .bind(&entry.id)
        .execute(self.pg_pool())
        .await;
        entry.last_used_at_ms = Some(now);
        Ok(entry)
    }

    pub async fn upsert_openai_conversation(
        &self,
        api_key_id: &str,
        proj_id: i64,
        client_conversation_key: &str,
        session_id: &str,
    ) -> Result<(), String> {
        let now = now_ms();
        let id = format!(
            "oc-{}",
            &uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_URL,
                format!("{api_key_id}:{client_conversation_key}").as_bytes()
            )
            .to_string()
            .replace('-', "")[..32]
        );
        sqlx::query(
            r#"INSERT INTO gateway_openai_conversation (
                id, cluster_id, api_key_id, proj_id, client_conversation_key, session_id,
                created_at_ms, updated_at_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$7)
            ON CONFLICT (api_key_id, client_conversation_key) DO UPDATE SET
                session_id = EXCLUDED.session_id,
                updated_at_ms = EXCLUDED.updated_at_ms,
                proj_id = EXCLUDED.proj_id"#,
        )
        .bind(&id)
        .bind(self.cluster_id())
        .bind(api_key_id)
        .bind(proj_id)
        .bind(client_conversation_key)
        .bind(session_id)
        .bind(now)
        .execute(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_openai_conversation_session(
        &self,
        api_key_id: &str,
        client_conversation_key: &str,
    ) -> Result<Option<(i64, String)>, String> {
        let row = sqlx::query(
            r#"SELECT proj_id, session_id FROM gateway_openai_conversation
               WHERE api_key_id = $1 AND client_conversation_key = $2
               LIMIT 1"#,
        )
        .bind(api_key_id)
        .bind(client_conversation_key)
        .fetch_optional(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(row.map(|r| (r.get::<i64, _>("proj_id"), r.get::<String, _>("session_id"))))
    }

    pub async fn insert_openai_response(
        &self,
        response_id: &str,
        api_key_id: &str,
        proj_id: i64,
        session_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        let now = now_ms();
        sqlx::query(
            r#"INSERT INTO gateway_openai_response (
                response_id, cluster_id, api_key_id, proj_id, session_id, turn_id, created_at_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT (response_id) DO NOTHING"#,
        )
        .bind(response_id)
        .bind(self.cluster_id())
        .bind(api_key_id)
        .bind(proj_id)
        .bind(session_id)
        .bind(turn_id)
        .bind(now)
        .execute(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_openai_response(
        &self,
        response_id: &str,
    ) -> Result<Option<(String, i64, String, String)>, String> {
        let row = sqlx::query(
            r#"SELECT api_key_id, proj_id, session_id, turn_id
               FROM gateway_openai_response WHERE response_id = $1 LIMIT 1"#,
        )
        .bind(response_id)
        .fetch_optional(self.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(row.map(|r| {
            (
                r.get("api_key_id"),
                r.get("proj_id"),
                r.get("session_id"),
                r.get("turn_id"),
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        assert_eq!(hash_token("ngmk_a_b"), hash_token("ngmk_a_b"));
        assert_ne!(hash_token("ngmk_a_b"), hash_token("ngmk_a_c"));
    }

    #[test]
    fn token_prefix_constant() {
        assert_eq!(TOKEN_PREFIX, "ngmk_");
    }
}
