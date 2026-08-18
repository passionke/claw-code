//! Per-project one-way git pull: remotes → NAS `{cluster}/proj_N/home/<destRel>/`.
//! Gateway clones to a local scratch cache and uploads via nas-api; it does not
//! inventory files for the worker system prompt. Author: kejiqing

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use claw_e2b_sandbox_client::{guest_path_under_claw_ds, proj_home_rel};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::process::Command;

const RESERVED_DEST_RELS: &[&str] = &["project_home_def", ".claw", ".vscode", ".git", "home"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitRemote {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "gitUrl", default)]
    pub git_url: String,
    #[serde(rename = "gitRef", default = "default_git_ref")]
    pub git_ref: String,
    #[serde(rename = "gitPatId", default, skip_serializing_if = "Option::is_none")]
    pub git_pat_id: Option<String>,
    #[serde(rename = "gitToken", default, skip_serializing_if = "Option::is_none")]
    pub git_token: Option<String>,
    #[serde(rename = "destRel", default)]
    pub dest_rel: String,
    #[serde(
        rename = "lastPullAtMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_pull_at_ms: Option<i64>,
    #[serde(
        rename = "lastPullCommitId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_pull_commit_id: Option<String>,
    #[serde(
        rename = "lastPullError",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_pull_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectGitSync {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub remotes: Vec<GitRemote>,
}

fn default_git_ref() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GitRemotePullOutcome {
    pub id: String,
    #[serde(rename = "destRel")]
    pub dest_rel: String,
    #[serde(rename = "gitUrl")]
    pub git_url: String,
    pub branch: String,
    pub pulled: bool,
    #[serde(rename = "commitId", skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GitPullOutcome {
    pub pulled: bool,
    pub remotes: Vec<GitRemotePullOutcome>,
    #[serde(rename = "commitId", skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    pub branch: String,
    #[serde(rename = "gitUrl")]
    pub git_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GitPullOutcome {
    #[must_use]
    pub fn from_remotes(remotes: Vec<GitRemotePullOutcome>) -> Self {
        let pulled = remotes.iter().any(|r| r.pulled);
        let first = remotes.first();
        Self {
            pulled,
            commit_id: first.and_then(|r| r.commit_id.clone()),
            branch: first.map(|r| r.branch.clone()).unwrap_or_default(),
            git_url: first.map(|r| r.git_url.clone()).unwrap_or_default(),
            error: remotes.iter().find_map(|r| r.error.clone()),
            remotes,
        }
    }
}

#[derive(Debug)]
pub struct ProjectGitSyncError {
    pub message: String,
}

impl ProjectGitSyncError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for ProjectGitSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProjectGitSyncError {}

type SyncResult<T> = Result<T, ProjectGitSyncError>;

pub struct GitImportFile {
    pub rel: String,
    pub bytes: Vec<u8>,
}

#[must_use]
pub fn parse_git_sync_json(v: &Value) -> ProjectGitSync {
    if v.is_null() || !v.is_object() {
        return ProjectGitSync::default();
    }
    let enabled = v.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let remotes = if let Some(arr) = v.get("remotes").and_then(Value::as_array) {
        arr.iter()
            .enumerate()
            .filter_map(|(i, item)| parse_remote(item, i))
            .collect()
    } else if v.get("gitUrl").is_some() {
        parse_remote(v, 0).into_iter().collect()
    } else {
        Vec::new()
    };
    ProjectGitSync { enabled, remotes }
}

fn parse_remote(v: &Value, index: usize) -> Option<GitRemote> {
    if !v.is_object() {
        return None;
    }
    let git_url = v
        .get("gitUrl")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let git_ref = v
        .get("gitRef")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main")
        .to_string();
    let git_pat_id = v
        .get("gitPatId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let git_token = v
        .get("gitToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let dest_raw = v
        .get("destRel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| default_dest_rel(&git_url));
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if dest_raw.is_empty() {
                format!("r{}", index + 1)
            } else {
                dest_raw.clone()
            }
        });
    Some(GitRemote {
        id,
        git_url,
        git_ref,
        git_pat_id,
        git_token,
        dest_rel: dest_raw,
        last_pull_at_ms: v.get("lastPullAtMs").and_then(Value::as_i64),
        last_pull_commit_id: v
            .get("lastPullCommitId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        last_pull_error: v
            .get("lastPullError")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
    })
}

pub fn git_sync_to_json(sync: &ProjectGitSync) -> Value {
    let remotes: Vec<Value> = sync.remotes.iter().map(remote_to_json).collect();
    json!({
        "enabled": sync.enabled,
        "remotes": remotes,
    })
}

fn remote_to_json(r: &GitRemote) -> Value {
    let mut o = json!({
        "id": r.id,
        "gitUrl": r.git_url,
        "gitRef": r.git_ref,
        "destRel": r.dest_rel,
    });
    if let Some(obj) = o.as_object_mut() {
        if let Some(id) = r
            .git_pat_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert("gitPatId".into(), json!(id));
        } else if let Some(tok) = r
            .git_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert("gitToken".into(), json!(tok));
        }
        if let Some(ms) = r.last_pull_at_ms {
            obj.insert("lastPullAtMs".into(), json!(ms));
        }
        if let Some(c) = &r.last_pull_commit_id {
            obj.insert("lastPullCommitId".into(), json!(c));
        }
        if let Some(e) = &r.last_pull_error {
            obj.insert("lastPullError".into(), json!(e));
        }
    }
    o
}

/// Resolve each remote's `gitPatId` → inline `git_token` (does not mutate stored JSON). Author: kejiqing
pub fn resolve_git_sync_credentials(
    sync: &ProjectGitSync,
    pat_tokens: &BTreeMap<String, String>,
) -> ProjectGitSync {
    let mut out = sync.clone();
    for remote in &mut out.remotes {
        if let Some(id) = remote
            .git_pat_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(tok) = pat_tokens.get(id) {
                remote.git_token = Some(tok.clone());
            }
        }
    }
    out
}

