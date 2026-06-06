//! Materialize `project_config` rows onto `ds_<id>/home` (rules, CLAUDE.md, inline skills from DB).
//! Author: kejiqing

use std::path::{Component, Path, PathBuf};

use crate::project_tools::parse_allowed_tools_json;
use crate::session_db::ProjectConfigRow;
use runtime::{GATEWAY_SYSTEM_PROMPT_SCAFFOLD_REL, GATEWAY_SYSTEM_PROMPT_USER_OVERRIDE_REL};
use serde_json::{json, Value};
use tokio::fs;

pub const APPLIED_REV_MARKER: &str = ".claw/project_config_applied_rev";
pub const ALLOWED_TOOLS_MARKER: &str = ".claw/project_allowed_tools.json";
/// Materialized from `project_config.solve_preflight_json` (DB truth). Author: kejiqing
pub const SOLVE_PREFLIGHT_MARKER: &str = "home/.claw/solve-preflight.json";
/// Materialized from `project_config.solve_orchestration_json` (DB truth). Author: kejiqing
pub const SOLVE_ORCHESTRATION_MARKER: &str = "home/.claw/solve-orchestration.json";

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

async fn write_skills_json(work_dir: &Path, skills: &Value) -> ApplyResult<()> {
    let Some(arr) = skills.as_array() else {
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

    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ProjectConfigApplyError::new(format!("skillsJson[{i}] must be an object"))
        })?;
        let skill_name = obj
            .get("skillName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ProjectConfigApplyError::new(format!("skillsJson[{i}] missing skillName"))
            })?;
        let content = obj
            .get("skillContent")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProjectConfigApplyError::new(format!("skillsJson[{i}] missing skillContent"))
            })?;
        let skill_dir = skills_dst.join(skill_name);
        fs::create_dir_all(&skill_dir).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("create skill dir {}: {e}", skill_dir.display()))
        })?;
        let skill_path = skill_dir.join("SKILL.md");
        fs::write(&skill_path, content.as_bytes())
            .await
            .map_err(|e| {
                ProjectConfigApplyError::new(format!("write {}: {e}", skill_path.display()))
            })?;
    }
    Ok(())
}

/// Relative paths under `home/` materialized from `project_config` — excluded from per-project git push.
/// Author: kejiqing
pub fn git_excluded_home_relpaths(row: &ProjectConfigRow) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if row
        .claude_md
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        out.push(PathBuf::from("CLAUDE.md"));
    }
    if row.skills_json.as_array().is_some_and(|a| !a.is_empty()) {
        out.push(PathBuf::from("skills"));
    }
    if let Some(items) = row.rules_json.as_array() {
        for item in items {
            let rel = item
                .get("relativePath")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(rel) = rel else {
                continue;
            };
            if safe_rel_under_home(rel).is_ok() {
                out.push(PathBuf::from(rel));
            }
        }
    }
    if gateway_solve_turn::project_preflight::has_enabled_solve_preflight(&row.solve_preflight_json)
    {
        out.push(PathBuf::from(SOLVE_PREFLIGHT_MARKER));
    }
    if row
        .solve_orchestration_json
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|k| k != "single_turn")
    {
        out.push(PathBuf::from(SOLVE_ORCHESTRATION_MARKER));
    }
    out
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
    system_prompt_scaffold: &str,
) -> ApplyResult<bool> {
    let applied = read_applied_content_rev(work_dir).await;
    if !force && applied.as_deref() == Some(row.content_rev.as_str()) {
        return Ok(false);
    }
    apply_full(work_dir, row, system_prompt_scaffold).await?;
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

async fn write_system_prompt_sidecars(
    work_dir: &Path,
    row: &ProjectConfigRow,
    system_prompt_scaffold: &str,
) -> ApplyResult<()> {
    let claw_dir = work_dir.join(".claw");
    fs::create_dir_all(&claw_dir)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create .claw: {e}")))?;
    let override_path = work_dir.join(GATEWAY_SYSTEM_PROMPT_USER_OVERRIDE_REL);
    let scaffold_path = work_dir.join(GATEWAY_SYSTEM_PROMPT_SCAFFOLD_REL);
    if let Some(text) = row.claude_md.as_deref().filter(|s| !s.trim().is_empty()) {
        fs::write(&override_path, text.as_bytes())
            .await
            .map_err(|e| ProjectConfigApplyError::new(format!("write user override: {e}")))?;
    } else {
        let _ = fs::remove_file(&override_path).await;
    }
    fs::write(scaffold_path, system_prompt_scaffold.as_bytes())
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("write system scaffold: {e}")))?;
    Ok(())
}

