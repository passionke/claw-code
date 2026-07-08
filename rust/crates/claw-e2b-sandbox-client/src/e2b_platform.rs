//! Self-hosted e2bserver platform health (NAS inject contract). Author: kejiqing

use serde::{Deserialize, Serialize};

/// Cached NAS platform state from e2b `GET /health` (not Claw business logic).
#[derive(Debug, Clone)]
pub struct E2bNasPlatform {
    pub ready: bool,
    /// e.g. `10.8.0.8:/mnt/NAS0/nfs-export`
    pub mount_source: String,
    /// e2b host bind source, e.g. `/Volumes/claw-nas` or `/mnt/nas0`
    pub host_mount_root: Option<String>,
    /// `bind` = hostMountRoot/relPath → mountDir; absent = legacy serverAddr NFS.
    pub sandbox_inject: Option<String>,
}

/// One template row from e2bserver `GET /health` → `templates.items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct E2bTemplateEntry {
    #[serde(rename = "templateId")]
    pub template_id: String,
    pub aliases: Vec<String>,
    #[serde(rename = "imagePresent", default)]
    pub image_present: bool,
    pub image: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct E2bHealthTemplatesBlock {
    items: Option<Vec<E2bTemplateEntryRaw>>,
}

#[derive(Debug, Deserialize)]
struct E2bTemplateEntryRaw {
    #[serde(rename = "templateId")]
    template_id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(rename = "imagePresent", default)]
    image_present: bool,
    image: Option<String>,
    arch: Option<String>,
}

impl E2bTemplateEntryRaw {
    fn into_entry(self) -> E2bTemplateEntry {
        E2bTemplateEntry {
            template_id: self.template_id,
            aliases: self.aliases,
            image_present: self.image_present,
            image: self.image,
            arch: self.arch,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct E2bHealthResponse {
    #[allow(dead_code)]
    pub ok: Option<bool>,
    pub nas: Option<E2bNasStatus>,
    templates: Option<E2bHealthTemplatesBlock>,
}

#[derive(Debug, Deserialize)]
pub struct E2bNasStatus {
    pub ready: Option<bool>,
    #[serde(rename = "mountSource")]
    pub mount_source: Option<String>,
    #[serde(rename = "hostMountRoot")]
    pub host_mount_root: Option<String>,
    #[serde(rename = "sandboxInject")]
    pub sandbox_inject: Option<String>,
    /// Platform mount options (informational; inject uses e2b host config).
    #[allow(dead_code)]
    #[serde(rename = "mountOptions")]
    pub mount_options: Option<String>,
}

impl E2bNasStatus {
    fn into_platform(self) -> Option<E2bNasPlatform> {
        let mount_source = self.mount_source?.trim().to_string();
        if mount_source.is_empty() {
            return None;
        }
        Some(E2bNasPlatform {
            ready: self.ready.unwrap_or(false),
            mount_source,
            host_mount_root: self
                .host_mount_root
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            sandbox_inject: self
                .sandbox_inject
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        })
    }
}

impl E2bNasPlatform {
    /// Self-hosted bind inject: gateway sends relPath + mountDir; e2b binds under hostMountRoot.
    #[must_use]
    pub fn uses_host_bind_inject(&self) -> bool {
        if !self.ready {
            return false;
        }
        if self
            .sandbox_inject
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("bind"))
        {
            return self.host_mount_root.is_some();
        }
        self.host_mount_root.is_some()
    }
}

const HEALTH_PATHS: &[&str] = &["/health", "/healthz", "/platform/health"];

/// `mountSource` + relative export path → `nasConfig.mountPoints[].serverAddr`.
#[must_use]
pub fn nas_mount_source_addr(mount_source: &str, rel_path: &str) -> String {
    let base = mount_source.trim().trim_end_matches('/');
    let rel = rel_path.trim_start_matches('/');
    if rel.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{rel}")
    }
}

pub async fn fetch_e2b_platform_nas(
    http: &reqwest::Client,
    api_url: &str,
    api_key: &str,
) -> Result<Option<E2bNasPlatform>, String> {
    let base = api_url.trim_end_matches('/');
    for path in HEALTH_PATHS {
        let url = format!("{base}{path}");
        let resp = match http.get(&url).header("X-API-Key", api_key).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(target: "claw_e2b_sandbox", %url, error = %e, "e2b health GET failed");
                continue;
            }
        };
        let Ok(text) = resp.text().await else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: E2bHealthResponse = match serde_json::from_str(trimmed) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(target: "claw_e2b_sandbox", %url, error = %e, "e2b health JSON parse failed");
                continue;
            }
        };
        let Some(nas) = parsed.nas else {
            continue;
        };
        return Ok(nas.into_platform());
    }
    Ok(None)
}