pub fn validate_git_sync_json(v: &Value) -> Result<(), String> {
    validate_git_sync_resolved(&parse_git_sync_json(v))
}

pub fn validate_git_sync_resolved(sync: &ProjectGitSync) -> Result<(), String> {
    if !sync.enabled {
        return Ok(());
    }
    if sync.remotes.is_empty() {
        return Err("gitSync.remotes must have at least one repository when enabled".into());
    }
    let mut dests = HashSet::new();
    let mut ids = HashSet::new();
    for (i, remote) in sync.remotes.iter().enumerate() {
        validate_remote(remote, i)?;
        if !ids.insert(remote.id.clone()) {
            return Err(format!(
                "gitSync.remotes[{i}].id `{}` is duplicated",
                remote.id
            ));
        }
        if !dests.insert(remote.dest_rel.clone()) {
            return Err(format!(
                "gitSync.remotes[{i}].destRel `{}` is duplicated",
                remote.dest_rel
            ));
        }
    }
    Ok(())
}

fn validate_remote(remote: &GitRemote, index: usize) -> Result<(), String> {
    let prefix = format!("gitSync.remotes[{index}]");
    if remote.id.trim().is_empty() {
        return Err(format!("{prefix}.id is required"));
    }
    validate_dest_rel(&remote.dest_rel).map_err(|e| format!("{prefix}.destRel: {e}"))?;
    let url = remote.git_url.trim();
    if url.is_empty() {
        return Err(format!("{prefix}.gitUrl is required"));
    }
    let is_http = url.starts_with("https://") || url.starts_with("http://");
    let is_ssh = url.starts_with("git@") || url.starts_with("ssh://");
    if !is_http && !is_ssh {
        return Err(format!(
            "{prefix}.gitUrl must be https://, http://, git@, or ssh:// (GitHub/GitLab style)"
        ));
    }
    if is_http && url.contains('@') {
        return Err(format!(
            "{prefix}.gitUrl must not embed credentials; use gitPatId"
        ));
    }
    let token = remote
        .git_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let pat_id = remote
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if token.is_none() && pat_id.is_none() {
        return Err(format!(
            "{prefix}.gitPatId is required for git pull (HTTP(S) or git@ with PAT)"
        ));
    }
    if token.is_none() && pat_id.is_some() {
        return Err(format!(
            "{prefix}.gitPatId is set but no token is available (configure PAT under gateway global settings)"
        ));
    }
    if remote.git_ref.trim().is_empty() {
        return Err(format!("{prefix}.gitRef must be non-empty"));
    }
    Ok(())
}

pub fn validate_dest_rel(dest: &str) -> Result<(), String> {
    let dest = dest.trim();
    if dest.is_empty() {
        return Err("must be non-empty".into());
    }
    if dest.contains('/') || dest.contains('\\') {
        return Err("must be a single path segment".into());
    }
    if dest == "." || dest == ".." {
        return Err("invalid destRel".into());
    }
    if RESERVED_DEST_RELS.iter().any(|r| *r == dest) {
        return Err(format!("`{dest}` is reserved"));
    }
    if !dest
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err("only [A-Za-z0-9._-] allowed".into());
    }
    Ok(())
}