async fn apply_full(
    work_dir: &Path,
    row: &ProjectConfigRow,
    system_prompt_scaffold: &str,
) -> ApplyResult<()> {
    let home = work_dir.join("home");
    fs::create_dir_all(&home)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("create home: {e}")))?;
    write_system_prompt_sidecars(work_dir, row, system_prompt_scaffold).await?;
    write_rules(&home, &row.rules_json).await?;
    if let Some(text) = row.claude_md.as_deref().filter(|s| !s.trim().is_empty()) {
        write_claude(work_dir, text).await?;
    }
    write_skills_json(work_dir, &row.skills_json).await?;
    write_allowed_tools_marker(work_dir, row).await?;
    write_solve_preflight_marker(work_dir, row).await?;
    write_solve_orchestration_marker(work_dir, row).await?;
    link_claw_compat_symlinks(work_dir).await?;
    Ok(())
}

/// Host `ds_<id>/` only: `home/` is `ds_home` canonical; `.claw/skills` / `.cursor/rules` symlinks for local claw discovery. Guest pool uses `.claw` directly (no `home/`). Author: kejiqing
pub async fn link_claw_compat_symlinks(work_dir: &Path) -> ApplyResult<()> {
    link_dir_symlink(
        work_dir,
        &work_dir.join("home/skills"),
        &work_dir.join(".claw/skills"),
        "../home/skills",
    )
    .await?;
    link_dir_symlink(
        work_dir,
        &work_dir.join("home/.cursor/rules"),
        &work_dir.join(".cursor/rules"),
        "../home/.cursor/rules",
    )
    .await?;
    Ok(())
}

/// Remove legacy guest `home/` mirror and host-style `.claw/skills` symlink before project_config writes. Author: kejiqing
#[must_use]
pub fn guest_prepare_worker_native_paths_shell() -> &'static str {
    r#"set -eu
root='/claw_host_root'
rm -rf "$root/home"
if [ -L "$root/.claw/skills" ]; then
  rm -f "$root/.claw/skills"
fi
"#
}

/// After PG materialize: project skills/rules/config are read-only for the solve uid (Admin owns writes). Author: kejiqing
#[must_use]
pub fn guest_lock_project_config_shell() -> &'static str {
    r#"set -eu
root='/claw_host_root'
for rel in \
  CLAUDE.md \
  .claw/settings.json \
  .claw/system_prompt_user_override.md \
  .claw/system_prompt_scaffold.md \
  .claw/project_allowed_tools.json \
  .claw/project_config_applied_rev \
  home/.claw/solve-preflight.json \
  home/.claw/solve-orchestration.json
do
  if [ -e "$root/$rel" ]; then chmod a-w "$root/$rel" 2>/dev/null || true; fi
done
if [ -d "$root/.claw/skills" ]; then
  find "$root/.claw/skills" -exec chmod a-w {} + 2>/dev/null || true
fi
if [ -d "$root/.cursor/rules" ]; then
  find "$root/.cursor/rules" -exec chmod a-w {} + 2>/dev/null || true
fi
"#
}

async fn link_dir_symlink(
    work_dir: &Path,
    src_dir: &Path,
    link_path: &Path,
    relative_target: &str,
) -> ApplyResult<()> {
    if !fs::metadata(src_dir).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("mkdir {}: {e}", parent.display()))
        })?;
    }
    remove_path_for_symlink(link_path).await?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(relative_target, link_path).map_err(|e| {
            ProjectConfigApplyError::new(format!(
                "symlink {} -> {relative_target} (cwd {}): {e}",
                link_path.display(),
                work_dir.display()
            ))
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = (work_dir, src_dir, link_path, relative_target);
    }
    Ok(())
}

