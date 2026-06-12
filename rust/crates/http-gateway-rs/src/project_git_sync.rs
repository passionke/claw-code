//! Per-`proj_id` one-way git pull: remote → user work under `home/` (GitHub/GitLab style URL + token).
//! Paths materialized from `project_config` (see `project_config_apply::git_excluded_home_relpaths`) are **not** imported.
//! Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::process::Command;

/// Relative to `proj_<id>/`: manifest of imported paths for pool prompt discovery. Author: kejiqing
pub const GIT_IMPORT_MANIFEST_REL: &str = "home/.claw/git-import-manifest.txt";

const MANIFEST_MAX_LINES: usize = 200;
const MANIFEST_TRUNCATION_PREFIX: &str = "... and ";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGitSync {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "gitUrl", default)]
    pub git_url: String,
    #[serde(rename = "gitRef", default = "default_git_ref")]
    pub git_ref: String,
    #[serde(rename = "gitPatId", default, skip_serializing_if = "Option::is_none")]
    pub git_pat_id: Option<String>,
    #[serde(rename = "gitToken", default, skip_serializing_if = "Option::is_none")]
    pub git_token: Option<String>,
    #[serde(
        rename = "authorName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub author_name: Option<String>,
    #[serde(
        rename = "authorEmail",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub author_email: Option<String>,
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
    /// Legacy push fields (deserialize only). Author: kejiqing
    #[serde(
        rename = "lastPushAtMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_push_at_ms: Option<i64>,
    #[serde(
        rename = "lastPushCommitId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_push_commit_id: Option<String>,
    #[serde(
        rename = "lastPushError",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_push_error: Option<String>,
}

fn default_git_ref() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct GitPullOutcome {
    pub pulled: bool,
    #[serde(rename = "commitId", skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    pub branch: String,
    #[serde(rename = "gitUrl")]
    pub git_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

#[must_use]
pub fn parse_git_sync_json(v: &Value) -> ProjectGitSync {
    if v.is_null() {
        return ProjectGitSync {
            enabled: false,
            git_url: String::new(),
            git_ref: default_git_ref(),
            git_pat_id: None,
            git_token: None,
            author_name: None,
            author_email: None,
            last_pull_at_ms: None,
            last_pull_commit_id: None,
            last_pull_error: None,
            last_push_at_ms: None,
            last_push_commit_id: None,
            last_push_error: None,
        };
    }
    serde_json::from_value(v.clone()).unwrap_or_else(|_| ProjectGitSync {
        enabled: false,
        git_url: String::new(),
        git_ref: default_git_ref(),
        git_pat_id: None,
        git_token: None,
        author_name: None,
        author_email: None,
        last_pull_at_ms: None,
        last_pull_commit_id: None,
        last_pull_error: None,
        last_push_at_ms: None,
        last_push_commit_id: None,
        last_push_error: None,
    })
}

pub fn git_sync_to_json(sync: &ProjectGitSync) -> Value {
    let mut out = sync.clone();
    if out
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        out.git_token = None;
    }
    serde_json::to_value(&out).unwrap_or_else(|_| json!({}))
}