/// NAS dest relative to export root: `{cluster}/proj_N/home/<destRel>`. Author: kejiqing
pub fn git_import_nas_dest(
    cluster_id: &str,
    proj_id: i64,
    dest_rel: &str,
) -> Result<String, String> {
    let cluster = cluster_id.trim();
    if cluster.is_empty() {
        return Err("cluster_id must be non-empty".into());
    }
    validate_dest_rel(dest_rel)?;
    Ok(format!("{}/{dest_rel}", proj_home_rel(cluster, proj_id)))
}

/// Path under project home for nas-api put: `<destRel>/<file>`. Author: kejiqing
pub fn git_import_home_file_rel(dest_rel: &str, rel_in_repo: &str) -> Result<String, String> {
    validate_dest_rel(dest_rel)?;
    let rel = rel_in_repo.trim_start_matches('/').replace('\\', "/");
    if rel.is_empty() {
        return Err("file rel must be non-empty".into());
    }
    if rel
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("file rel must stay inside destRel".into());
    }
    Ok(format!("{dest_rel}/{rel}"))
}

/// Worker-visible path: `/claw_ds/<destRel>/<file>`. Author: kejiqing
pub fn git_import_guest_file(dest_rel: &str, rel_in_repo: &str) -> Result<String, String> {
    Ok(guest_path_under_claw_ds(&git_import_home_file_rel(
        dest_rel,
        rel_in_repo,
    )?))
}

#[must_use]
pub fn default_dest_rel(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let no_git = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let leaf = no_git.rsplit(['/', ':']).next().unwrap_or("repo").trim();
    let sanitized: String = leaf
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('.').trim_matches('-');
    if sanitized.is_empty() || validate_dest_rel(sanitized).is_err() {
        "repo".to_string()
    } else {
        sanitized.to_string()
    }
}

/// Replace PAT plaintext and URL userinfo before logs / lastPullError / API errors. Author: kejiqing
#[must_use]
pub fn redact_git_secret(text: &str, token: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(t) = token.map(str::trim).filter(|s| !s.is_empty()) {
        out = out.replace(t, "***");
    }
    redact_url_userinfo(&out)
}

fn redact_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if let Some(scheme_len) = https_scheme_len(rest) {
            out.push_str(&rest[..scheme_len]);
            i += scheme_len;
            if let Some(at) = text[i..].find('@') {
                let userinfo = &text[i..i + at];
                if userinfo.contains(':')
                    && !userinfo.contains('/')
                    && !userinfo.contains(' ')
                    && !userinfo.is_empty()
                {
                    let user = userinfo.split_once(':').map(|(u, _)| u).unwrap_or(userinfo);
                    out.push_str(user);
                    out.push_str(":***");
                    i += at;
                    continue;
                }
            }
            continue;
        }
        out.push(text[i..].chars().next().unwrap_or('?'));
        i += text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    out
}

fn https_scheme_len(s: &str) -> Option<usize> {
    if s.len() >= 8 && s[..8].eq_ignore_ascii_case("https://") {
        Some(8)
    } else if s.len() >= 7 && s[..7].eq_ignore_ascii_case("http://") {
        Some(7)
    } else {
        None
    }
}

pub fn git_sync_list_summary(v: &Value) -> Value {
    let sync = parse_git_sync_json(v);
    let token_set = sync.remotes.iter().any(|r| {
        r.git_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
            || r.git_pat_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
    });
    let configured = sync.enabled && sync.remotes.iter().any(|r| !r.git_url.trim().is_empty());
    let last_err = sync.remotes.iter().find_map(|r| r.last_pull_error.clone());
    let last_ok = configured
        && last_err.as_deref().unwrap_or("").is_empty()
        && sync.remotes.iter().any(|r| r.last_pull_at_ms.is_some());
    let last_pull_at = sync.remotes.iter().filter_map(|r| r.last_pull_at_ms).max();
    let last_commit = sync
        .remotes
        .iter()
        .find_map(|r| r.last_pull_commit_id.clone());
    json!({
        "enabled": sync.enabled,
        "configured": configured,
        "remoteCount": sync.remotes.len(),
        "gitTokenSet": token_set,
        "lastPullAtMs": last_pull_at,
        "lastPullCommitId": last_commit,
        "lastPullOk": last_ok,
        "lastPullError": last_err,
    })
}