async fn remove_path_for_symlink(path: &Path) -> ApplyResult<()> {
    let meta = match fs::symlink_metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(ProjectConfigApplyError::new(format!(
                "stat {} before symlink: {e}",
                path.display()
            )));
        }
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
            .await
            .map_err(|e| ProjectConfigApplyError::new(format!("rm dir {}: {e}", path.display())))?;
    } else {
        fs::remove_file(path)
            .await
            .map_err(|e| ProjectConfigApplyError::new(format!("rm {}: {e}", path.display())))?;
    }
    Ok(())
}

async fn write_solve_orchestration_marker(
    work_dir: &Path,
    row: &ProjectConfigRow,
) -> ApplyResult<()> {
    gateway_solve_turn::project_orchestration::validate_solve_orchestration_json(
        &row.solve_orchestration_json,
    )
    .map_err(ProjectConfigApplyError::new)?;
    let path = work_dir.join(SOLVE_ORCHESTRATION_MARKER);
    let kind = row
        .solve_orchestration_json
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("single_turn");
    if kind == "single_turn" {
        let _ = fs::remove_file(&path).await;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("create {} parent: {e}", path.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(
        &gateway_solve_turn::project_orchestration::materialize_solve_orchestration_json(
            &row.solve_orchestration_json,
        ),
    )
    .map_err(|e| ProjectConfigApplyError::new(format!("serialize solve-orchestration: {e}")))?;
    fs::write(&path, bytes)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("write {}: {e}", path.display())))?;
    Ok(())
}