/// Resolve `gitPatId` → inline `git_token` for pull/validate (does not mutate stored JSON). Author: kejiqing
pub fn resolve_git_sync_credentials(
    sync: &ProjectGitSync,
    pat_tokens: &std::collections::BTreeMap<String, String>,
) -> ProjectGitSync {
    let mut out = sync.clone();
    if let Some(id) = out
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(tok) = pat_tokens.get(id) {
            out.git_token = Some(tok.clone());
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
    let url = sync.git_url.trim();
    if url.is_empty() {
        return Err("gitSync.gitUrl is required when gitSync.enabled is true".into());
    }
    let is_http = url.starts_with("https://") || url.starts_with("http://");
    let is_ssh = url.starts_with("git@") || url.starts_with("ssh://");
    if !is_http && !is_ssh {
        return Err(
            "gitSync.gitUrl must be https://, http://, git@, or ssh:// (GitHub/GitLab style)"
                .into(),
        );
    }
    if is_http && url.contains('@') {
        return Err(
            "gitSync.gitUrl must not embed credentials; use gitSync.gitToken for PAT".into(),
        );
    }
    if is_http || is_ssh {
        let token = sync
            .git_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let pat_id = sync
            .git_pat_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if token.is_none() && pat_id.is_none() {
            return Err(
                "gitSync.gitPatId or gitSync.gitToken is required for git pull (HTTP(S) or git@ with PAT)"
                    .into(),
            );
        }
        if token.is_none() && pat_id.is_some() {
            return Err(
                "gitSync.gitPatId is set but no token is available (configure PAT under gateway global settings)"
                    .into(),
            );
        }
    }
    let git_ref = sync.git_ref.trim();
    if git_ref.is_empty() {
        return Err("gitSync.gitRef must be non-empty".into());
    }
    Ok(())
}

pub fn git_sync_list_summary(v: &Value) -> Value {
    let sync = parse_git_sync_json(v);
    let token_set = sync
        .git_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
        || sync
            .git_pat_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
    let configured = sync.enabled && !sync.git_url.trim().is_empty();
    let last_ok = configured
        && sync.last_pull_error.as_deref().unwrap_or("").is_empty()
        && sync.last_pull_at_ms.is_some();
    json!({
        "enabled": sync.enabled,
        "configured": configured,
        "gitUrl": sync.git_url,
        "gitRef": sync.git_ref,
        "gitPatId": sync.git_pat_id,
        "gitTokenSet": token_set,
        "lastPullAtMs": sync.last_pull_at_ms,
        "lastPullCommitId": sync.last_pull_commit_id,
        "lastPullOk": last_ok,
        "lastPullError": sync.last_pull_error,
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
                "gitSync.gitUrl: invalid git@ or ssh:// URL; use https:// with PAT or git@host:group/repo.git",
            )
        })?
    } else {
        trimmed.to_string()
    };
    if is_ssh && token.is_none() {
        return Err(ProjectGitSyncError::new(
            "gitSync.gitPatId is required for git@/ssh:// URLs (gateway converts to HTTPS with PAT; no ssh client in container)",
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
async fn git_run(cwd: &Path, args: &[&str]) -> SyncResult<String> {
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
        return Err(ProjectGitSyncError::new(format!(
            "git {:?} in {} failed ({}): {stderr}",
            args,
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
    )
    .await;
}

/// True when `rel` (under `home/`) is the same as or under a DB-materialized path. Author: kejiqing
#[must_use]
pub fn is_home_rel_db_controlled(rel: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|base| {
        if rel == base {
            return true;
        }
        rel.strip_prefix(base)
            .is_ok_and(|tail| !tail.as_os_str().is_empty())
    })
}

async fn copy_tree(
    src_root: &Path,
    dst_root: &Path,
    excluded_home_relpaths: &[PathBuf],
) -> SyncResult<()> {
    if !fs::metadata(src_root).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src_root.to_path_buf(), dst_root.to_path_buf())];
    while let Some((src_dir, dst_dir)) = stack.pop() {
        fs::create_dir_all(&dst_dir)
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("create dir: {e}")))?;
        let mut entries = fs::read_dir(&src_dir)
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("read dir: {e}")))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("read entry: {e}")))?
        {
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let entry_path = entry.path();
            let rel = entry_path.strip_prefix(src_root).unwrap_or(&entry_path);
            if is_home_rel_db_controlled(rel, excluded_home_relpaths) {
                continue;
            }
            let dst_path = dst_dir.join(&name);
            let file_type = entry
                .file_type()
                .await
                .map_err(|e| ProjectGitSyncError::new(format!("file_type: {e}")))?;
            if file_type.is_dir() {
                stack.push((entry_path, dst_path));
            } else if file_type.is_file() {
                if let Some(parent) = dst_path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| ProjectGitSyncError::new(format!("mkdir parent: {e}")))?;
                }
                fs::copy(&entry_path, &dst_path)
                    .await
                    .map_err(|e| ProjectGitSyncError::new(format!("copy: {e}")))?;
            }
        }
    }
    Ok(())
}

async fn ensure_git_repo(cache_dir: &Path, clone_url: &str, git_ref: &str) -> SyncResult<()> {
    ensure_safe_directory(cache_dir).await;
    let git_dir = cache_dir.join(".git");
    if fs::metadata(&git_dir).await.is_ok_and(|m| m.is_dir()) {
        git_run(cache_dir, &["remote", "set-url", "origin", clone_url]).await?;
        git_run(cache_dir, &["fetch", "--depth", "1", "origin", git_ref]).await?;
        // Shallow cache: `checkout` local branch does not advance; FETCH_HEAD has the new tip. Author: kejiqing
        git_run(cache_dir, &["reset", "--hard", "FETCH_HEAD"]).await?;
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
    )
    .await
    .is_err()
    {
        git_run(parent, &["clone", "--depth", "1", clone_url, leaf]).await?;
        git_run(cache_dir, &["checkout", "-f", git_ref]).await?;
    }
    Ok(())
}

