//! Admin account / membership / session persistence. Author: kejiqing

use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::session_db::GatewaySessionDb;

use super::acl::AuthPrincipal;
use super::password::{hash_password, verify_password};

pub const SESSION_TOKEN_PREFIX: &str = "cass_";
const SESSION_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn hash_token(plain: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plain.as_bytes());
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone)]
pub struct AccountRow {
    pub account_id: String,
    pub cluster_id: String,
    pub username: String,
    pub password_hash: String,
    pub system_role: String,
    pub disabled: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl AccountRow {
    #[must_use]
    pub fn is_system_admin(&self) -> bool {
        self.system_role == "system_admin"
    }
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AccountPublic {
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub username: String,
    #[serde(rename = "systemRole")]
    pub system_role: String,
    pub disabled: bool,
    #[serde(rename = "projectIds")]
    pub project_ids: Vec<i64>,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

pub async fn count_accounts(db: &GatewaySessionDb) -> Result<i64, String> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM gateway_admin_accounts WHERE cluster_id = $1",
    )
    .bind(db.cluster_id())
    .fetch_one(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(n)
}

pub async fn get_account_by_username(
    db: &GatewaySessionDb,
    username: &str,
) -> Result<Option<AccountRow>, String> {
    let row = sqlx::query(
        "SELECT account_id, cluster_id, username, password_hash, system_role, disabled,
                created_at_ms, updated_at_ms
         FROM gateway_admin_accounts
         WHERE cluster_id = $1 AND username = $2",
    )
    .bind(db.cluster_id())
    .bind(username)
    .fetch_optional(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| row_to_account(&r)))
}

pub async fn get_account_by_id(
    db: &GatewaySessionDb,
    account_id: &str,
) -> Result<Option<AccountRow>, String> {
    let row = sqlx::query(
        "SELECT account_id, cluster_id, username, password_hash, system_role, disabled,
                created_at_ms, updated_at_ms
         FROM gateway_admin_accounts
         WHERE cluster_id = $1 AND account_id = $2",
    )
    .bind(db.cluster_id())
    .bind(account_id)
    .fetch_optional(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| row_to_account(&r)))
}

fn row_to_account(r: &sqlx::postgres::PgRow) -> AccountRow {
    AccountRow {
        account_id: r.get("account_id"),
        cluster_id: r.get("cluster_id"),
        username: r.get("username"),
        password_hash: r.get("password_hash"),
        system_role: r.get("system_role"),
        disabled: r.get("disabled"),
        created_at_ms: r.get("created_at_ms"),
        updated_at_ms: r.get("updated_at_ms"),
    }
}