async fn write_solve_preflight_marker(work_dir: &Path, row: &ProjectConfigRow) -> ApplyResult<()> {
    gateway_solve_turn::project_preflight::validate_solve_preflight_json(&row.solve_preflight_json)
        .map_err(ProjectConfigApplyError::new)?;
    let path = work_dir.join(SOLVE_PREFLIGHT_MARKER);
    if !gateway_solve_turn::project_preflight::has_enabled_solve_preflight(
        &row.solve_preflight_json,
    ) {
        let _ = fs::remove_file(&path).await;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ProjectConfigApplyError::new(format!("create {} parent: {e}", path.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(
        &gateway_solve_turn::project_preflight::materialize_solve_preflight_json(
            &row.solve_preflight_json,
        ),
    )
    .map_err(|e| ProjectConfigApplyError::new(format!("serialize solve-preflight: {e}")))?;
    fs::write(&path, bytes)
        .await
        .map_err(|e| ProjectConfigApplyError::new(format!("write {}: {e}", path.display())))?;
    Ok(())
}

/// One file under pool guest work root (`/claw_host_root/<rel_path>`). Author: kejiqing
#[derive(Debug, Clone)]
pub struct GuestMaterializeWrite {
    pub rel_path: PathBuf,
    pub bytes: Vec<u8>,
}

/// Runtime `settings.json` from Admin `mcp_servers_json` (same as gateway `build_settings`). Author: kejiqing
#[must_use]
pub fn build_settings_json_from_row(row: &ProjectConfigRow) -> Value {
    let mut servers = serde_json::Map::new();
    if let Some(extra) = row.mcp_servers_json.as_object() {
        for (k, v) in extra {
            servers.insert(k.clone(), v.clone());
        }
    }
    json!({
        "mcpServers": servers,
        "auto_hidden_system_prompt": 1
    })
}

/// PG `project_config` → bytes for `materialize_in` (every solve; not host `ds_*` / worker age). Author: kejiqing
pub fn build_guest_materialize_writes(
    row: &ProjectConfigRow,
    system_prompt_scaffold: &str,
) -> ApplyResult<Vec<GuestMaterializeWrite>> {
    let mut out = Vec::new();
    push_system_prompt_sidecars(&mut out, row, system_prompt_scaffold);
    push_rules_writes(&mut out, &row.rules_json)?;
    if let Some(text) = row.claude_md.as_deref().filter(|s| !s.trim().is_empty()) {
        push_claude_writes(&mut out, text);
    }
    push_skills_writes(&mut out, &row.skills_json)?;
    push_allowed_tools_write(&mut out, row)?;
    push_solve_preflight_write(&mut out, row)?;
    push_solve_orchestration_write(&mut out, row)?;
    out.push(GuestMaterializeWrite {
        rel_path: PathBuf::from(APPLIED_REV_MARKER),
        bytes: row.content_rev.as_bytes().to_vec(),
    });
    let settings = build_settings_json_from_row(row);
    let settings_bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|e| ProjectConfigApplyError::new(format!("serialize guest settings.json: {e}")))?;
    out.push(GuestMaterializeWrite {
        rel_path: PathBuf::from(".claw/settings.json"),
        bytes: settings_bytes,
    });
    Ok(out)
}

fn push_write(out: &mut Vec<GuestMaterializeWrite>, rel_path: PathBuf, bytes: Vec<u8>) {
    out.push(GuestMaterializeWrite { rel_path, bytes });
}

fn push_system_prompt_sidecars(
    out: &mut Vec<GuestMaterializeWrite>,
    row: &ProjectConfigRow,
    system_prompt_scaffold: &str,
) {
    if let Some(text) = row.claude_md.as_deref().filter(|s| !s.trim().is_empty()) {
        push_write(
            out,
            PathBuf::from(GATEWAY_SYSTEM_PROMPT_USER_OVERRIDE_REL),
            text.as_bytes().to_vec(),
        );
    }
    push_write(
        out,
        PathBuf::from(GATEWAY_SYSTEM_PROMPT_SCAFFOLD_REL),
        system_prompt_scaffold.as_bytes().to_vec(),
    );
}

fn push_rules_writes(out: &mut Vec<GuestMaterializeWrite>, rules: &Value) -> ApplyResult<()> {
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
        push_write(out, rel_path, content.as_bytes().to_vec());
    }
    Ok(())
}

fn push_claude_writes(out: &mut Vec<GuestMaterializeWrite>, text: &str) {
    push_write(out, PathBuf::from("CLAUDE.md"), text.as_bytes().to_vec());
}

fn push_skills_writes(out: &mut Vec<GuestMaterializeWrite>, skills: &Value) -> ApplyResult<()> {
    let Some(arr) = skills.as_array() else {
        return Ok(());
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ProjectConfigApplyError::new(format!("skillsJson[{i}] must be an object"))
        })?;
        let skill_name = obj
            .get("skillName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ProjectConfigApplyError::new(format!("skillsJson[{i}] missing skillName"))
            })?;
        let content = obj
            .get("skillContent")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProjectConfigApplyError::new(format!("skillsJson[{i}] missing skillContent"))
            })?;
        push_write(
            out,
            PathBuf::from(".claw/skills")
                .join(skill_name)
                .join("SKILL.md"),
            content.as_bytes().to_vec(),
        );
    }
    Ok(())
}

fn push_allowed_tools_write(
    out: &mut Vec<GuestMaterializeWrite>,
    row: &ProjectConfigRow,
) -> ApplyResult<()> {
    let selected = parse_allowed_tools_json(&row.allowed_tools_json)
        .map_err(|e| ProjectConfigApplyError::new(format!("allowed_tools_json invalid: {e}")))?;
    let body = json!({
        "contentRev": row.content_rev,
        "allowedTools": selected,
    });
    let bytes = serde_json::to_vec_pretty(&body).map_err(|e| {
        ProjectConfigApplyError::new(format!("serialize project_allowed_tools: {e}"))
    })?;
    push_write(out, PathBuf::from(ALLOWED_TOOLS_MARKER), bytes);
    Ok(())
}