/// Merge PUT payload with stored git_sync_json (preserve PAT + lastPull* per remote id). Author: kejiqing
pub fn merge_git_sync_from_put(incoming: &Value, existing: &Value) -> Value {
    let inc = parse_git_sync_json(incoming);
    let ex = parse_git_sync_json(existing);
    let incoming_has_remotes = incoming.get("remotes").and_then(Value::as_array).is_some()
        || incoming.get("gitUrl").is_some();
    if !incoming_has_remotes {
        let mut keep = ex;
        if incoming.get("enabled").is_some() {
            keep.enabled = inc.enabled;
        }
        return git_sync_to_json(&keep);
    }
    let existing_by_id: BTreeMap<&str, &GitRemote> =
        ex.remotes.iter().map(|r| (r.id.as_str(), r)).collect();
    let incoming_arr = incoming.get("remotes").and_then(Value::as_array);
    let mut merged_remotes = Vec::new();
    for (i, remote) in inc.remotes.iter().enumerate() {
        let mut r = remote.clone();
        let raw = incoming_arr.and_then(|a| a.get(i)).unwrap_or(incoming);
        let prev = existing_by_id.get(r.id.as_str()).copied();
        if raw.get("gitPatId").is_none() {
            r.git_pat_id = prev.and_then(|p| p.git_pat_id.clone());
        } else if raw.get("gitPatId").is_some_and(Value::is_null) {
            r.git_pat_id = None;
        }
        let uses_global_pat = r
            .git_pat_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        if uses_global_pat {
            r.git_token = None;
        } else if r
            .git_token
            .as_deref()
            .map(str::trim)
            .as_ref()
            .is_none_or(|s| s.is_empty())
        {
            r.git_token = prev.and_then(|p| p.git_token.clone());
        }
        if raw.get("lastPullAtMs").is_none() {
            r.last_pull_at_ms = prev.and_then(|p| p.last_pull_at_ms);
            r.last_pull_commit_id = prev.and_then(|p| p.last_pull_commit_id.clone());
            r.last_pull_error = prev.and_then(|p| p.last_pull_error.clone());
        }
        merged_remotes.push(r);
    }
    git_sync_to_json(&ProjectGitSync {
        enabled: inc.enabled,
        remotes: merged_remotes,
    })
}

/// `git@host:org/repo.git` → `https://host/org/repo.git` (GitLab-style SSH). Author: kejiqing
fn ssh_git_url_to_https(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        if host.is_empty() || path.is_empty() {
            return None;
        }
        return Some(format!("https://{host}/{path}"));
    }
    if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.trim_start_matches("git@");
        let (host, path) = rest.split_once('/')?;
        if host.is_empty() || path.is_empty() {
            return None;
        }
        return Some(format!("https://{host}/{path}"));
    }
    None
}

fn https_auth_user_for_host(host_and_path: &str) -> &'static str {
    let host = host_and_path.split('/').next().unwrap_or(host_and_path);
    if host.eq_ignore_ascii_case("github.com") || host.ends_with(".github.com") {
        "x-access-token"
    } else {
        "oauth2"
    }
}

pub fn effective_clone_url(url: &str, token: Option<&str>) -> SyncResult<String> {
    let token = token.map(str::trim).filter(|s| !s.is_empty());
    let trimmed = url.trim();
    let is_ssh = trimmed.starts_with("git@") || trimmed.starts_with("ssh://");
    let base = if is_ssh {
        ssh_git_url_to_https(trimmed).ok_or_else(|| {
            ProjectGitSyncError::new(
                "gitUrl: invalid git@ or ssh:// URL; use https:// with PAT or git@host:group/repo.git",
            )
        })?
    } else {
        trimmed.to_string()
    };
    if is_ssh && token.is_none() {
        return Err(ProjectGitSyncError::new(
            "gitPatId is required for git@/ssh:// URLs (gateway converts to HTTPS with PAT; no ssh client in container)",
        ));
    }
    if let Some(t) = token {
        if let Some(rest) = base.strip_prefix("https://") {
            if !rest.contains('@') {
                let user = https_auth_user_for_host(rest);
                return Ok(format!("https://{user}:{t}@{rest}"));
            }
        }
        if let Some(rest) = base.strip_prefix("http://") {
            if !rest.contains('@') {
                let user = https_auth_user_for_host(rest);
                return Ok(format!("http://{user}:{t}@{rest}"));
            }
        }
    }
    Ok(base)
}

#[allow(clippy::similar_names)]
async fn git_run(cwd: &Path, args: &[&str], secret: Option<&str>) -> SyncResult<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["-c", "http.version=HTTP/1.1"]);
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| ProjectGitSyncError::new(format!("git failed to start: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let args_dbg = redact_git_secret(&format!("{args:?}"), secret);
        let stderr_redacted = redact_git_secret(&stderr, secret);
        return Err(ProjectGitSyncError::new(format!(
            "git {args_dbg} in {} failed ({}): {stderr_redacted}",
            cwd.display(),
            output.status
        )));
    }
    Ok(stdout)
}