pub async fn list_project_ids_for_account(
    db: &GatewaySessionDb,
    account_id: &str,
) -> Result<Vec<i64>, String> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT proj_id FROM gateway_admin_project_members
         WHERE cluster_id = $1 AND account_id = $2
         ORDER BY proj_id",
    )
    .bind(db.cluster_id())
    .bind(account_id)
    .fetch_all(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn principal_from_account(
    db: &GatewaySessionDb,
    account: &AccountRow,
) -> Result<AuthPrincipal, String> {
    if account.disabled {
        return Err("account disabled".into());
    }
    let project_ids = if account.is_system_admin() {
        Vec::new()
    } else {
        list_project_ids_for_account(db, &account.account_id).await?
    };
    Ok(AuthPrincipal {
        account_id: account.account_id.clone(),
        username: account.username.clone(),
        system_admin: account.is_system_admin(),
        project_ids,
        legacy_unbound_camt: false,
    })
}

pub async fn to_public(
    db: &GatewaySessionDb,
    account: &AccountRow,
) -> Result<AccountPublic, String> {
    let project_ids = list_project_ids_for_account(db, &account.account_id).await?;
    Ok(AccountPublic {
        account_id: account.account_id.clone(),
        username: account.username.clone(),
        system_role: account.system_role.clone(),
        disabled: account.disabled,
        project_ids,
        created_at_ms: account.created_at_ms,
        updated_at_ms: account.updated_at_ms,
    })
}

/// Seed system_admin from PLAYGROUND_ADMIN_* when cluster has zero accounts.
pub async fn ensure_seed_system_admin(
    db: &GatewaySessionDb,
) -> Result<Option<AccountPublic>, String> {
    if count_accounts(db).await? > 0 {
        return Ok(None);
    }
    let username = std::env::var("PLAYGROUND_ADMIN_USER")
        .unwrap_or_else(|_| "admin".into())
        .trim()
        .to_string();
    let password = std::env::var("PLAYGROUND_ADMIN_PASSWORD").unwrap_or_else(|_| "sunmi123".into());
    if username.is_empty() {
        return Err("PLAYGROUND_ADMIN_USER empty; cannot seed admin".into());
    }
    let created = create_account(db, &username, &password, "system_admin").await?;
    Ok(Some(created))
}

pub async fn create_account(
    db: &GatewaySessionDb,
    username: &str,
    password: &str,
    system_role: &str,
) -> Result<AccountPublic, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("username is required".into());
    }
    if password.len() < 6 {
        return Err("password must be at least 6 characters".into());
    }
    if system_role != "system_admin" && system_role != "none" {
        return Err("systemRole must be system_admin or none".into());
    }
    let account_id = format!("acc-{}", Uuid::new_v4());
    let password_hash = hash_password(password)?;
    let now = now_ms();
    sqlx::query(
        "INSERT INTO gateway_admin_accounts
         (account_id, cluster_id, username, password_hash, system_role, disabled, created_at_ms, updated_at_ms)
         VALUES ($1, $2, $3, $4, $5, FALSE, $6, $6)",
    )
    .bind(&account_id)
    .bind(db.cluster_id())
    .bind(username)
    .bind(&password_hash)
    .bind(system_role)
    .bind(now)
    .execute(db.pg_pool())
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(d) = &e {
            if d.is_unique_violation() {
                return "username already exists".to_string();
            }
        }
        e.to_string()
    })?;
    let row = get_account_by_id(db, &account_id)
        .await?
        .ok_or_else(|| "account missing after insert".to_string())?;
    to_public(db, &row).await
}

pub async fn list_accounts(db: &GatewaySessionDb) -> Result<Vec<AccountPublic>, String> {
    let rows = sqlx::query(
        "SELECT account_id, cluster_id, username, password_hash, system_role, disabled,
                created_at_ms, updated_at_ms
         FROM gateway_admin_accounts
         WHERE cluster_id = $1
         ORDER BY username",
    )
    .bind(db.cluster_id())
    .fetch_all(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let acc = row_to_account(r);
        out.push(to_public(db, &acc).await?);
    }
    Ok(out)
}

