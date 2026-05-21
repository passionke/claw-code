//! Materialize `project_config` rows onto `ds_<id>/home` (rules, CLAUDE.md, skills git sources).
//! Git credentials: env only via `tokenEnv` on each skills source. Author: kejiqing

use std::path::{Component, Path, PathBuf};

use crate::project_tools::parse_allowed_tools_json;
use crate::session_db::ProjectConfigRow;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;

pub const APPLIED_REV_MARKER: &str = ".claw/project_config_applied_rev";
pub const ALLOWED_TOOLS_MARKER: &str = ".claw/project_allowed_tools.json";

#[derive(Debug)]
pub struct ProjectConfigApplyError {
    pub message: String,
}

impl ProjectConfigApplyError {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for ProjectConfigApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProjectConfigApplyError {}

type ApplyResult<T> = Result<T, ProjectConfigApplyError>;

/// Same injection rules as gateway `projects_git_effective_clone_url`; token value from `token_env` only.
pub fn git_effective_clone_url(url: &str, token_env: Option<&str>) -> ApplyResult<String> {
    let token = match token_env {
        Some(name) => Some(read_git_token_env(name)?),
        None => None,
    };
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

fn read_git_token_env(name: &str) -> ApplyResult<String> {
    let v = std::env::var(name)
        .map_err(|_| ProjectConfigApplyError::new(format!("git token env {name} is not set")))?;
    let t = v.trim();
    if t.is_empty() {
        return Err(ProjectConfigApplyError::new(format!(
            "git token env {name} is empty"
        )));
    }
    Ok(t.to_string())
}

async fn git_run(cwd: &Path, args: &[&str]) -> ApplyResult<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["-c", "http.version=HTTP/1.1"]);
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("git failed to start: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(ProjectConfigApplyError::new(format!(
            "git {:?} in {} failed ({}): {stderr}",
            args,
            cwd.display(),
            output.status
        )));
    }
    Ok(stdout)
}

async fn git_run_ok(cwd: &Path, args: &[&str]) -> ApplyResult<()> {
    git_run(cwd, args).await?;
    Ok(())
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

fn safe_rel_under_home(rel: &str) -> ApplyResult<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err(ProjectConfigApplyError::new("relativePath is empty"));
    }
    let path = Path::new(rel);
    for comp in path.components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ProjectConfigApplyError::new(format!(
                    "unsafe relativePath: {rel}"
                )));
            }
            _ => {}
        }
    }
    Ok(path.to_path_buf())
}

async fn write_rules(home: &Path, rules: &Value) -> ApplyResult<()> {
    let Some(items) = rules.as_array() else {
        return Ok(());
    };
    for (i, item) in items.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ProjectConfigApplyError::new(format!("rulesJson[{i}] must be an object"))
        })?;
        let rel = obj
            .get("relativePath")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProjectConfigApplyError::new(format!("rulesJson[{i}] missing relativePath"))
            })?;
        let content = obj.get("content").and_then(Value::as_str).ok_or_else(|| {
            ProjectConfigApplyError::new(format!("rulesJson[{i}] missing content"))
        })?;
        let rel_path = safe_rel_under_home(rel)?;
        let dest = home.join(&rel_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ProjectConfigApplyError::new(format!("create rule parent dir: {e}"))
            })?;
        }
        fs::write(&dest, content).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("write rule {}: {e}", dest.display()))
        })?;
    }
    Ok(())
}

async fn write_claude(work_dir: &Path, text: &str) -> ApplyResult<()> {
    let home = work_dir.join("home");
    fs::create_dir_all(&home)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create home dir: {e}")))?;
    let home_claude = home.join("CLAUDE.md");
    let root_claude = work_dir.join("CLAUDE.md");
    fs::write(&home_claude, text)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("write home/CLAUDE.md: {e}")))?;
    fs::copy(&home_claude, &root_claude)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("mirror CLAUDE.md to ds root: {e}")))?;
    Ok(())
}

