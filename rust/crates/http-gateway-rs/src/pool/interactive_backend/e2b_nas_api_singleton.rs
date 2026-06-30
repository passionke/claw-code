//! e2b NAS API client — gateway NAS layout + file writes via the out-of-band
//! `claw-nas-api` singleton (deployed by `gateway.sh nas-api-up`). Author: kejiqing
//!
//! Boundary: the gateway NEVER creates the nas-api sandbox. It discovers the
//! endpoint from PG (`gateway_global_settings.settings_json.e2bNasApi`, written by
//! `e2b-nas-api-up.py`) on every call, so a re-deploy of the singleton is picked up
//! without restarting the gateway. If no endpoint is configured, startup fails
//! with a hint to run `./deploy/stack/gateway.sh nas-api-up`.

use std::sync::Arc;

use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::gateway_e2b_nas_api_settings::load_e2b_nas_api_base_url;
use crate::session_db::GatewaySessionDb;

/// HTTP client for the out-of-band nas-api singleton (endpoint resolved from PG).
pub struct E2bNasApiSingleton {
    http: reqwest::Client,
    db: RwLock<Option<Arc<GatewaySessionDb>>>,
}

#[derive(Serialize)]
struct MkdirBody<'a> {
    #[serde(rename = "relPath")]
    rel_path: &'a str,
    parents: bool,
}

#[derive(Serialize)]
struct SymlinkBody<'a> {
    #[serde(rename = "relPath")]
    rel_path: &'a str,
    target: &'a str,
}

impl Default for E2bNasApiSingleton {
    fn default() -> Self {
        Self::new()
    }
}

impl E2bNasApiSingleton {
    #[must_use]
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            db: RwLock::new(None),
        }
    }

    /// Bind the session DB after gateway startup (PG is opened after pool wiring).
    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        *self.db.write().await = Some(db);
    }

    /// Resolve nas-api `baseUrl` from PG on every call (no caching → re-deploy aware).
    async fn base_url(&self) -> Result<String, String> {
        let db = {
            let guard = self.db.read().await;
            guard.clone()
        };
        let db = db.ok_or_else(|| "nas-api: session DB not bound yet".to_string())?;
        let url = load_e2b_nas_api_base_url(&db)
            .await
            .map_err(|e| format!("load e2bNasApi endpoint from PG: {e}"))?;
        url.ok_or_else(|| {
            "nas-api endpoint not configured in PG; deploy it first: \
             ./deploy/stack/gateway.sh nas-api-up"
                .to_string()
        })
    }

    /// `POST /v1/mkdir` — create directory under NAS export root (`parents=true` → mkdir -p).
    pub async fn mkdir(&self, rel_path: &str, parents: bool) -> Result<(), String> {
        let base = self.base_url().await?;
        let url = format!("{base}/v1/mkdir");
        let resp = self
            .http
            .post(&url)
            .json(&MkdirBody { rel_path, parents })
            .send()
            .await
            .map_err(|e| format!("nas-api mkdir request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api mkdir HTTP {status}: {text}"))
    }

    /// `PUT /v1/files/{relPath}` — write bytes under NAS export root.
    pub async fn put_file(&self, rel_path: &str, bytes: &[u8]) -> Result<(), String> {
        let base = self.base_url().await?;
        let rel = rel_path.trim_start_matches('/');
        let url = format!("{base}/v1/files/{rel}");
        let resp = self
            .http
            .put(&url)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|e| format!("nas-api put_file request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api put_file HTTP {status}: {text}"))
    }

    /// Write project-home bytes through the generic cluster-aware file API. Author: kejiqing
    pub async fn put_proj_home_file(
        &self,
        cluster_id: &str,
        proj_id: i64,
        rel_path: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let rel = rel_path.trim_start_matches('/');
        let home_rel = format!("{cluster_id}/proj_{proj_id}/home/{rel}");
        self.put_file(&home_rel, bytes).await
    }

    /// `GET /v1/files/{relPath}` — read bytes under NAS export root; `Ok(None)` on 404.
    pub async fn get_file(&self, rel_path: &str) -> Result<Option<Vec<u8>>, String> {
        let base = self.base_url().await?;
        let rel = rel_path.trim_start_matches('/');
        let url = format!("{base}/v1/files/{rel}");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("nas-api get_file request: {e}"))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("nas-api get_file HTTP {status}: {text}"));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("nas-api get_file body: {e}"))?;
        Ok(Some(bytes.to_vec()))
    }

    /// `POST /v1/symlink` — session → worker link.
    pub async fn symlink(&self, rel_path: &str, target: &str) -> Result<(), String> {
        let base = self.base_url().await?;
        let url = format!("{base}/v1/symlink");
        let resp = self
            .http
            .post(&url)
            .json(&SymlinkBody { rel_path, target })
            .send()
            .await
            .map_err(|e| format!("nas-api symlink request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api symlink HTTP {status}: {text}"))
    }

    /// `DELETE /v1/path/{relPath}` — remove file or symlink (not directories).
    pub async fn unlink(&self, rel_path: &str) -> Result<(), String> {
        let base = self.base_url().await?;
        let rel = rel_path.trim_start_matches('/');
        let url = format!("{base}/v1/path/{rel}");
        let resp = self
            .http
            .delete(&url)
            .send()
            .await
            .map_err(|e| format!("nas-api unlink request: {e}"))?;
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api unlink HTTP {status}: {text}"))
    }

    /// Startup probe: PG must have `e2bNasApi.baseUrl` before e2b NAS layout runs.
    pub async fn verify_endpoint_configured(&self) -> Result<(), String> {
        self.base_url().await.map(|_| ())
    }

    /// `CLAW_E2B_NAS_API` gate: unset/`1`/`true` → enabled; `0`/`false`/`no`/`off` → disabled.
    #[must_use]
    pub fn enabled_from_env() -> bool {
        match std::env::var("CLAW_E2B_NAS_API")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("0") | Some("false") | Some("no") | Some("off") => false,
            _ => true,
        }
    }
}
