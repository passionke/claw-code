//! Per-`ds_id` one-way git push: user work under `home/` → remote (GitHub/GitLab style URL + token).
//! Paths materialized from `project_config` (see `project_config_apply::git_excluded_home_relpaths`) are **not** pushed.
//! Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::process::Command;

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
    #[serde(rename = "authorName", default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(rename = "authorEmail", default, skip_serializing_if = "Option::is_none")]
    pub author_email: Option<String>,
    #[serde(rename = "lastPushAtMs", default, skip_serializing_if = "Option::is_none")]
    pub last_push_at_ms: Option<i64>,
    #[serde(
        rename = "lastPushCommitId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_push_commit_id: Option<String>,
    #[serde(rename = "lastPushError", default, skip_serializing_if = "Option::is_none")]
    pub last_push_error: Option<String>,
}

fn default_git_ref() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct GitPushOutcome {
    pub pushed: bool,
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

/// Resolve `gitPatId` → inline `git_token` for push/validate (does not mutate stored JSON). Author: kejiqing
pub fn resolve_git_sync_credentials(
    sync: &ProjectGitSync,
    pat_tokens: &std::collections::BTreeMap<String, String>,
) -> ProjectGitSync {
    let mut out = sync.clone();
    if let Some(id) = out.git_pat_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
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
    let sync = sync;
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
            "gitSync.gitUrl must be https://, http://, git@, or ssh:// (GitHub/GitLab style)".into(),
        );
    }
    if is_http && url.contains('@') {
        return Err(
            "gitSync.gitUrl must not embed credentials; use gitSync.gitToken for PAT".into(),
        );
    }
    if is_http {
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
                "gitSync.gitPatId or gitSync.gitToken is required for HTTP(S) gitUrl (use gateway global PAT or inline token)"
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
        && sync.last_push_error.as_deref().unwrap_or("").is_empty()
        && sync.last_push_at_ms.is_some();
    json!({
        "enabled": sync.enabled,
        "configured": configured,
        "gitUrl": sync.git_url,
        "gitRef": sync.git_ref,
        "gitPatId": sync.git_pat_id,
        "gitTokenSet": token_set,
        "lastPushAtMs": sync.last_push_at_ms,
        "lastPushCommitId": sync.last_push_commit_id,
        "lastPushOk": last_ok,
        "lastPushError": sync.last_push_error,
    })
}

pub fn effective_clone_url(url: &str, token: Option<&str>) -> SyncResult<String> {
    let token = token.map(str::trim).filter(|s| !s.is_empty());
    let base = url.trim();
    if let Some(t) = token {
        if let Some(rest) = base.strip_prefix("https://") {
            if !rest.contains('@') {
                return Ok(format!("https://x-access-token:{t}@{rest}"));
            }
        }
        if let Some(rest) = base.strip_prefix("http://") {
            if !rest.contains('@') {
                return Ok(format!("http://x-access-token:{t}@{rest}"));
            }
        }
    }
    Ok(base.to_string())
}

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

async fn git_run_env(cwd: &Path, env_pairs: &[(&str, &str)], args: &[&str]) -> SyncResult<()> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["-c", "http.version=HTTP/1.1"]);
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| ProjectGitSyncError::new(format!("git failed to start: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ProjectGitSyncError::new(format!(
            "git {:?} failed: {stderr}",
            args
        )));
    }
    Ok(())
}

async fn ensure_safe_directory(path: &Path) {
    let parent = path.parent().unwrap_or(path);
    let p = path.display().to_string();
    let _ = git_run(parent, &["config", "--global", "--add", "safe.directory", &p]).await;
}