/// List templates registered on self-hosted e2bserver (`GET /health` → `templates.items`).
pub async fn fetch_e2b_templates(
    http: &reqwest::Client,
    api_url: &str,
    api_key: &str,
) -> Result<Vec<E2bTemplateEntry>, String> {
    let base = api_url.trim_end_matches('/');
    for path in HEALTH_PATHS {
        let url = format!("{base}{path}");
        let resp = http
            .get(&url)
            .header("X-API-Key", api_key)
            .send()
            .await
            .map_err(|e| format!("e2b GET {url}: {e}"))?;
        if !resp.status().is_success() {
            continue;
        }
        let text = resp
            .text()
            .await
            .map_err(|e| format!("e2b GET {url} body: {e}"))?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: E2bHealthResponse =
            serde_json::from_str(trimmed).map_err(|e| format!("e2b GET {url} JSON: {e}"))?;
        let Some(block) = parsed.templates else {
            continue;
        };
        let items = block.items.unwrap_or_default();
        let mut out: Vec<E2bTemplateEntry> = items
            .into_iter()
            .map(E2bTemplateEntryRaw::into_entry)
            .collect();
        out.sort_by(|a, b| {
            b.image_present
                .cmp(&a.image_present)
                .then_with(|| b.template_id.cmp(&a.template_id))
        });
        return Ok(out);
    }
    Err("e2b GET /health missing templates.items (self-hosted e2bserver required)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nas_mount_source_addr_root_and_rel() {
        assert_eq!(
            nas_mount_source_addr("10.8.0.8:/mnt/NAS0/nfs-export", ""),
            "10.8.0.8:/mnt/NAS0/nfs-export"
        );
        assert_eq!(
            nas_mount_source_addr("10.8.0.8:/mnt/NAS0/nfs-export", "proj_1/home"),
            "10.8.0.8:/mnt/NAS0/nfs-export/proj_1/home"
        );
    }

    #[test]
    fn parse_health_nas_block() {
        let raw = r#"{
            "ok": true,
            "nas": {
                "ready": true,
                "mountSource": "10.8.0.8:/mnt/NAS0/nfs-export",
                "hostMountRoot": "/Volumes/claw-nas",
                "sandboxInject": "bind"
            }
        }"#;
        let h: E2bHealthResponse = serde_json::from_str(raw).unwrap();
        let p = h.nas.unwrap().into_platform().unwrap();
        assert!(p.ready);
        assert!(p.uses_host_bind_inject());
        assert_eq!(p.mount_source, "10.8.0.8:/mnt/NAS0/nfs-export");
        assert_eq!(p.host_mount_root.as_deref(), Some("/Volumes/claw-nas"));
    }

    #[test]
    fn host_bind_requires_mount_root() {
        let p = E2bNasPlatform {
            ready: true,
            mount_source: "10.8.0.8:/export".into(),
            host_mount_root: None,
            sandbox_inject: Some("bind".into()),
        };
        assert!(!p.uses_host_bind_inject());
    }

    #[test]
    fn parse_health_templates_block() {
        let raw = r#"{
            "ok": true,
            "templates": {
                "items": [
                    {
                        "templateId": "tpl_abc",
                        "aliases": ["claw-nas-api"],
                        "imagePresent": true,
                        "image": "e2b-tpl-claw-nas-api:ready",
                        "arch": "amd64"
                    }
                ]
            }
        }"#;
        let h: E2bHealthResponse = serde_json::from_str(raw).unwrap();
        let items = h.templates.unwrap().items.unwrap();
        let t = items.into_iter().next().unwrap().into_entry();
        assert_eq!(t.template_id, "tpl_abc");
        assert_eq!(t.aliases, vec!["claw-nas-api"]);
        assert!(t.image_present);
    }
}
