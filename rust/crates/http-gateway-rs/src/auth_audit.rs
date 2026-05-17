//! Optional JWT gate and audit log (L5). Author: kejiqing
#![allow(clippy::must_use_candidate)]

use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub sub: String,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct AuthAuditState {
    pub enabled: bool,
    /// When enabled without real JWT parser, accept `Bearer dev:<sub>:<tenant>` for tests.
    pub audit: Arc<Mutex<Vec<AuditRow>>>,
    pub ds_tenant: HashMap<i64, String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditRow {
    pub ts_ms: i64,
    pub tenant_id: String,
    pub user_sub: String,
    pub session_id: String,
    pub ds_id: i64,
    pub action: String,
    pub detail: Value,
}

impl AuthAuditState {
    pub fn from_env() -> Self {
        let enabled = std::env::var("CLAW_GATEWAY_AUTH").is_ok_and(|v| {
            v == "1" || v.eq_ignore_ascii_case("true")
        });
        let mut ds_tenant = HashMap::new();
        if let Ok(raw) = std::env::var("CLAW_AUTH_DS_TENANT_MAP") {
            for part in raw.split(',') {
                let Some((ds, tenant)) = part.split_once(':') else {
                    continue;
                };
                if let Ok(ds_id) = ds.trim().parse::<i64>() {
                    ds_tenant.insert(ds_id, tenant.trim().to_string());
                }
            }
        }
        Self {
            enabled,
            audit: Arc::new(Mutex::new(Vec::new())),
            ds_tenant,
        }
    }

    pub fn parse_bearer(&self, headers: &HeaderMap) -> Result<AuthContext, StatusCode> {
        if !self.enabled {
            return Ok(AuthContext {
                sub: "anonymous".into(),
                tenant_id: None,
            });
        }
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let Some(token) = auth.strip_prefix("Bearer ") else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if let Some(rest) = token.strip_prefix("dev:") {
            let mut parts = rest.splitn(2, ':');
            let sub = parts.next().unwrap_or("user").to_string();
            let tenant = parts.next().map(String::from);
            return Ok(AuthContext { sub, tenant_id: tenant });
        }
        Err(StatusCode::UNAUTHORIZED)
    }

    pub fn authorize_ds(&self, ctx: &AuthContext, ds_id: i64) -> Result<(), StatusCode> {
        if !self.enabled {
            return Ok(());
        }
        let Some(required) = self.ds_tenant.get(&ds_id) else {
            return Ok(());
        };
        if ctx.tenant_id.as_deref() == Some(required.as_str()) {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }

    pub fn record(
        &self,
        ctx: &AuthContext,
        session_id: &str,
        ds_id: i64,
        action: &str,
        detail: Value,
    ) {
        if !self.enabled {
            return;
        }
        let row = AuditRow {
            ts_ms: now_ms(),
            tenant_id: ctx
                .tenant_id
                .clone()
                .unwrap_or_else(|| "default".into()),
            user_sub: ctx.sub.clone(),
            session_id: session_id.to_string(),
            ds_id,
            action: action.to_string(),
            detail,
        };
        self.audit.lock().expect("audit lock").push(row);
    }

    pub fn list_audit(&self, tenant_id: Option<&str>) -> Vec<AuditRow> {
        let rows = self.audit.lock().expect("audit lock");
        rows.iter()
            .filter(|r| match tenant_id {
                None => true,
                Some(t) => r.tenant_id == t,
            })
            .cloned()
            .collect()
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dev_bearer_parses() {
        let state = AuthAuditState {
            enabled: true,
            audit: Arc::new(Mutex::new(Vec::new())),
            ds_tenant: HashMap::from([(1, "team-a".into())]),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer dev:alice:team-a".parse().unwrap(),
        );
        let ctx = state.parse_bearer(&headers).unwrap();
        assert_eq!(ctx.sub, "alice");
        assert!(state.authorize_ds(&ctx, 1).is_ok());
        assert!(state.authorize_ds(&ctx, 2).is_ok());
    }

    #[tokio::test]
    async fn audit_records_when_enabled() {
        let state = AuthAuditState {
            enabled: true,
            audit: Arc::new(Mutex::new(Vec::new())),
            ds_tenant: HashMap::new(),
        };
        let ctx = AuthContext {
            sub: "u".into(),
            tenant_id: Some("t".into()),
        };
        state.record(&ctx, "sess", 1, "solve.start", json!({}));
        assert_eq!(state.list_audit(Some("t")).len(), 1);
    }
}