/// Collect relative paths under `home/` that were imported (non-PG), for manifest + prompt. Author: kejiqing
async fn collect_imported_home_relpaths(
    home: &Path,
    excluded: &[PathBuf],
) -> SyncResult<Vec<String>> {
    let mut out = Vec::new();
    if !fs::metadata(home).await.is_ok_and(|m| m.is_dir()) {
        return Ok(out);
    }
    let mut stack = vec![home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("read home dir: {e}")))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("read home entry: {e}")))?
        {
            let path = entry.path();
            let rel = path
                .strip_prefix(home)
                .map_err(|_| ProjectGitSyncError::new("strip home prefix failed"))?;
            if is_home_rel_db_controlled(rel, excluded) {
                continue;
            }
            let ft = entry
                .file_type()
                .await
                .map_err(|e| ProjectGitSyncError::new(format!("file_type: {e}")))?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

async fn write_git_import_manifest(
    work_dir: &Path,
    home: &Path,
    excluded: &[PathBuf],
) -> SyncResult<()> {
    let paths = collect_imported_home_relpaths(home, excluded).await?;
    let manifest_path = work_dir.join(GIT_IMPORT_MANIFEST_REL);
    if paths.is_empty() {
        let _ = fs::remove_file(&manifest_path).await;
        return Ok(());
    }
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ProjectGitSyncError::new(format!("mkdir manifest parent: {e}")))?;
    }
    let mut body = String::new();
    let shown = paths.len().min(MANIFEST_MAX_LINES);
    for rel in paths.iter().take(shown) {
        body.push_str(rel);
        body.push('\n');
    }
    if paths.len() > MANIFEST_MAX_LINES {
        use std::fmt::Write as _;
        let _ = writeln!(body, "... and {} more", paths.len() - MANIFEST_MAX_LINES);
    }
    fs::write(&manifest_path, body.as_bytes())
        .await
        .map_err(|e| ProjectGitSyncError::new(format!("write git import manifest: {e}")))?;
    Ok(())
}

/// One guest file under `/claw_ds/<rel_path>` (sandbox tmpfs materialize). Author: kejiqing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitImportGuestWrite {
    pub rel_path: PathBuf,
    pub bytes: Vec<u8>,
}

fn manifest_line_is_truncation_marker(line: &str) -> bool {
    line.trim().starts_with(MANIFEST_TRUNCATION_PREFIX)
}

fn manifest_buf_is_truncated(buf: &str) -> bool {
    buf.lines().any(manifest_line_is_truncation_marker)
}

/// `git_excluded_home_relpaths` may use `home/…` markers; scan paths are relative to `home/`. Author: kejiqing
fn excluded_relpaths_under_home(excluded: &[PathBuf]) -> Vec<PathBuf> {
    excluded
        .iter()
        .map(|p| match p.strip_prefix("home") {
            Ok(tail) if !tail.as_os_str().is_empty() => tail.to_path_buf(),
            _ => p.clone(),
        })
        .collect()
}

const GIT_IMPORT_MANIFEST_HOME_REL: &str = ".claw/git-import-manifest.txt";

fn safe_home_relpath(rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = Path::new(trimmed);
    if p.is_absolute() {
        return None;
    }
    for c in p.components() {
        if matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return None;
        }
    }
    Some(p.to_path_buf())
}

fn read_file_under_home(home: &Path, rel: &Path) -> SyncResult<Vec<u8>> {
    let full = home.join(rel);
    let home_canon = home
        .canonicalize()
        .map_err(|e| ProjectGitSyncError::new(format!("canonicalize home: {e}")))?;
    let file_canon = full
        .canonicalize()
        .map_err(|e| ProjectGitSyncError::new(format!("read git import {}: {e}", rel.display())))?;
    if !file_canon.starts_with(&home_canon) {
        return Err(ProjectGitSyncError::new(format!(
            "git import path escapes home: {}",
            rel.display()
        )));
    }
    let meta = std::fs::metadata(&file_canon)
        .map_err(|e| ProjectGitSyncError::new(format!("stat {}: {e}", rel.display())))?;
    if !meta.is_file() {
        return Err(ProjectGitSyncError::new(format!(
            "git import path is not a file: {}",
            rel.display()
        )));
    }
    std::fs::read(&file_canon)
        .map_err(|e| ProjectGitSyncError::new(format!("read git import {}: {e}", rel.display())))
}