async fn copy_tree(src_root: &Path, dst_root: &Path) -> ApplyResult<()> {
    if !fs::metadata(src_root).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src_root.to_path_buf(), dst_root.to_path_buf())];
    while let Some((src_dir, dst_dir)) = stack.pop() {
        fs::create_dir_all(&dst_dir).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("create dir during skills sync: {e}"))
        })?;
        let mut entries = fs::read_dir(&src_dir).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("read dir during skills sync: {e}"))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            ProjectConfigApplyError::new(format!("iterate dir during skills sync: {e}"))
        })? {
            let entry_path = entry.path();
            let dst_path = dst_dir.join(entry.file_name());
            let file_type = entry.file_type().await.map_err(|e| {
                ProjectConfigApplyError::new(format!("file_type during skills sync: {e}"))
            })?;
            if file_type.is_dir() {
                stack.push((entry_path, dst_path));
            } else if file_type.is_file() {
                if let Some(parent) = dst_path.parent() {
                    fs::create_dir_all(parent).await.map_err(|e| {
                        ProjectConfigApplyError::new(format!(
                            "create parent during skills sync: {e}"
                        ))
                    })?;
                }
                fs::copy(&entry_path, &dst_path).await.map_err(|e| {
                    ProjectConfigApplyError::new(format!("copy during skills sync: {e}"))
                })?;
            }
        }
    }
    Ok(())
}

async fn ensure_git_cache_repo(
    cache_dir: &Path,
    git_url: &str,
    git_ref: &str,
    token_env: Option<&str>,
) -> ApplyResult<()> {
    ensure_safe_directory(cache_dir).await;
    let clone_url = git_effective_clone_url(git_url, token_env)?;
    let git_dir = cache_dir.join(".git");
    if fs::metadata(&git_dir).await.is_ok_and(|m| m.is_dir()) {
        git_run_ok(cache_dir, &["remote", "set-url", "origin", &clone_url]).await?;
        git_run_ok(cache_dir, &["fetch", "--depth", "1", "origin", git_ref]).await?;
        git_run_ok(cache_dir, &["checkout", "-f", git_ref]).await?;
        return Ok(());
    }
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("remove stale skills git cache: {e}"))
        })?;
    }
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("create skills git cache parent: {e}"))
        })?;
    }
    let parent = cache_dir
        .parent()
        .ok_or_else(|| ProjectConfigApplyError::new("skills git cache has no parent"))?;
    let leaf = cache_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ProjectConfigApplyError::new("invalid skills git cache dir name"))?;
    if git_run_ok(
        parent,
        &[
            "clone", "--depth", "1", "--branch", git_ref, &clone_url, leaf,
        ],
    )
    .await
    .is_err()
    {
        git_run_ok(parent, &["clone", "--depth", "1", &clone_url, leaf]).await?;
        git_run_ok(cache_dir, &["checkout", "-f", git_ref]).await?;
    }
    Ok(())
}

async fn materialize_skills_sources(work_dir: &Path, sources: &Value) -> ApplyResult<()> {
    let Some(arr) = sources.as_array() else {
        return Ok(());
    };
    if arr.is_empty() {
        return Ok(());
    }
    let home = work_dir.join("home");
    fs::create_dir_all(&home)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create home: {e}")))?;
    let skills_dst = home.join("skills");
    if fs::metadata(&skills_dst).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&skills_dst)
            .await
            .map_err(|e| ProjectConfigApplyError::new(format!("reset home/skills: {e}")))?;
    }
    fs::create_dir_all(&skills_dst)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create home/skills: {e}")))?;

    let cache_root = work_dir.join(".claw/project_config_git_cache");
    fs::create_dir_all(&cache_root)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create git cache root: {e}")))?;

    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ProjectConfigApplyError::new(format!("skillsSourcesJson[{i}] must be an object"))
        })?;
        let git_url = obj
            .get("gitUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ProjectConfigApplyError::new(format!("skillsSourcesJson[{i}] missing gitUrl"))
            })?;
        let git_ref = obj
            .get("gitRef")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("main");
        let path_in_repo = obj
            .get("pathInRepo")
            .and_then(Value::as_str)
            .map_or("", str::trim);
        let target_under_home = obj
            .get("targetUnderHome")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("skills");
        let target_rel = safe_rel_under_home(target_under_home)?;
        let token_env = obj.get("tokenEnv").and_then(Value::as_str).map(str::trim);
        let is_http = git_url.starts_with("https://") || git_url.starts_with("http://");
        let token_env = if is_http {
            Some(token_env.ok_or_else(|| {
                ProjectConfigApplyError::new(format!(
                    "skillsSourcesJson[{i}]: tokenEnv required for HTTP(S) gitUrl"
                ))
            })?)
        } else {
            token_env
        };

        let cache_dir = cache_root.join(format!("src_{i}"));
        ensure_git_cache_repo(&cache_dir, git_url, git_ref, token_env).await?;

        let src_subtree = if path_in_repo.is_empty() {
            cache_dir.clone()
        } else {
            cache_dir.join(path_in_repo)
        };
        let dst_subtree = home.join(&target_rel);
        fs::create_dir_all(&dst_subtree)
            .await
            .map_err(|e| ProjectConfigApplyError::new(format!("create skills target dir: {e}")))?;
        copy_tree(&src_subtree, &dst_subtree).await?;
    }
    Ok(())
}