/// True when `rel` (under `home/`) is the same as or under a DB-materialized path. Author: kejiqing
pub fn is_home_rel_db_controlled(rel: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|base| {
        if rel == base {
            return true;
        }
        rel.strip_prefix(base)
            .map(|tail| !tail.as_os_str().is_empty())
            .unwrap_or(false)
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

async fn clear_worktree_except_git(repo_dir: &Path) -> SyncResult<()> {
    let mut rd = fs::read_dir(repo_dir)
        .await
        .map_err(|e| ProjectGitSyncError::new(format!("read repo dir: {e}")))?;
    while let Some(ent) = rd
        .next_entry()
        .await
        .map_err(|e| ProjectGitSyncError::new(format!("read repo entry: {e}")))?
    {
        if ent.file_name() == ".git" {
            continue;
        }
        let p = ent.path();
        if ent.file_type().await.is_ok_and(|t| t.is_dir()) {
            fs::remove_dir_all(&p)
                .await
                .map_err(|e| ProjectGitSyncError::new(format!("rm dir: {e}")))?;
        } else {
            fs::remove_file(&p)
                .await
                .map_err(|e| ProjectGitSyncError::new(format!("rm file: {e}")))?;
        }
    }
    Ok(())
}

async fn ensure_git_repo(cache_dir: &Path, clone_url: &str, git_ref: &str) -> SyncResult<()> {
    ensure_safe_directory(cache_dir).await;
    let git_dir = cache_dir.join(".git");
    if fs::metadata(&git_dir).await.is_ok_and(|m| m.is_dir()) {
        git_run(cache_dir, &["remote", "set-url", "origin", clone_url]).await?;
        git_run(
            cache_dir,
            &["fetch", "--depth", "1", "origin", git_ref],
        )
        .await?;
        git_run(cache_dir, &["checkout", "-f", git_ref]).await?;
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

const PUSH_MAX_ATTEMPTS: u32 = 8;

/// One-way: copy user work under `home/` into per-project remote (no pull into DB). Author: kejiqing
pub async fn push_home_oneway(
    work_dir: &Path,
    sync: &ProjectGitSync,
    excluded_home_relpaths: &[PathBuf],
    default_author_name: &str,
    default_author_email: &str,
) -> SyncResult<GitPushOutcome> {
    if !sync.enabled {
        return Err(ProjectGitSyncError::new("git sync is disabled"));
    }
    let home = work_dir.join("home");
    if !fs::metadata(&home).await.is_ok_and(|m| m.is_dir()) {
        return Err(ProjectGitSyncError::new(
            "home/ missing; run POST /v1/init or apply project_config first",
        ));
    }
    let git_url = sync.git_url.trim();
    let git_ref = sync.git_ref.trim();
    let token = sync.git_token.as_deref().map(str::trim);
    let clone_url = effective_clone_url(git_url, token)?;
    let cache_dir = work_dir.join(".claw/project_git_remote");
    ensure_git_repo(&cache_dir, &clone_url, git_ref).await?;
    clear_worktree_except_git(&cache_dir).await?;
    copy_tree(&home, &cache_dir, excluded_home_relpaths).await?;

    git_run(&cache_dir, &["add", "-A"]).await?;
    let dirty = git_run(&cache_dir, &["status", "--porcelain"]).await?;
    if dirty.trim().is_empty() {
        let head = git_run(&cache_dir, &["rev-parse", "HEAD"]).await.ok();
        return Ok(GitPushOutcome {
            pushed: false,
            commit_id: head,
            branch: git_ref.to_string(),
            git_url: git_url.to_string(),
            error: None,
        });
    }

    let author_name = sync
        .author_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default_author_name);
    let author_email = sync
        .author_email
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default_author_email);
    let msg = format!("chore(project): sync user work under home ({git_ref})");
    git_run_env(
        &cache_dir,
        &[
            ("GIT_AUTHOR_NAME", author_name),
            ("GIT_AUTHOR_EMAIL", author_email),
            ("GIT_COMMITTER_NAME", author_name),
            ("GIT_COMMITTER_EMAIL", author_email),
        ],
        &["commit", "-m", &msg],
    )
    .await?;

    let mut pushed = false;
    for attempt in 0..PUSH_MAX_ATTEMPTS {
        match git_run(&cache_dir, &["pull", "--rebase", "origin", git_ref]).await {
            Ok(_) => {}
            Err(e) => {
                let _ = git_run(&cache_dir, &["rebase", "--abort"]).await;
                return Err(e);
            }
        }
        match git_run(&cache_dir, &["push", "origin", git_ref]).await {
            Ok(_) => {
                pushed = true;
                break;
            }
            Err(e) => {
                if attempt + 1 < PUSH_MAX_ATTEMPTS {
                    let ms = 40_u64.saturating_mul(1_u64 << attempt.min(6));
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    let commit_id = git_run(&cache_dir, &["rev-parse", "HEAD"]).await.ok();
    Ok(GitPushOutcome {
        pushed,
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
}
