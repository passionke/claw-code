//! e2b NAS API client — gateway NAS layout + file writes via the `claw-nas-api`
//! singleton (ensured on gateway startup + lease ticker). Author: kejiqing
//!
//! The gateway ensures the nas-api sandbox exists and registers it for TTL renewal;
//! endpoint is read from PG (`gateway_global_settings.settings_json.e2bNasApi`) on
//! every call so re-deploy picks up without gateway restart.

use std::sync::Arc;

use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, EXPECT};
use serde::Serialize;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::gateway_e2b_nas_api_settings::load_e2b_nas_api_base_url;
use crate::project_git_sync::split_git_import_upload_parts;
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
struct RmdirBody<'a> {
    #[serde(rename = "relPath")]
    rel_path: &'a str,
    recursive: bool,
}

#[derive(Serialize)]
struct SymlinkBody<'a> {
    #[serde(rename = "relPath")]
    rel_path: &'a str,
    target: &'a str,
}

#[derive(Serialize)]
struct GitImportFinishBody<'a> {
    #[serde(rename = "destRelPath")]
    dest_rel_path: &'a str,
    #[serde(rename = "uploadId")]
    upload_id: &'a str,
    parts: usize,
}

impl Default for E2bNasApiSingleton {
    fn default() -> Self {
        Self::new()
    }
}

impl E2bNasApiSingleton {
    #[must_use]
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
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
            .header(CONTENT_LENGTH, bytes.len())
            .header(EXPECT, "")
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

    /// Chunked git-import: PUT parts (≤64MB each) then POST finish → one extract. Author: kejiqing
    pub async fn upload_git_import_tar_gz_dest(
        &self,
        dest_rel_path: &str,
        tar_gz: &[u8],
    ) -> Result<(), String> {
        let base = self.base_url().await?;
        let dest = dest_rel_path.trim_start_matches('/');
        let upload_id = Uuid::new_v4().simple().to_string();
        let parts = split_git_import_upload_parts(tar_gz);
        let part_count = parts.len();
        if part_count == 1 {
            return self
                .extract_tar_git_import_dest(dest_rel_path, tar_gz)
                .await;
        }
        for (index, chunk) in parts.into_iter().enumerate() {
            let url = format!("{base}/v1/git-import-parts/{upload_id}/{index}");
            let resp = self
                .http
                .put(&url)
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(CONTENT_LENGTH, chunk.len())
                .header(EXPECT, "")
                .body(chunk)
                .send()
                .await
                .map_err(|e| format!("nas-api git-import part {index} request: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(format!(
                    "nas-api git-import part {index} HTTP {status}: {text}"
                ));
            }
        }
        let finish_url = format!("{base}/v1/git-import-finish");
        let resp = self
            .http
            .post(&finish_url)
            .json(&GitImportFinishBody {
                dest_rel_path: dest,
                upload_id: &upload_id,
                parts: part_count,
            })
            .send()
            .await
            .map_err(|e| format!("nas-api git-import-finish request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api git-import-finish HTTP {status}: {text}"))
    }

    /// `PUT /v1/extract-tar/{relPath}` — single-shot tar.gz upload. Author: kejiqing
    pub async fn extract_tar_git_import_dest(
        &self,
        dest_rel_path: &str,
        tar_gz: &[u8],
    ) -> Result<(), String> {
        let base = self.base_url().await?;
        let rel = dest_rel_path.trim_start_matches('/');
        let url = format!("{base}/v1/extract-tar/{rel}");
        let resp = self
            .http
            .put(&url)
            .header(CONTENT_TYPE, "application/gzip")
            .header(CONTENT_LENGTH, tar_gz.len())
            .header(EXPECT, "")
            .body(tar_gz.to_vec())
            .send()
            .await
            .map_err(|e| format!("nas-api extract-tar request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api extract-tar HTTP {status}: {text}"))
    }

    /// `POST /v1/rmdir` — recursive delete of a git-import dest under proj home. Author: kejiqing
    pub async fn rmdir_git_import_dest(&self, rel_path: &str) -> Result<(), String> {
        let base = self.base_url().await?;
        let url = format!("{base}/v1/rmdir");
        let resp = self
            .http
            .post(&url)
            .json(&RmdirBody {
                rel_path,
                recursive: true,
            })
            .send()
            .await
            .map_err(|e| format!("nas-api rmdir request: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("nas-api rmdir HTTP {status}: {text}"))
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

    /// Optional startup probe: PG `e2bNasApi.baseUrl` (warn-only at boot; errors on use).
    pub async fn verify_endpoint_configured(&self) -> Result<(), String> {
        self.base_url().await.map(|_| ())
    }

    /// `CLAW_E2B_NAS_API` gate: unset/`1`/`true` → enabled; `0`/`false`/`no`/`off` → disabled.
    #[must_use]
    pub fn enabled_from_env() -> bool {
        !matches!(
            std::env::var("CLAW_E2B_NAS_API")
                .ok()
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("0" | "false" | "no" | "off")
        )
    }
}