pub async fn read_applied_content_rev(work_dir: &Path) -> Option<String> {
    let path = work_dir.join(APPLIED_REV_MARKER);
    let buf = fs::read_to_string(path).await.ok()?;
    let t = buf.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Returns `true` if files were materialized.
pub async fn apply_if_needed(
    work_dir: &Path,
    row: &ProjectConfigRow,
    force: bool,
) -> ApplyResult<bool> {
    let applied = read_applied_content_rev(work_dir).await;
    if !force && applied.as_deref() == Some(row.content_rev.as_str()) {
        return Ok(false);
    }
    apply_full(work_dir, row).await?;
    let marker = work_dir.join(APPLIED_REV_MARKER);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("create .claw for rev marker: {e}"))
        })?;
    }
    fs::write(&marker, row.content_rev.as_bytes())
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("write applied rev marker: {e}")))?;
    Ok(true)
}

async fn apply_full(work_dir: &Path, row: &ProjectConfigRow) -> ApplyResult<()> {
    let home = work_dir.join("home");
    fs::create_dir_all(&home)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create home: {e}")))?;
    write_rules(&home, &row.rules_json).await?;
    if let Some(text) = row.claude_md.as_deref().filter(|s| !s.trim().is_empty()) {
        write_claude(work_dir, text).await?;
    }
    materialize_skills_sources(work_dir, &row.skills_sources_json).await?;
    write_allowed_tools_marker(work_dir, row).await?;
    Ok(())
}

async fn write_allowed_tools_marker(work_dir: &Path, row: &ProjectConfigRow) -> ApplyResult<()> {
    let claw_dir = work_dir.join(".claw");
    fs::create_dir_all(&claw_dir)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create .claw: {e}")))?;
    let selected = parse_allowed_tools_json(&row.allowed_tools_json)
        .map_err(|e| ProjectConfigApplyError::new(format!("allowed_tools_json invalid: {e}")))?;
    let body = json!({
        "contentRev": row.content_rev,
        "allowedTools": selected,
    });
    let bytes = serde_json::to_vec_pretty(&body).map_err(|e| {
        ProjectConfigApplyError::new(format!("serialize project_allowed_tools: {e}"))
    })?;
    fs::write(work_dir.join(ALLOWED_TOOLS_MARKER), bytes)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("write project_allowed_tools: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_rejects_parent() {
        assert!(safe_rel_under_home("../x").is_err());
        assert!(safe_rel_under_home("skills/a").is_ok());
    }

    #[test]
    fn git_effective_clone_url_reads_env() {
        std::env::set_var("CLAW_TEST_GIT_TOKEN", "tok");
        let u =
            git_effective_clone_url("https://github.com/org/r.git", Some("CLAW_TEST_GIT_TOKEN"))
                .unwrap();
        assert!(u.contains("x-access-token:tok@"));
        std::env::remove_var("CLAW_TEST_GIT_TOKEN");
    }
}