pub async fn patch_account(
    db: &GatewaySessionDb,
    account_id: &str,
    password: Option<&str>,
    system_role: Option<&str>,
    disabled: Option<bool>,
) -> Result<AccountPublic, String> {
    let Some(mut acc) = get_account_by_id(db, account_id).await? else {
        return Err("account not found".into());
    };
    if let Some(role) = system_role {
        if role != "system_admin" && role != "none" {
            return Err("systemRole must be system_admin or none".into());
        }
        acc.system_role = role.to_string();
    }
    if let Some(d) = disabled {
        acc.disabled = d;
    }
    let mut new_hash = None;
    if let Some(pw) = password {
        if pw.len() < 6 {
            return Err("password must be at least 6 characters".into());
        }
        new_hash = Some(hash_password(pw)?);
    }
    let now = now_ms();
    if let Some(hash) = new_hash {
        sqlx::query(
            "UPDATE gateway_admin_accounts
             SET password_hash = $1, system_role = $2, disabled = $3, updated_at_ms = $4
             WHERE cluster_id = $5 AND account_id = $6",
        )
        .bind(&hash)
        .bind(&acc.system_role)
        .bind(acc.disabled)
        .bind(now)
        .bind(db.cluster_id())
        .bind(account_id)
        .execute(db.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
    } else {
        sqlx::query(
            "UPDATE gateway_admin_accounts
             SET system_role = $1, disabled = $2, updated_at_ms = $3
             WHERE cluster_id = $4 AND account_id = $5",
        )
        .bind(&acc.system_role)
        .bind(acc.disabled)
        .bind(now)
        .bind(db.cluster_id())
        .bind(account_id)
        .execute(db.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
    }
    let updated = get_account_by_id(db, account_id)
        .await?
        .ok_or_else(|| "account missing after update".to_string())?;
    to_public(db, &updated).await
}

pub async fn upsert_project_member(
    db: &GatewaySessionDb,
    account_id: &str,
    proj_id: i64,
) -> Result<(), String> {
    if proj_id < 1 {
        return Err("projId must be >= 1".into());
    }
    let Some(acc) = get_account_by_id(db, account_id).await? else {
        return Err("account not found".into());
    };
    let _ = acc;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM project_config WHERE cluster_id = $1 AND proj_id = $2)",
    )
    .bind(db.cluster_id())
    .bind(proj_id)
    .fetch_one(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    if !exists {
        return Err(format!("project_config not found for projId={proj_id}"));
    }
    let now = now_ms();
    sqlx::query(
        "INSERT INTO gateway_admin_project_members
         (cluster_id, account_id, proj_id, role, created_at_ms)
         VALUES ($1, $2, $3, 'space_admin', $4)
         ON CONFLICT (cluster_id, account_id, proj_id) DO NOTHING",
    )
    .bind(db.cluster_id())
    .bind(account_id)
    .bind(proj_id)
    .bind(now)
    .execute(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn delete_project_member(
    db: &GatewaySessionDb,
    account_id: &str,
    proj_id: i64,
) -> Result<bool, String> {
    let res = sqlx::query(
        "DELETE FROM gateway_admin_project_members
         WHERE cluster_id = $1 AND account_id = $2 AND proj_id = $3",
    )
    .bind(db.cluster_id())
    .bind(account_id)
    .bind(proj_id)
    .execute(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(res.rows_affected() > 0)
}

pub async fn create_session(
    db: &GatewaySessionDb,
    account_id: &str,
) -> Result<(String, i64), String> {
    let session_id = format!("sess-{}", Uuid::new_v4());
    let secret = Uuid::new_v4().to_string().replace('-', "");
    let plain = format!("{SESSION_TOKEN_PREFIX}{session_id}_{secret}");
    let token_hash = hash_token(&plain);
    let now = now_ms();
    let expires = now.saturating_add(SESSION_TTL_MS);
    sqlx::query(
        "INSERT INTO gateway_admin_sessions
         (session_id, cluster_id, account_id, token_hash, expires_at_ms, created_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&session_id)
    .bind(db.cluster_id())
    .bind(account_id)
    .bind(&token_hash)
    .bind(expires)
    .bind(now)
    .execute(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok((plain, expires))
}

pub async fn revoke_session_by_token(db: &GatewaySessionDb, plain: &str) -> Result<bool, String> {
    let hash = hash_token(plain.trim());
    let res = sqlx::query("DELETE FROM gateway_admin_sessions WHERE token_hash = $1")
        .bind(&hash)
        .execute(db.pg_pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.rows_affected() > 0)
}

pub async fn resolve_session_principal(
    db: &GatewaySessionDb,
    plain: &str,
) -> Result<AuthPrincipal, String> {
    let plain = plain.trim();
    if !plain.starts_with(SESSION_TOKEN_PREFIX) {
        return Err("not a session token".into());
    }
    let hash = hash_token(plain);
    let now = now_ms();
    let row = sqlx::query(
        "SELECT session_id, account_id, expires_at_ms
         FROM gateway_admin_sessions
         WHERE cluster_id = $1 AND token_hash = $2",
    )
    .bind(db.cluster_id())
    .bind(&hash)
    .fetch_optional(db.pg_pool())
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Err("invalid session".into());
    };
    let expires_at_ms: i64 = row.get("expires_at_ms");
    if now >= expires_at_ms {
        let _ = sqlx::query("DELETE FROM gateway_admin_sessions WHERE token_hash = $1")
            .bind(&hash)
            .execute(db.pg_pool())
            .await;
        return Err("session expired".into());
    }
    let account_id: String = row.get("account_id");
    let Some(acc) = get_account_by_id(db, &account_id).await? else {
        return Err("account not found".into());
    };
    principal_from_account(db, &acc).await
}

pub async fn login(
    db: &GatewaySessionDb,
    username: &str,
    password: &str,
) -> Result<(AuthPrincipal, String, i64), String> {
    ensure_seed_system_admin(db).await?;
    let username = username.trim();
    let Some(acc) = get_account_by_username(db, username).await? else {
        return Err("invalid username or password".into());
    };
    if acc.disabled {
        return Err("account disabled".into());
    }
    if !verify_password(password, &acc.password_hash)? {
        return Err("invalid username or password".into());
    }
    let principal = principal_from_account(db, &acc).await?;
    let (token, expires) = create_session(db, &acc.account_id).await?;
    Ok((principal, token, expires))
}