fn push_solve_preflight_write(
    out: &mut Vec<GuestMaterializeWrite>,
    row: &ProjectConfigRow,
) -> ApplyResult<()> {
    gateway_solve_turn::project_preflight::validate_solve_preflight_json(&row.solve_preflight_json)
        .map_err(ProjectConfigApplyError::new)?;
    if !gateway_solve_turn::project_preflight::has_enabled_solve_preflight(
        &row.solve_preflight_json,
    ) {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(
        &gateway_solve_turn::project_preflight::materialize_solve_preflight_json(
            &row.solve_preflight_json,
        ),
    )
    .map_err(|e| ProjectConfigApplyError::new(format!("serialize solve-preflight: {e}")))?;
    push_write(out, PathBuf::from(SOLVE_PREFLIGHT_MARKER), bytes);
    Ok(())
}

fn push_solve_orchestration_write(
    out: &mut Vec<GuestMaterializeWrite>,
    row: &ProjectConfigRow,
) -> ApplyResult<()> {
    gateway_solve_turn::project_orchestration::validate_solve_orchestration_json(
        &row.solve_orchestration_json,
    )
    .map_err(ProjectConfigApplyError::new)?;
    let kind = row
        .solve_orchestration_json
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("single_turn");
    if kind == "single_turn" {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(
        &gateway_solve_turn::project_orchestration::materialize_solve_orchestration_json(
            &row.solve_orchestration_json,
        ),
    )
    .map_err(|e| ProjectConfigApplyError::new(format!("serialize solve-orchestration: {e}")))?;
    push_write(out, PathBuf::from(SOLVE_ORCHESTRATION_MARKER), bytes);
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
    use crate::session_db::ProjectConfigRow;

    #[test]
    fn git_excluded_paths_follow_db_config() {
        let row = ProjectConfigRow {
            ds_id: 1,
            content_rev: "r".into(),
            stable_content_rev: Some("r".into()),
            draft_open: false,
            updated_at_ms: 0,
            rules_json: json!([{
                "relativePath": ".cursor/rules/safety.mdc",
                "content": "x"
            }]),
            mcp_servers_json: json!({}),
            skills_sources_json: json!([]),
            skills_json: json!([{"skillName": "a", "skillContent": "b"}]),
            allowed_tools_json: json!([]),
            claude_md: Some("# c".into()),
            git_sync_json: json!({}),
            solve_preflight_json: json!({"kind": "none"}),
            solve_orchestration_json: json!({"kind": "single_turn"}),
            extra_session_fields_json: json!([]),
        };
        let ex = git_excluded_home_relpaths(&row);
        assert!(ex.contains(&PathBuf::from("CLAUDE.md")));
        assert!(ex.contains(&PathBuf::from("skills")));
        assert!(ex.contains(&PathBuf::from(".cursor/rules/safety.mdc")));
    }

    #[tokio::test]
    async fn link_claw_compat_symlinks_maps_home_tree_to_claw_paths() {
        let root = std::env::temp_dir().join(format!(
            "claw-compat-link-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("home/skills/desc"))
            .await
            .expect("skills");
        fs::write(root.join("home/skills/desc/SKILL.md"), "body")
            .await
            .expect("skill md");
        fs::create_dir_all(root.join("home/.cursor/rules"))
            .await
            .expect("rules");
        fs::write(root.join("home/.cursor/rules/safety.mdc"), "rule")
            .await
            .expect("rule mdc");
        link_claw_compat_symlinks(&root).await.expect("link");
        let claw_skill = fs::read_link(root.join(".claw/skills"))
            .await
            .expect("claw skills link");
        assert_eq!(claw_skill.to_string_lossy(), "../home/skills");
        let cursor_rules = fs::read_link(root.join(".cursor/rules"))
            .await
            .expect("cursor rules link");
        assert_eq!(cursor_rules.to_string_lossy(), "../home/.cursor/rules");
        let via_claw = fs::read_to_string(root.join(".claw/skills/desc/SKILL.md"))
            .await
            .expect("read via symlink");
        assert_eq!(via_claw, "body");
        let _ = fs::remove_dir_all(root).await;
    }

    #[test]
    fn build_guest_materialize_writes_uses_worker_native_paths() {
        let row = ProjectConfigRow {
            ds_id: 1,
            content_rev: "rev-2".into(),
            stable_content_rev: Some("rev-2".into()),
            draft_open: false,
            updated_at_ms: 0,
            rules_json: json!([{
                "relativePath": ".cursor/rules/safety.mdc",
                "content": "rule body"
            }]),
            mcp_servers_json: json!({}),
            skills_sources_json: json!([]),
            skills_json: json!([{"skillName": "plan", "skillContent": "skill body"}]),
            allowed_tools_json: json!([]),
            claude_md: Some("claude body".into()),
            git_sync_json: json!({}),
            solve_preflight_json: json!({"kind": "none"}),
            solve_orchestration_json: json!({"kind": "single_turn"}),
            extra_session_fields_json: json!([]),
        };
        let writes = build_guest_materialize_writes(&row, "scaffold").expect("writes");
        let paths: Vec<_> = writes.iter().map(|w| w.rel_path.clone()).collect();
        assert!(paths.contains(&PathBuf::from(".claw/skills/plan/SKILL.md")));
        assert!(!paths.iter().any(|p| p.starts_with("home/skills")));
        assert!(paths.contains(&PathBuf::from(".cursor/rules/safety.mdc")));
        assert!(!paths.iter().any(|p| p.starts_with("home/.cursor")));
        assert!(paths.contains(&PathBuf::from("CLAUDE.md")));
    }

    #[tokio::test]
    async fn guest_prepare_shell_drops_home_and_claw_skills_symlink() {
        let root = std::env::temp_dir().join(format!(
            "claw-guest-prepare-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".claw"))
            .await
            .expect("claw dir");
        fs::create_dir_all(root.join("home/skills/plan"))
            .await
            .expect("home skills");
        fs::write(root.join("home/skills/plan/SKILL.md"), "old")
            .await
            .expect("old skill");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("../home/skills", root.join(".claw/skills"))
                .expect("loop symlink");
        }
        let script = guest_prepare_worker_native_paths_shell()
            .replace("/claw_host_root", &root.display().to_string());
        let status = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .status()
            .await
            .expect("run prepare shell");
        assert!(status.success(), "guest prepare shell failed");
        assert!(!root.join("home").exists());
        assert!(!root.join(".claw/skills").exists());
        let _ = fs::remove_dir_all(root).await;
    }

    #[test]
    fn build_guest_materialize_writes_includes_claude_and_settings() {
        let row = ProjectConfigRow {
            ds_id: 1,
            content_rev: "rev-1".into(),
            stable_content_rev: Some("rev-1".into()),
            draft_open: false,
            updated_at_ms: 0,
            rules_json: json!([]),
            mcp_servers_json: json!({"sqlbot": {"url": "http://example"}}),
            skills_sources_json: json!([]),
            skills_json: json!([]),
            allowed_tools_json: json!([]),
            claude_md: Some("BOSS body".into()),
            git_sync_json: json!({}),
            solve_preflight_json: json!({"kind": "none"}),
            solve_orchestration_json: json!({"kind": "single_turn"}),
            extra_session_fields_json: json!([]),
        };
        let writes = build_guest_materialize_writes(&row, "scaffold").expect("writes");
        let paths: Vec<_> = writes
            .iter()
            .map(|w| w.rel_path.to_string_lossy().to_string())
            .collect();
        assert!(paths
            .iter()
            .any(|p| p.contains("system_prompt_user_override")));
        assert!(paths.iter().any(|p| p.ends_with("CLAUDE.md")));
        assert!(!paths.iter().any(|p| p.contains("home/CLAUDE.md")));
        let settings = writes
            .iter()
            .find(|w| w.rel_path == PathBuf::from(".claw/settings.json"))
            .expect("settings");
        let v: Value = serde_json::from_slice(&settings.bytes).expect("settings json");
        assert!(v.get("mcpServers").and_then(|m| m.get("sqlbot")).is_some());
    }

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