async fn ensure_safe_directory(path: &Path) {
    let parent = path.parent().unwrap_or(path);
    let p = path.display().to_string();
    let _ = git_run(
        parent,
        &["config", "--global", "--add", "safe.directory", &p],
        None,
    )
    .await;
}

/// True when `rel` (under repo root) is the same as or under a DB-materialized path. Author: kejiqing
#[must_use]
pub fn is_home_rel_db_controlled(rel: &Path, excluded: &[PathBuf]) -> bool {
    let excluded = excluded_relpaths_under_home(excluded);
    excluded.iter().any(|base| {
        if rel == base {
            return true;
        }
        rel.strip_prefix(base)
            .is_ok_and(|tail| !tail.as_os_str().is_empty())
    })
}

fn excluded_relpaths_under_home(excluded: &[PathBuf]) -> Vec<PathBuf> {
    excluded
        .iter()
        .map(|p| match p.strip_prefix("home") {
            Ok(tail) if !tail.as_os_str().is_empty() => tail.to_path_buf(),
            _ => p.clone(),
        })
        .collect()
}

#[must_use]
pub fn git_import_cache_dir(work_dir: &Path, remote_id: &str) -> PathBuf {
    work_dir.join(".claw/project_git_remote").join(remote_id)
}

async fn ensure_git_repo(
    cache_dir: &Path,
    clone_url: &str,
    git_ref: &str,
    secret: Option<&str>,
) -> SyncResult<()> {
    ensure_safe_directory(cache_dir).await;
    let git_dir = cache_dir.join(".git");
    if fs::metadata(&git_dir).await.is_ok_and(|m| m.is_dir()) {
        git_run(
            cache_dir,
            &["remote", "set-url", "origin", clone_url],
            secret,
        )
        .await?;
        git_run(
            cache_dir,
            &["fetch", "--depth", "1", "origin", git_ref],
            secret,
        )
        .await?;
        git_run(cache_dir, &["reset", "--hard", "FETCH_HEAD"], secret).await?;
        return Ok(());
    }
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("remove stale git cache: {e}")))?;
    }
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("create cache parent: {e}")))?;
    }
    let parent = cache_dir
        .parent()
        .ok_or_else(|| ProjectGitSyncError::new("git cache has no parent"))?;
    let leaf = cache_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ProjectGitSyncError::new("invalid cache dir name"))?;
    if git_run(
        parent,
        &[
            "clone", "--depth", "1", "--branch", git_ref, clone_url, leaf,
        ],
        secret,
    )
    .await
    .is_err()
    {
        git_run(parent, &["clone", "--depth", "1", clone_url, leaf], secret).await?;
        git_run(cache_dir, &["checkout", "-f", git_ref], secret).await?;
    }
    Ok(())
}