/// Collect relative paths under `home/` imported from git (sync; for sandbox guest_write). Author: kejiqing
pub fn collect_git_import_home_relpaths(
    work_dir: &Path,
    excluded_home_relpaths: &[PathBuf],
) -> SyncResult<Vec<String>> {
    let excluded = excluded_relpaths_under_home(excluded_home_relpaths);
    let home = work_dir.join("home");
    let manifest_path = work_dir.join(GIT_IMPORT_MANIFEST_REL);
    if let Ok(buf) = std::fs::read_to_string(&manifest_path) {
        if !manifest_buf_is_truncated(&buf) {
            let mut from_manifest = Vec::new();
            for line in buf.lines() {
                let t = line.trim();
                if t.is_empty() || manifest_line_is_truncation_marker(t) {
                    continue;
                }
                let Some(rel) = safe_home_relpath(t) else {
                    continue;
                };
                if is_home_rel_db_controlled(&rel, &excluded) {
                    continue;
                }
                let path = home.join(&rel);
                if path.is_file() {
                    from_manifest.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
            if !from_manifest.is_empty() {
                from_manifest.sort();
                from_manifest.dedup();
                return Ok(from_manifest);
            }
        }
    }
    collect_imported_home_relpaths_sync(&home, &excluded)
}

fn collect_imported_home_relpaths_sync(
    home: &Path,
    excluded: &[PathBuf],
) -> SyncResult<Vec<String>> {
    let mut out = Vec::new();
    if !home.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| ProjectGitSyncError::new(format!("read home dir: {e}")))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| ProjectGitSyncError::new(format!("read home entry: {e}")))?;
            let path = entry.path();
            let rel = path
                .strip_prefix(home)
                .map_err(|_| ProjectGitSyncError::new("strip home prefix failed"))?;
            if is_home_rel_db_controlled(rel, excluded)
                || rel == Path::new(GIT_IMPORT_MANIFEST_HOME_REL)
            {
                continue;
            }
            let ft = entry
                .file_type()
                .map_err(|e| ProjectGitSyncError::new(format!("file_type: {e}")))?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Host `proj_<id>/` git-imported files → bytes for sandbox `/claw_ds` guest_write. Author: kejiqing
pub fn build_guest_git_import_writes(
    work_dir: &Path,
    excluded_home_relpaths: &[PathBuf],
    max_file_bytes: usize,
) -> Result<Vec<GitImportGuestWrite>, String> {
    let home = work_dir.join("home");
    let relpaths = collect_git_import_home_relpaths(work_dir, excluded_home_relpaths)
        .map_err(|e| e.message)?;
    let mut out = Vec::with_capacity(relpaths.len().saturating_add(1));
    for rel in relpaths {
        let rel_path = PathBuf::from(&rel);
        let bytes = read_file_under_home(&home, &rel_path).map_err(|e| e.message)?;
        if bytes.len() > max_file_bytes {
            return Err(format!(
                "git import file {rel} exceeds cap {max_file_bytes} bytes"
            ));
        }
        out.push(GitImportGuestWrite {
            rel_path: PathBuf::from("home").join(&rel),
            bytes,
        });
    }
    let manifest_path = work_dir.join(GIT_IMPORT_MANIFEST_REL);
    if manifest_path.is_file() && !out.is_empty() {
        let bytes = std::fs::read(&manifest_path)
            .map_err(|e| format!("read {GIT_IMPORT_MANIFEST_REL}: {e}"))?;
        if bytes.len() > max_file_bytes {
            return Err(format!(
                "{GIT_IMPORT_MANIFEST_REL} exceeds cap {max_file_bytes} bytes"
            ));
        }
        out.push(GitImportGuestWrite {
            rel_path: PathBuf::from(GIT_IMPORT_MANIFEST_REL),
            bytes,
        });
    }
    Ok(out)
}

/// One-way: copy remote checkout into `home/` (no push). Author: kejiqing
pub async fn pull_home_oneway(
    work_dir: &Path,
    sync: &ProjectGitSync,
    excluded_home_relpaths: &[PathBuf],
) -> SyncResult<GitPullOutcome> {
    if !sync.enabled {
        return Err(ProjectGitSyncError::new("git sync is disabled"));
    }
    let git_url = sync.git_url.trim();
    let git_ref = sync.git_ref.trim();
    let token = sync.git_token.as_deref().map(str::trim);
    let clone_url = effective_clone_url(git_url, token)?;
    let cache_dir = work_dir.join(".claw/project_git_remote");
    let commit_before = if fs::metadata(cache_dir.join(".git"))
        .await
        .is_ok_and(|m| m.is_dir())
    {
        git_run(&cache_dir, &["rev-parse", "HEAD"]).await.ok()
    } else {
        None
    };
    ensure_git_repo(&cache_dir, &clone_url, git_ref).await?;

    let home = work_dir.join("home");
    fs::create_dir_all(&home)
        .await
        .map_err(|e| ProjectGitSyncError::new(format!("create home: {e}")))?;

    let before = collect_imported_home_relpaths(&home, excluded_home_relpaths).await?;
    copy_tree(&cache_dir, &home, excluded_home_relpaths).await?;
    let after = collect_imported_home_relpaths(&home, excluded_home_relpaths).await?;
    write_git_import_manifest(work_dir, &home, excluded_home_relpaths).await?;

    let commit_id = git_run(&cache_dir, &["rev-parse", "HEAD"]).await.ok();
    let pulled = commit_before != commit_id || before != after;
    Ok(GitPullOutcome {
        pulled,
        commit_id,
        branch: git_ref.to_string(),
        git_url: git_url.to_string(),
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn excluded_relpaths_under_home_strips_prefix() {
        let ex = vec![PathBuf::from("home/.claw/language-pipeline.json")];
        let norm = excluded_relpaths_under_home(&ex);
        assert_eq!(norm, vec![PathBuf::from(".claw/language-pipeline.json")]);
    }

    #[test]
    fn collect_git_import_from_manifest_skips_db_controlled() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".claw")).expect("mkdir");
        std::fs::write(home.join("README.md"), "# repo").expect("readme");
        std::fs::write(home.join(".claw/solve-orchestration.json"), "{}").expect("orch");
        std::fs::write(
            root.path().join(GIT_IMPORT_MANIFEST_REL),
            "README.md\n.claw/solve-orchestration.json\n",
        )
        .expect("manifest");
        let excluded = vec![PathBuf::from("home/.claw/solve-orchestration.json")];
        let paths = collect_git_import_home_relpaths(root.path(), &excluded).expect("collect");
        assert_eq!(paths, vec!["README.md".to_string()]);
    }

    #[test]
    fn collect_git_import_falls_back_to_scan_when_manifest_truncated() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".claw")).expect("mkdir");
        std::fs::write(home.join("work.md"), "w").expect("work");
        std::fs::write(home.join("extra.md"), "e").expect("extra");
        std::fs::write(
            root.path().join(GIT_IMPORT_MANIFEST_REL),
            "work.md\n... and 1 more\n",
        )
        .expect("manifest");
        let paths = collect_git_import_home_relpaths(root.path(), &[]).expect("collect");
        assert!(paths.contains(&"work.md".to_string()));
        assert!(paths.contains(&"extra.md".to_string()));
    }

    #[test]
    fn build_guest_git_import_writes_maps_under_claw_ds_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".claw")).expect("mkdir");
        std::fs::write(home.join("README.md"), "body").expect("readme");
        std::fs::write(root.path().join(GIT_IMPORT_MANIFEST_REL), "README.md\n").expect("manifest");
        let writes = build_guest_git_import_writes(root.path(), &[], 1024).expect("writes");
        let paths: Vec<_> = writes.iter().map(|w| w.rel_path.clone()).collect();
        assert!(paths.contains(&PathBuf::from("home/README.md")));
        assert!(paths.contains(&PathBuf::from(GIT_IMPORT_MANIFEST_REL)));
        let readme = writes
            .iter()
            .find(|w| w.rel_path == PathBuf::from("home/README.md"))
            .expect("readme write");
        assert_eq!(readme.bytes, b"body");
    }

    #[test]
    fn build_guest_git_import_writes_empty_when_no_imports() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(root.path().join("home/.claw")).expect("mkdir");
        std::fs::write(root.path().join("home/.claw/language-pipeline.json"), "{}")
            .expect("pg only");
        let writes = build_guest_git_import_writes(
            root.path(),
            &[PathBuf::from(".claw/language-pipeline.json")],
            1024,
        )
        .expect("writes");
        assert!(writes.is_empty());
    }

    #[test]
    fn build_guest_git_import_writes_enforces_size_cap() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".claw")).expect("mkdir");
        std::fs::write(home.join("big.md"), vec![b'x'; 8]).expect("big");
        std::fs::write(root.path().join(GIT_IMPORT_MANIFEST_REL), "big.md\n").expect("manifest");
        let err = build_guest_git_import_writes(root.path(), &[], 4).expect_err("cap");
        assert!(err.contains("exceeds cap"));
    }

    #[test]
    fn list_summary_uses_pull_fields() {
        let v = json!({
            "enabled": true,
            "gitUrl": "https://github.com/o/r.git",
            "gitRef": "main",
            "lastPullAtMs": 1,
            "lastPullCommitId": "abc"
        });
        let s = git_sync_list_summary(&v);
        assert_eq!(s.get("lastPullOk").and_then(|x| x.as_bool()), Some(true));
        assert!(s.get("lastPushAtMs").is_none());
    }
}
