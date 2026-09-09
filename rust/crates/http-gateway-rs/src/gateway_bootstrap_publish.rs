//! Bootstrap: publish e2b core templates from an ACR/CI image tag. Author: kejiqing

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::gateway_bootstrap_deploy::{resolve_bootstrap_repo_root, resolve_deploy_env_file};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapPublishPhase {
    Idle,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapPublishJob {
    pub phase: BootstrapPublishPhase,
    #[serde(rename = "imageTag", skip_serializing_if = "Option::is_none")]
    pub image_tag: Option<String>,
    #[serde(rename = "startedAtMs", skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(rename = "finishedAtMs", skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// CamelCase for Admin UI. Author: kejiqing
    #[serde(rename = "logTail", default, skip_serializing_if = "Vec::is_empty")]
    pub log_tail: Vec<String>,
}

impl Default for BootstrapPublishJob {
    fn default() -> Self {
        Self {
            phase: BootstrapPublishPhase::Idle,
            image_tag: None,
            started_at_ms: None,
            finished_at_ms: None,
            message: None,
            log_tail: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BootstrapPublishTemplatesInput {
    #[serde(rename = "imageTag")]
    pub image_tag: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BootstrapPublishTemplatesResponse {
    pub accepted: bool,
    pub job: BootstrapPublishJob,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BootstrapCiImageTagsResponse {
    #[serde(rename = "registryHost")]
    pub registry_host: String,
    #[serde(rename = "repository")]
    pub repository: String,
    pub tags: Vec<String>,
    #[serde(rename = "suggestedTag", skip_serializing_if = "Option::is_none")]
    pub suggested_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

struct PublishState {
    job: BootstrapPublishJob,
    running: bool,
}

fn publish_state() -> &'static Mutex<PublishState> {
    static STATE: OnceLock<Mutex<PublishState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(PublishState {
            job: BootstrapPublishJob::default(),
            running: false,
        })
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn push_log(job: &mut BootstrapPublishJob, line: String) {
    const MAX: usize = 240;
    job.log_tail.push(line);
    if job.log_tail.len() > MAX {
        let drop_n = job.log_tail.len() - MAX;
        job.log_tail.drain(0..drop_n);
    }
}

/// Default tag suggestion for the wizard (env / GATEWAY_IMAGE). Author: kejiqing
#[must_use]
pub fn suggested_ci_image_tag() -> Option<String> {
    for key in ["CLAW_IMAGE_RELEASE_TAG", "CLAW_BOOTSTRAP_CI_IMAGE_TAG"] {
        if let Ok(raw) = std::env::var(key) {
            let t = raw.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    if let Ok(gw) = std::env::var("GATEWAY_IMAGE") {
        if let Some((_, tag)) = gw.rsplit_once(':') {
            let t = tag.trim();
            if !t.is_empty() && t != "local" {
                return Some(t.to_string());
            }
        }
    }
    if let Ok(gw) = std::env::var("CLAW_GATEWAY_IMAGE_REF") {
        if let Some((_, tag)) = gw.rsplit_once(':') {
            let t = tag.trim();
            if !t.is_empty() && t != "local" {
                return Some(t.to_string());
            }
        }
    }
    None
}

#[must_use]
pub fn current_publish_job() -> BootstrapPublishJob {
    publish_state()
        .lock()
        .map(|g| g.job.clone())
        .unwrap_or_default()
}

fn resolve_publish_script() -> Result<PathBuf, String> {
    let root = resolve_bootstrap_repo_root().ok_or_else(|| {
        "CLAW_REPO_ROOT / deploy .env not found — cannot locate publish script".to_string()
    })?;
    let script = root.join("deploy/e2b/bootstrap-templates-from-ci-tag.sh");
    if !script.is_file() {
        return Err(format!("missing publish script: {}", script.display()));
    }
    Ok(script)
}

/// Start async publish; returns immediately. Poll `current_publish_job` / bootstrap status.
/// Rejects if a job is already running. Author: kejiqing
pub fn start_publish_templates(
    input: &BootstrapPublishTemplatesInput,
) -> Result<BootstrapPublishTemplatesResponse, String> {
    let tag = input.image_tag.trim().to_string();
    if tag.is_empty() {
        return Err("imageTag is required (e.g. release-v1.8.11)".into());
    }
    if tag.contains('/') || tag.contains(' ') || tag.contains('\n') {
        return Err("imageTag must be a bare tag (no registry path or whitespace)".into());
    }
    let script = resolve_publish_script()?;
    let repo = resolve_bootstrap_repo_root().ok_or_else(|| "repo root missing".to_string())?;

    {
        let mut guard = publish_state()
            .lock()
            .map_err(|_| "publish state lock poisoned".to_string())?;
        if guard.running {
            return Ok(BootstrapPublishTemplatesResponse {
                accepted: false,
                job: guard.job.clone(),
                message: Some("publish already running".into()),
            });
        }
        guard.running = true;
        guard.job = BootstrapPublishJob {
            phase: BootstrapPublishPhase::Running,
            image_tag: Some(tag.clone()),
            started_at_ms: Some(now_ms()),
            finished_at_ms: None,
            message: Some(format!("publishing templates from {tag}")),
            log_tail: vec![format!("==> start tag={tag}")],
        };
    }

    let script_arc = Arc::new(script);
    let repo_arc = Arc::new(repo);
    let tag_spawn = tag.clone();
    tokio::spawn(async move {
        let outcome =
            run_publish_script(script_arc.as_path(), repo_arc.as_path(), &tag_spawn).await;
        let Ok(mut guard) = publish_state().lock() else {
            return;
        };
        guard.running = false;
        guard.job.finished_at_ms = Some(now_ms());
        match outcome {
            Ok(()) => {
                guard.job.phase = BootstrapPublishPhase::Succeeded;
                guard.job.message = Some(format!("templates published from {tag_spawn}"));
                push_log(&mut guard.job, "==> succeeded".into());
                info!(
                    target: "claw_gateway_bootstrap",
                    tag = %tag_spawn,
                    "bootstrap template publish succeeded"
                );
            }
            Err(err) => {
                guard.job.phase = BootstrapPublishPhase::Failed;
                guard.job.message = Some(err.clone());
                push_log(&mut guard.job, format!("==> failed: {err}"));
                warn!(
                    target: "claw_gateway_bootstrap",
                    tag = %tag_spawn,
                    error = %err,
                    "bootstrap template publish failed"
                );
            }
        }
    });

    Ok(BootstrapPublishTemplatesResponse {
        accepted: true,
        job: current_publish_job(),
        message: Some("publish started".into()),
    })
}

async fn run_publish_script(
    script: &std::path::Path,
    repo: &std::path::Path,
    tag: &str,
) -> Result<(), String> {
    let art = std::env::var("CLAW_BOOTSTRAP_ARTIFACT_DIR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("/tmp/claw-bootstrap-{tag}"));

    let mut cmd = Command::new("bash");
    cmd.arg(script)
        .arg(tag)
        .current_dir(repo)
        .env("CLAW_IMAGE_RELEASE_TAG", tag)
        .env("CLAW_BOOTSTRAP_ARTIFACT_DIR", &art)
        // Product path: registry extract needs docker config inside gateway. Author: kejiqing
        .env(
            "CLAW_DOCKER_CONFIG",
            std::env::var("CLAW_DOCKER_CONFIG")
                .unwrap_or_else(|_| "/run/claw/claw/docker-config.json".into()),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if let Some(env_file) = resolve_deploy_env_file() {
        cmd.env("CLAW_DEPLOY_ENV_FILE", env_file);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn publish script: {e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let out_task = async {
        let Some(out) = stdout else {
            return;
        };
        let mut lines = BufReader::new(out).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(mut guard) = publish_state().lock() {
                push_log(&mut guard.job, line);
            }
        }
    };
    let err_task = async {
        let Some(err) = stderr else {
            return;
        };
        let mut lines = BufReader::new(err).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(mut guard) = publish_state().lock() {
                push_log(&mut guard.job, line);
            }
        }
    };

    let ((), ()) = tokio::join!(out_task, err_task);
    let status = child
        .wait()
        .await
        .map_err(|e| format!("wait publish script: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "publish script exited {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        ))
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Registry namespace for CI worker images (ACR / GHCR). Author: kejiqing
#[must_use]
pub fn resolve_ci_image_prefix() -> String {
    if let Some(p) = env_nonempty("CLAW_IMAGE_PREFIX").or_else(|| env_nonempty("CLAW_GHCR_PREFIX"))
    {
        return p.trim_end_matches('/').to_string();
    }
    for key in [
        "GATEWAY_IMAGE",
        "CLAW_GATEWAY_IMAGE_REF",
        "CLAW_E2B_WORKER_IMAGE",
    ] {
        if let Some(img) = env_nonempty(key) {
            if let Some((prefix, _)) = img.rsplit_once("/claw-") {
                if prefix.contains('.') {
                    return prefix.to_string();
                }
            }
            if let Some((prefix, _)) = img.rsplit_once("/claw-code") {
                if prefix.contains('.') {
                    return prefix.to_string();
                }
            }
        }
    }
    let backend = env_nonempty("CLAW_IMAGE_REGISTRY")
        .unwrap_or_else(|| "acr".into())
        .to_ascii_lowercase();
    if backend == "ghcr" {
        env_nonempty("CLAW_GHCR_DEFAULT_PREFIX").unwrap_or_else(|| "ghcr.io/passionke".into())
    } else {
        env_nonempty("CLAW_ACR_IMAGE_PREFIX").unwrap_or_else(|| {
            "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke".into()
        })
    }
}

fn split_registry_repo(prefix: &str, image_name: &str) -> Result<(String, String), String> {
    let full = format!("{}/{}", prefix.trim_end_matches('/'), image_name);
    let (host, repo) = full
        .split_once('/')
        .ok_or_else(|| format!("invalid image ref {full}"))?;
    if host.is_empty() || repo.is_empty() {
        return Err(format!("invalid image ref {full}"));
    }
    Ok((host.to_string(), repo.to_string()))
}

fn docker_config_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = env_nonempty("CLAW_DOCKER_CONFIG") {
        out.push(PathBuf::from(p));
    }
    out.push(PathBuf::from("/run/claw/claw/docker-config.json"));
    out.push(PathBuf::from("/run/claw/docker-config.json"));
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join(".docker/config.json"));
    }
    out.push(PathBuf::from("/root/.docker/config.json"));
    out
}

fn basic_auth_from_docker_config(registry_host: &str) -> Option<(String, String)> {
    use base64::Engine;
    for path in docker_config_paths() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let auths = cfg.get("auths")?.as_object()?;
        let entry = auths
            .get(registry_host)
            .or_else(|| auths.get(&format!("https://{registry_host}")))?;
        let auth = entry.get("auth")?.as_str()?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(auth)
            .ok()?;
        let text = String::from_utf8(decoded).ok()?;
        let (user, pass) = text.split_once(':')?;
        if !user.is_empty() && !pass.is_empty() {
            return Some((user.to_string(), pass.to_string()));
        }
    }
    None
}

fn registry_basic_credentials(registry_host: &str) -> Option<(String, String)> {
    if let (Some(u), Some(p)) = (
        env_nonempty("ACR_USERNAME").or_else(|| env_nonempty("ACR_USER")),
        env_nonempty("ACR_PASSWORD").or_else(|| env_nonempty("ACR_PASSWORK")),
    ) {
        return Some((u, p));
    }
    basic_auth_from_docker_config(registry_host)
}

fn parse_bearer_challenge(www: &str) -> Result<(String, String, String), String> {
    let rest = www
        .strip_prefix("Bearer ")
        .ok_or_else(|| format!("unsupported WWW-Authenticate: {www}"))?;
    let mut realm = None;
    let mut service = None;
    let mut scope = None;
    for part in rest.split(',') {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "realm" => realm = Some(v.to_string()),
            "service" => service = Some(v.to_string()),
            "scope" => scope = Some(v.to_string()),
            _ => {}
        }
    }
    Ok((
        realm.ok_or_else(|| "Bearer challenge missing realm".to_string())?,
        service.unwrap_or_default(),
        scope.unwrap_or_default(),
    ))
}

/// Exchange registry Bearer for tags/list. Prefer Basic when creds exist; else anonymous.
/// Author: kejiqing
async fn registry_bearer_token(
    client: &reqwest::Client,
    registry_host: &str,
    repository: &str,
) -> Result<String, String> {
    use base64::Engine;
    let creds = registry_basic_credentials(registry_host);
    let probe = format!("https://{registry_host}/v2/{repository}/tags/list?n=1");
    let resp = client
        .get(&probe)
        .send()
        .await
        .map_err(|e| format!("registry probe: {e}"))?;
    if resp.status().is_success() {
        return Ok(String::new());
    }
    if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!(
            "registry probe HTTP {} for {probe}",
            resp.status().as_u16()
        ));
    }
    let www = resp
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "registry 401 without WWW-Authenticate".to_string())?
        .to_string();
    let (realm, service, scope_from_hdr) = parse_bearer_challenge(&www)?;
    let scope = if scope_from_hdr.is_empty() {
        format!("repository:{repository}:pull")
    } else {
        scope_from_hdr
    };
    let mut url = reqwest::Url::parse(&realm).map_err(|e| format!("token realm url: {e}"))?;
    {
        let mut q = url.query_pairs_mut();
        if !service.is_empty() {
            q.append_pair("service", &service);
        }
        q.append_pair("scope", &scope);
    }
    let mut tok_req = client.get(url);
    let auth_mode = if let Some((user, pass)) = &creds {
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        tok_req = tok_req.header(reqwest::header::AUTHORIZATION, format!("Basic {basic}"));
        "basic"
    } else {
        "anonymous"
    };
    let tok_resp = tok_req
        .send()
        .await
        .map_err(|e| format!("token exchange ({auth_mode}): {e}"))?;
    if !tok_resp.status().is_success() {
        return Err(format!(
            "registry token exchange failed ({auth_mode}) HTTP {} for {registry_host}/{repository}; \
             tags/list needs pull access — set ACR_USERNAME/ACR_PASSWORD (or docker config) if the repo is private, \
             or check registry/network",
            tok_resp.status().as_u16()
        ));
    }
    let body: serde_json::Value = tok_resp
        .json()
        .await
        .map_err(|e| format!("token json: {e}"))?;
    body.get("token")
        .or_else(|| body.get("access_token"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "token response missing token".to_string())
}

fn sort_ci_tags(mut tags: Vec<String>) -> Vec<String> {
    fn rank(tag: &str) -> (u8, Vec<i64>, String) {
        let lower = tag.to_ascii_lowercase();
        let class = if lower.starts_with("release-v") {
            0
        } else if lower.starts_with("branch-") {
            1
        } else if lower.starts_with("dev-") {
            2
        } else {
            3
        };
        let nums: Vec<i64> = lower
            .trim_start_matches("release-v")
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|p| p.parse::<i64>().ok())
            .collect();
        (class, nums, lower)
    }
    tags.sort_by(|a, b| {
        let ra = rank(a);
        let rb = rank(b);
        // class asc, then version nums desc, then name desc
        ra.0.cmp(&rb.0)
            .then_with(|| rb.1.cmp(&ra.1))
            .then_with(|| rb.2.cmp(&ra.2))
    });
    tags
}

/// List CI/ACR tags for claw-gateway-worker (bootstrap dropdown). Author: kejiqing
pub async fn list_ci_image_tags() -> Result<BootstrapCiImageTagsResponse, String> {
    let prefix = resolve_ci_image_prefix();
    let image_name = env_nonempty("CLAW_BOOTSTRAP_CI_IMAGE_NAME")
        .unwrap_or_else(|| "claw-gateway-worker".into());
    let (registry_host, repository) = split_registry_repo(&prefix, &image_name)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let token = registry_bearer_token(&client, &registry_host, &repository).await?;

    let mut tags: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    for _ in 0..20 {
        let mut url = reqwest::Url::parse(&format!(
            "https://{registry_host}/v2/{repository}/tags/list"
        ))
        .map_err(|e| format!("tags url: {e}"))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("n", "100");
            if let Some(ref l) = last {
                q.append_pair("last", l);
            }
        }
        let mut req = client.get(url);
        if !token.is_empty() {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let resp = req.send().await.map_err(|e| format!("tags list: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "tags list HTTP {} for {registry_host}/{repository}",
                resp.status().as_u16()
            ));
        }
        let link = resp
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body: serde_json::Value = resp.json().await.map_err(|e| format!("tags json: {e}"))?;
        let page = body
            .get("tags")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if page.is_empty() {
            break;
        }
        for t in page {
            if let Some(s) = t.as_str() {
                let s = s.trim();
                if !s.is_empty() {
                    tags.push(s.to_string());
                }
            }
        }
        let Some(link_hdr) = link else {
            break;
        };
        // RFC5988: <url>; rel="next"
        let next = link_hdr.split(',').find_map(|part| {
            let part = part.trim();
            if !part.contains("rel=\"next\"") && !part.contains("rel=next") {
                return None;
            }
            let start = part.find('<')? + 1;
            let end = part.find('>')?;
            Some(part[start..end].to_string())
        });
        let Some(next_url) = next else {
            break;
        };
        let parsed = reqwest::Url::parse(&next_url).ok();
        last = parsed
            .as_ref()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "last")
                    .map(|(_, v)| v.to_string())
            })
            .or_else(|| tags.last().cloned());
        if last.is_none() {
            break;
        }
    }

    tags.sort();
    tags.dedup();
    let tags = sort_ci_tags(tags);
    let suggested = suggested_ci_image_tag()
        .filter(|s| tags.iter().any(|t| t == s))
        .or_else(|| tags.first().cloned());

    Ok(BootstrapCiImageTagsResponse {
        registry_host,
        repository,
        tags,
        suggested_tag: suggested,
        message: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_like_tag() {
        let err = start_publish_templates(&BootstrapPublishTemplatesInput {
            image_tag: "reg/foo:bar".into(),
        })
        .expect_err("path tag");
        assert!(err.contains("bare tag"));
    }

    #[test]
    fn sort_puts_release_first() {
        let sorted = sort_ci_tags(vec![
            "latest".into(),
            "release-v1.7.1".into(),
            "branch-foo".into(),
            "release-v1.8.11".into(),
            "release-v1.8.2".into(),
        ]);
        assert_eq!(sorted[0], "release-v1.8.11");
        assert_eq!(sorted[1], "release-v1.8.2");
        assert_eq!(sorted[2], "release-v1.7.1");
        assert!(sorted[3].starts_with("branch-"));
    }

    #[test]
    fn parse_bearer_challenge_ok() {
        let (realm, service, scope) = parse_bearer_challenge(
            "Bearer realm=\"https://dockerauth.example/auth\",service=\"registry\",scope=\"repository:ns/img:pull\"",
        )
        .expect("parse");
        assert!(realm.contains("dockerauth"));
        assert_eq!(service, "registry");
        assert!(scope.contains("pull"));
    }
}