/// Walk cloned tree (skip `.git` and PG-controlled paths) → files for nas-api put. Author: kejiqing
pub fn collect_import_files(
    src_root: &Path,
    excluded_home_relpaths: &[PathBuf],
) -> SyncResult<Vec<GitImportFile>> {
    if !src_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut stack = vec![src_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| ProjectGitSyncError::new(format!("read dir: {e}")))?;
        for entry in entries {
            let entry = entry.map_err(|e| ProjectGitSyncError::new(format!("read entry: {e}")))?;
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(src_root).unwrap_or(&path);
            if is_home_rel_db_controlled(rel, excluded_home_relpaths) {
                continue;
            }
            let ft = entry
                .file_type()
                .map_err(|e| ProjectGitSyncError::new(format!("file_type: {e}")))?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let bytes = std::fs::read(&path)
                    .map_err(|e| ProjectGitSyncError::new(format!("read file: {e}")))?;
                out.push(GitImportFile {
                    rel: rel.to_string_lossy().replace('\\', "/"),
                    bytes,
                });
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// Clone/fetch one remote into gateway scratch; return commit + files to upload. Author: kejiqing
pub async fn pull_remote_to_cache(
    cache_dir: &Path,
    remote: &GitRemote,
    excluded_home_relpaths: &[PathBuf],
) -> SyncResult<(GitRemotePullOutcome, Vec<GitImportFile>)> {
    let git_url = remote.git_url.trim();
    let git_ref = remote.git_ref.trim();
    let token = remote.git_token.as_deref().map(str::trim);
    let clone_url = effective_clone_url(git_url, token)?;
    let commit_before = if fs::metadata(cache_dir.join(".git"))
        .await
        .is_ok_and(|m| m.is_dir())
    {
        git_run(cache_dir, &["rev-parse", "HEAD"], token).await.ok()
    } else {
        None
    };
    ensure_git_repo(cache_dir, &clone_url, git_ref, token).await?;
    let commit_id = git_run(cache_dir, &["rev-parse", "HEAD"], token).await.ok();
    let files = collect_import_files(cache_dir, excluded_home_relpaths)?;
    let pulled = commit_before != commit_id;
    Ok((
        GitRemotePullOutcome {
            id: remote.id.clone(),
            dest_rel: remote.dest_rel.clone(),
            git_url: git_url.to_string(),
            branch: git_ref.to_string(),
            pulled,
            commit_id,
            error: None,
        },
        files,
    ))
}

pub fn remote_has_pat_token(remote: &GitRemote, tokens: Option<&BTreeMap<String, String>>) -> bool {
    if remote
        .git_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    let Some(id) = remote
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    tokens.is_some_and(|t| t.contains_key(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_e2b_sandbox_client::{
        guest_path_from_nas_proj_rel, warm_worker_mounts, GUEST_CLAW_DS,
    };
    use std::fs as stdfs;

    #[test]
    fn home_rel_db_controlled_matches_prefixes() {
        let ex = vec![
            PathBuf::from("CLAUDE.md"),
            PathBuf::from("skills"),
            PathBuf::from(".cursor/rules/a.mdc"),
        ];
        assert!(is_home_rel_db_controlled(Path::new("CLAUDE.md"), &ex));
        assert!(is_home_rel_db_controlled(
            Path::new("skills/foo/SKILL.md"),
            &ex
        ));
        assert!(is_home_rel_db_controlled(
            Path::new(".cursor/rules/a.mdc"),
            &ex
        ));
        assert!(!is_home_rel_db_controlled(Path::new("reports/out.md"), &ex));
    }

    #[test]
    fn parse_legacy_single_url_becomes_remote() {
        let v = json!({
            "enabled": true,
            "gitUrl": "https://github.com/org/r.git",
            "gitRef": "main",
            "gitPatId": "pat-1",
            "lastPullCommitId": "abc"
        });
        let s = parse_git_sync_json(&v);
        assert!(s.enabled);
        assert_eq!(s.remotes.len(), 1);
        assert_eq!(s.remotes[0].git_url, "https://github.com/org/r.git");
        assert_eq!(s.remotes[0].dest_rel, "r");
        assert_eq!(s.remotes[0].git_pat_id.as_deref(), Some("pat-1"));
        assert_eq!(s.remotes[0].last_pull_commit_id.as_deref(), Some("abc"));
    }

    #[test]
    fn parse_remotes_array() {
        let v = json!({
            "enabled": true,
            "remotes": [
                {"gitUrl": "https://github.com/org/a.git", "gitPatId": "gh", "destRel": "a"},
                {"gitUrl": "https://gitlab.example/org/b.git", "gitPatId": "gl"}
            ]
        });
        let s = parse_git_sync_json(&v);
        assert_eq!(s.remotes.len(), 2);
        assert_eq!(s.remotes[1].dest_rel, "b");
        assert_eq!(s.remotes[0].id, "a");
    }

    #[test]
    fn validate_requires_token_for_https() {
        let v = json!({
            "enabled": true,
            "gitUrl": "https://github.com/org/r.git",
            "gitRef": "main"
        });
        assert!(validate_git_sync_json(&v).is_err());
        let ok = json!({
            "enabled": true,
            "gitUrl": "https://github.com/org/r.git",
            "gitRef": "main",
            "gitToken": "ghp_test"
        });
        assert!(validate_git_sync_json(&ok).is_ok());
    }

    #[test]
    fn validate_rejects_duplicate_dest() {
        let v = json!({
            "enabled": true,
            "remotes": [
                {"gitUrl": "https://github.com/o/a.git", "gitToken": "t", "destRel": "same"},
                {"gitUrl": "https://gitlab.com/o/b.git", "gitToken": "t", "destRel": "same"}
            ]
        });
        let err = validate_git_sync_json(&v).unwrap_err();
        assert!(err.contains("duplicated"));
    }

    #[test]
    fn dest_rel_reserved() {
        assert!(validate_dest_rel("project_home_def").is_err());
        assert!(validate_dest_rel(".claw").is_err());
        assert!(validate_dest_rel("ok_repo").is_ok());
        assert!(validate_dest_rel("a/b").is_err());
    }

    #[test]
    fn git_import_nas_dest_joins_cluster_proj_home() {
        let dest = git_import_nas_dest("local-dev", 12, "workspace_test").unwrap();
        assert_eq!(dest, "local-dev/proj_12/home/workspace_test");
        assert_eq!(
            dest,
            format!("{}/workspace_test", proj_home_rel("local-dev", 12))
        );
        // nas-api POST /v1/rmdir only accepts this four-segment shape. Author: kejiqing
        let parts: Vec<&str> = dest.split('/').collect();
        assert_eq!(parts, ["local-dev", "proj_12", "home", "workspace_test"]);
        assert!(git_import_nas_dest("local-dev", 12, "project_home_def").is_err());
        assert!(git_import_nas_dest("", 12, "repo").is_err());
        assert!(git_import_nas_dest("local-dev", 12, "a/b").is_err());
    }

    #[test]
    fn resolve_assigns_pat_per_remote() {
        let sync = parse_git_sync_json(&json!({
            "enabled": true,
            "remotes": [
                {"gitUrl": "https://github.com/o/a.git", "gitPatId": "gh", "destRel": "a"},
                {"gitUrl": "https://gitlab.example/o/b.git", "gitPatId": "gl", "destRel": "b"}
            ]
        }));
        let mut tokens = BTreeMap::new();
        tokens.insert("gh".into(), "ghp_aaa".into());
        tokens.insert("gl".into(), "glpat_bbb".into());
        let resolved = resolve_git_sync_credentials(&sync, &tokens);
        assert_eq!(resolved.remotes[0].git_token.as_deref(), Some("ghp_aaa"));
        assert_eq!(resolved.remotes[1].git_token.as_deref(), Some("glpat_bbb"));
    }

    #[test]
    fn effective_clone_url_injects_pat() {
        let u = effective_clone_url("https://github.com/o/r.git", Some("tok")).unwrap();
        assert!(u.contains("x-access-token:tok@"));
    }

    #[test]
    fn effective_clone_url_ssh_to_https_with_pat() {
        let u = effective_clone_url(
            "git@code.sunmi.com:data/workspace_test.git",
            Some("glpat_x"),
        )
        .unwrap();
        assert_eq!(
            u,
            "https://oauth2:glpat_x@code.sunmi.com/data/workspace_test.git"
        );
    }

    #[test]
    fn effective_clone_url_ssh_requires_pat() {
        assert!(effective_clone_url("git@gitlab.com:org/r.git", None).is_err());
    }

    #[test]
    fn redact_hides_pat_and_userinfo() {
        let url = "https://oauth2:glpat_secret@gitlab.example/org/r.git";
        let err = format!("git [\"clone\", \"{url}\"] failed: fatal {url}");
        let out = redact_git_secret(&err, Some("glpat_secret"));
        assert!(!out.contains("glpat_secret"));
        assert!(out.contains("oauth2:***@"));
    }

    #[test]
    fn excluded_relpaths_under_home_strips_prefix() {
        let ex = vec![PathBuf::from("home/.claw/language-pipeline.json")];
        let norm = excluded_relpaths_under_home(&ex);
        assert_eq!(norm, vec![PathBuf::from(".claw/language-pipeline.json")]);
    }

    #[test]
    fn collect_import_skips_git_and_db_paths() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join(".git")).expect("git");
        std::fs::write(root.path().join(".git/HEAD"), "ref").expect("head");
        std::fs::write(root.path().join("README.md"), "# repo").expect("readme");
        std::fs::write(root.path().join("CLAUDE.md"), "nope").expect("claude");
        let files =
            collect_import_files(root.path(), &[PathBuf::from("CLAUDE.md")]).expect("collect");
        let rels: Vec<_> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["README.md"]);
    }

    #[test]
    fn list_summary_uses_pull_fields() {
        let v = json!({
            "enabled": true,
            "gitUrl": "https://github.com/o/r.git",
            "gitRef": "main",
            "gitPatId": "p1",
            "lastPullAtMs": 1,
            "lastPullCommitId": "abc"
        });
        let s = git_sync_list_summary(&v);
        assert_eq!(s.get("lastPullOk").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(s.get("remoteCount").and_then(|x| x.as_u64()), Some(1));
    }

    #[test]
    fn merge_preserves_pat_and_last_pull() {
        let existing = json!({
            "enabled": true,
            "remotes": [{
                "id": "a",
                "gitUrl": "https://github.com/o/a.git",
                "gitRef": "main",
                "gitPatId": "pat-old",
                "destRel": "a",
                "lastPullAtMs": 9,
                "lastPullCommitId": "c1"
            }]
        });
        let incoming = json!({
            "enabled": true,
            "remotes": [{
                "id": "a",
                "gitUrl": "https://github.com/o/a.git",
                "gitRef": "main",
                "destRel": "a"
            }]
        });
        let merged = merge_git_sync_from_put(&incoming, &existing);
        let s = parse_git_sync_json(&merged);
        assert_eq!(s.remotes[0].git_pat_id.as_deref(), Some("pat-old"));
        assert_eq!(s.remotes[0].last_pull_commit_id.as_deref(), Some("c1"));
    }

    #[test]
    fn merge_null_git_pat_id_clears_stored_pat() {
        let existing = json!({
            "enabled": true,
            "remotes": [{
                "id": "a",
                "gitUrl": "https://github.com/o/a.git",
                "gitRef": "main",
                "gitPatId": "pat-old",
                "destRel": "a"
            }]
        });
        let incoming = json!({
            "enabled": true,
            "remotes": [{
                "id": "a",
                "gitUrl": "https://github.com/o/a.git",
                "gitRef": "main",
                "gitPatId": null,
                "destRel": "a"
            }]
        });
        let merged = merge_git_sync_from_put(&incoming, &existing);
        let s = parse_git_sync_json(&merged);
        assert!(s.remotes[0].git_pat_id.is_none());
    }

    #[test]
    fn git_import_home_file_rel_rejects_escape() {
        assert_eq!(
            git_import_home_file_rel("a", "docs/x.md").unwrap(),
            "a/docs/x.md"
        );
        assert!(git_import_home_file_rel("a", "../secret").is_err());
        assert!(git_import_home_file_rel("project_home_def", "x").is_err());
        assert_eq!(
            git_import_guest_file("Hello-World", "README").unwrap(),
            "/claw_ds/Hello-World/README"
        );
    }

    /// NAS write path + worker `/claw_ds` bind: dest files are readable at guest path. Author: kejiqing
    #[test]
    fn git_import_files_readable_after_simulated_claw_ds_bind() {
        let cluster = "local-dev";
        let proj_id = 99180_i64;
        let nas = tempfile::tempdir().expect("nas");
        let excluded = [PathBuf::from("CLAUDE.md")];
        let remotes = [
            ("Hello-World", "README", b"hello from github\n".as_slice()),
            (
                "workspace_test",
                "docs/note.md",
                b"hello from gitlab\n".as_slice(),
            ),
        ];

        for (dest, rel, body) in remotes {
            let src = tempfile::tempdir().expect("clone");
            let file = src.path().join(rel);
            stdfs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
            stdfs::write(&file, body).expect("write clone file");
            stdfs::write(src.path().join("CLAUDE.md"), "pg owned").expect("claude");
            let files = collect_import_files(src.path(), &excluded).expect("collect");
            assert!(
                files.iter().all(|f| f.rel != "CLAUDE.md"),
                "PG-controlled paths must not be uploaded into dest"
            );
            let nas_dest = git_import_nas_dest(cluster, proj_id, dest).expect("nas dest");
            stdfs::create_dir_all(nas.path().join(&nas_dest)).expect("mkdir dest");
            for file in &files {
                let under_home = git_import_home_file_rel(dest, &file.rel).expect("home rel");
                let nas_file = format!("{cluster}/proj_{proj_id}/home/{under_home}");
                assert_eq!(
                    nas_file,
                    format!("{nas_dest}/{}", file.rel.replace('\\', "/"))
                );
                let abs = nas.path().join(&nas_file);
                stdfs::create_dir_all(abs.parent().expect("parent")).expect("mkdir file");
                stdfs::write(&abs, &file.bytes).expect("nas put");
            }
        }

        let mounts = warm_worker_mounts(cluster, proj_id, "wrk_test", true);
        let home = &mounts[0];
        assert_eq!(home.mount_dir, GUEST_CLAW_DS);
        assert!(home.read_only);
        let guest_root = nas.path().join(&home.rel_path);
        assert!(
            guest_root.is_dir(),
            "bind source {} missing",
            guest_root.display()
        );

        for (dest, rel, body) in remotes {
            let nas_file = format!("{}/{dest}/{rel}", proj_home_rel(cluster, proj_id));
            let guest = guest_path_from_nas_proj_rel(cluster, proj_id, &nas_file)
                .expect("nas rel must map into /claw_ds");
            assert_eq!(guest, git_import_guest_file(dest, rel).unwrap());
            assert_eq!(guest, format!("/claw_ds/{dest}/{rel}"));

            let under = nas_file
                .strip_prefix(&format!("{}/", home.rel_path))
                .expect("file must be inside home mount");
            let bound = guest_root.join(under);
            let got = stdfs::read(&bound)
                .unwrap_or_else(|e| panic!("worker read {}: {e}", bound.display()));
            assert_eq!(got, body, "worker bind must expose dest file {dest}/{rel}");
            assert!(!bound.starts_with(guest_root.join("project_home_def")));
        }

        let a = guest_root.join("Hello-World").join("README");
        let b = guest_root.join("workspace_test").join("docs/note.md");
        assert!(a.is_file() && b.is_file());
        assert_ne!(
            a.parent(),
            b.parent(),
            "dest trees must not flatten onto each other"
        );
    }
}
