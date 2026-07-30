// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct InitRequest {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    proj_id: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct CreateProjectRequest {
    #[serde(
        rename = "projId",
        alias = "proj_id",
        alias = "dsId",
        alias = "ds_id",
        default
    )]
    proj_id: Option<i64>,
    #[serde(rename = "projectCode", alias = "project_code")]
    project_code: String,
    #[serde(rename = "projectDescription", alias = "project_description", default)]
    project_description: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PatchProjectRequest {
    #[serde(rename = "projectCode", alias = "project_code")]
    project_code: Option<String>,
    #[serde(rename = "projectDescription", alias = "project_description")]
    project_description: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PatchProjectResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "projectCode")]
    project_code: String,
    #[serde(rename = "projectDescription")]
    project_description: String,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct DeleteProjectQuery {
    #[serde(default = "default_true")]
    purge_sessions: bool,
}

pub(crate) fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct ProjectListEntry {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    proj_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
    #[serde(rename = "skillsCountDb")]
    skills_count_db: i64,
    #[serde(rename = "claudeInDb")]
    claude_in_db: bool,
    #[serde(rename = "rulesCountDb")]
    rules_count_db: i64,
    #[serde(rename = "mcpServersCountDb")]
    mcp_servers_count_db: i64,
    #[serde(rename = "workDirPresent")]
    work_dir_present: bool,
    #[serde(rename = "environmentPrepared")]
    environment_prepared: bool,
    #[serde(rename = "claudeOnDisk")]
    claude_on_disk: bool,
    #[serde(rename = "skillsCountDisk")]
    skills_count_disk: u64,
    #[serde(rename = "appliedRev")]
    applied_rev: Option<String>,
    #[serde(rename = "dbSyncedToDisk")]
    db_synced_to_disk: bool,
    /// Per-project one-way git (no PAT in list). Author: kejiqing
    #[serde(rename = "gitSync")]
    #[schema(value_type = Object)]
    git_sync: Value,
    #[serde(rename = "projectCode")]
    project_code: String,
    #[serde(rename = "projectDescription")]
    project_description: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectListResponse {
    projects: Vec<ProjectListEntry>,
    #[serde(rename = "listedAtMs")]
    listed_at_ms: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DeleteProjectResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    deleted: bool,
    #[serde(rename = "purgeSessions")]
    purge_sessions: bool,
    #[serde(rename = "sessionsRemoved")]
    sessions_removed: u64,
    #[serde(rename = "projectConfigRemoved")]
    project_config_removed: bool,
    #[serde(rename = "gitSync", skip_serializing_if = "Option::is_none")]
    git_sync: Option<GitSyncResponse>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct InitResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    initialized: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectGitPullResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    outcome: GitPullOutcome,
    #[serde(rename = "gitSyncJson")]
    #[schema(value_type = Object)]
    git_sync_json: Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GitSyncResponse {
    repo: String,
    branch: String,
    #[serde(rename = "commitId")]
    commit_id: String,
    pushed: bool,
}

pub(crate) fn projects_git_effective_clone_url(url: &str, token: Option<&str>) -> String {
    let base = url.trim();
    if let Some(t) = token.filter(|s| !s.trim().is_empty()) {
        if let Some(rest) = base.strip_prefix("https://") {
            if !rest.contains('@') {
                return format!("https://x-access-token:{t}@{rest}");
            }
        }
        if let Some(rest) = base.strip_prefix("http://") {
            if !rest.contains('@') {
                return format!("http://x-access-token:{t}@{rest}");
            }
        }
    }
    base.to_string()
}

pub(crate) async fn sync_projects_git_remote(cfg: &GatewayConfig, repo_dir: &Path) -> Result<(), ApiError> {
    let git_dir = repo_dir.join(".git");
    if !fs::metadata(&git_dir).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let url =
        projects_git_effective_clone_url(&cfg.projects_git_url, cfg.projects_git_token.as_deref());
    run_git(repo_dir, &["remote", "set-url", "origin", &url])
        .await
        .map(|_| ())
}

pub(crate) async fn claude_instructions_usable(path: &Path) -> bool {
    let meta = match fs::metadata(path).await {
        Ok(m) if m.is_file() => m,
        _ => return false,
    };
    if meta.len() == 0 {
        return false;
    }
    match fs::read_to_string(path).await {
        Ok(text) => !text.trim().is_empty(),
        Err(_) => false,
    }
}

pub(crate) async fn proj_tree_ready(
    work_dir: &Path,
    materialize_row: Option<&session_db::ProjectConfigRow>,
) -> bool {
    let (home_claude, root_claude) = project_claude_paths(work_dir);
    let disk_claude = claude_instructions_usable(&home_claude).await
        || claude_instructions_usable(&root_claude).await;
    let Some(row) = materialize_row else {
        return disk_claude;
    };
    if row
        .claude_md
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return disk_claude;
    }
    let applied = project_config_apply::read_applied_content_rev(work_dir).await;
    applied.as_deref() == Some(row.content_rev.as_str()) && !disk_claude
}

pub(crate) fn proj_environment_not_prepared_error(proj_id: i64, has_project_config: bool) -> ApiError {
    let hint = if has_project_config {
        format!(
            "ds {proj_id} environment not prepared: project_config exists but home/CLAUDE.md is missing or empty; \
             set claudeMd in PUT /v1/project/config/{proj_id}, then POST /v1/init"
        )
    } else {
        format!(
            "ds {proj_id} environment not prepared: no project_config row; \
             POST /v1/projects or PUT /v1/project/config/{proj_id} with non-empty claudeMd, then POST /v1/init"
        )
    };
    ApiError::new(StatusCode::PRECONDITION_FAILED, hint)
}

pub(crate) async fn write_proj_settings_json(state: &AppState, proj_id: i64) -> Result<(), ApiError> {
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let settings = build_settings(state, proj_id).await;
    let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize settings failed: {e}"),
        )
    })?;
    fs::write(work_dir.join(".claw/settings.json"), settings_content)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write settings failed: {e}"),
            )
        })?;
    Ok(())
}

pub(crate) async fn apply_project_config_for_proj(
    state: &AppState,
    proj_id: i64,
    force: bool,
) -> Result<(), ApiError> {
    apply_project_config_for_proj_inner(state, proj_id, force).await
}

pub(crate) async fn apply_project_config_for_proj_inner(
    state: &AppState,
    proj_id: i64,
    force: bool,
) -> Result<(), ApiError> {
    let row = project_config_draft::row_for_materialize(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Ok(());
    };
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds work dir failed: {e}"),
            )
        })?;
    let tree_ready = proj_tree_ready(&work_dir, Some(&row)).await;
    let force_apply = force || !tree_ready;
    let scaffold = gateway_global_settings::load_system_prompt_default(&state.session_db)
        .await
        .map_err(|e| session_db_err(&e))?;
    project_config_apply::apply_if_needed(&work_dir, &row, force_apply, &scaffold)
        .await
        .map_err(|e| map_project_config_apply_err(&e))?;
    write_proj_settings_json(state, proj_id).await?;
    Ok(())
}

pub(crate) async fn try_pull_project_git(
    state: &AppState,
    proj_id: i64,
) -> Result<GitPullOutcome, project_git_sync::ProjectGitSyncError> {
    let row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| project_git_sync::ProjectGitSyncError::new(format!("db: {e}")))?;
    let Some(row) = row else {
        return Err(project_git_sync::ProjectGitSyncError::new(
            "no project_config row",
        ));
    };
    let sync_raw = parse_git_sync_json(&row.git_sync_json);
    if !sync_raw.enabled {
        return Err(project_git_sync::ProjectGitSyncError::new(
            "git sync is disabled",
        ));
    }
    let pat_tokens = gateway_global_settings::load_git_pat_tokens(&state.session_db)
        .await
        .map_err(|e| project_git_sync::ProjectGitSyncError::new(format!("global settings: {e}")))?;
    let sync = project_git_sync::resolve_git_sync_credentials(&sync_raw, &pat_tokens.tokens);
    if let Err(msg) = project_git_sync::validate_git_sync_resolved(&sync) {
        return Err(project_git_sync::ProjectGitSyncError::new(msg));
    }
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    let excluded = project_config_apply::git_excluded_home_relpaths(&row);
    match project_git_sync::pull_home_oneway(&work_dir, &sync, &excluded).await {
        Ok(outcome) => {
            let mut updated = sync;
            updated.last_pull_at_ms = Some(now_ms());
            updated.last_pull_commit_id.clone_from(&outcome.commit_id);
            updated.last_pull_error = None;
            let git_sync_json = git_sync_to_json(&updated);
            persist_git_sync_status(state, &row, &git_sync_json)
                .await
                .map_err(|e| {
                    project_git_sync::ProjectGitSyncError::new(format!("db upsert: {e}"))
                })?;
            Ok(outcome)
        }
        Err(e) => {
            let mut updated = sync;
            updated.last_pull_at_ms = Some(now_ms());
            updated.last_pull_error = Some(e.message.clone());
            let git_sync_json = git_sync_to_json(&updated);
            let _ = persist_git_sync_status(state, &row, &git_sync_json).await;
            Err(e)
        }
    }
}

pub(crate) async fn persist_git_sync_status(
    state: &AppState,
    row: &session_db::ProjectConfigRow,
    git_sync_json: &Value,
) -> Result<(), sqlx::Error> {
    let mut updated = row.clone();
    updated.git_sync_json = git_sync_json.clone();
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &updated,
            &updated.content_rev,
            now_ms(),
            updated.claude_md.as_deref(),
            updated.stable_content_rev.as_deref(),
        ))
        .await
}

pub(crate) async fn sync_proj_from_git_mirror(state: &AppState, proj_id: i64) -> Result<(), ApiError> {
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    let _mirror = state.projects_git_mirror_lock.lock().await;
    let repo_dir = projects_git_mirror_pull_impl(&state.cfg.work_root, state.cfg.as_ref()).await?;
    sync_proj_home_from_repo(&repo_dir, &work_dir, proj_id).await
}

pub(crate) async fn ensure_proj_ready(state: &AppState, proj_id: i64) -> Result<(), ApiError> {
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds work dir failed: {e}"),
            )
        })?;
    let has_project_config = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .is_some();
    apply_project_config_for_proj(state, proj_id, false).await?;
    let materialize_row = project_config_draft::row_for_materialize(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if proj_tree_ready(&work_dir, materialize_row.as_ref()).await {
        return Ok(());
    }
    Err(proj_environment_not_prepared_error(
        proj_id,
        has_project_config,
    ))
}

pub(crate) const PROJECTS_GIT_PUSH_MAX_ATTEMPTS: u32 = 20;

pub(crate) fn projects_git_message_suggests_push_retry(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("non-fast-forward")
        || m.contains("failed to push")
        || m.contains("! [remote rejected]")
        || m.contains("updates were rejected")
        || m.contains("stale info")
}

pub(crate) async fn projects_git_rebase_in_progress(repo_dir: &Path) -> bool {
    fs::metadata(repo_dir.join(".git/rebase-merge"))
        .await
        .is_ok_and(|m| m.is_dir())
        || fs::metadata(repo_dir.join(".git/rebase-apply"))
            .await
            .is_ok_and(|m| m.is_dir())
}

pub(crate) async fn projects_git_abort_rebase_best_effort(repo_dir: &Path) {
    if projects_git_rebase_in_progress(repo_dir).await {
        let _ = run_git(repo_dir, &["rebase", "--abort"]).await;
    }
}

pub(crate) async fn projects_git_try_resolve_rebase_with_workspace(
    repo_dir: &Path,
    projects_git_author: &str,
    src: &Path,
    dst: &Path,
    rel_git_path: &str,
) -> Result<bool, ApiError> {
    if !projects_git_rebase_in_progress(repo_dir).await {
        return Ok(false);
    }
    let unmerged = match run_git(repo_dir, &["diff", "--name-only", "--diff-filter=U"]).await {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };
    let paths: Vec<&str> = unmerged
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if paths.len() == 1 && paths[0] == rel_git_path {
        fs::copy(src, dst).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("re-resolve conflict file from workspace failed: {e}"),
            )
        })?;
        run_git(repo_dir, &["add", rel_git_path]).await?;
        let (git_name, git_email) = parse_projects_git_author(projects_git_author);
        run_git_env(
            repo_dir,
            &[
                ("GIT_AUTHOR_NAME", git_name.as_str()),
                ("GIT_AUTHOR_EMAIL", git_email.as_str()),
                ("GIT_COMMITTER_NAME", git_name.as_str()),
                ("GIT_COMMITTER_EMAIL", git_email.as_str()),
                ("GIT_EDITOR", "true"),
            ],
            &["rebase", "--continue"],
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) async fn ensure_projects_repo_ready(
    work_root: &Path,
    cfg: &GatewayConfig,
) -> Result<PathBuf, ApiError> {
    ensure_projects_git_safe_directory(work_root).await;
    let repo_dir = projects_repo_dir(work_root);
    if fs::metadata(&repo_dir).await.is_ok_and(|m| m.is_dir()) {
        sync_projects_git_remote(cfg, &repo_dir).await?;
        return Ok(repo_dir);
    }
    fs::create_dir_all(work_root).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create work root failed: {e}"),
        )
    })?;
    let clone_url =
        projects_git_effective_clone_url(&cfg.projects_git_url, cfg.projects_git_token.as_deref());
    run_git(
        work_root,
        &[
            "clone",
            "--branch",
            cfg.projects_git_branch.as_str(),
            &clone_url,
            ".claw-code-projects",
        ],
    )
    .await?;
    Ok(repo_dir)
}

pub(crate) async fn pull_projects_repo(repo_dir: &Path, cfg: &GatewayConfig) -> Result<(), ApiError> {
    sync_projects_git_remote(cfg, repo_dir).await?;
    run_git(repo_dir, &["checkout", cfg.projects_git_branch.as_str()]).await?;
    run_git(
        repo_dir,
        &[
            "pull",
            "--ff-only",
            "origin",
            cfg.projects_git_branch.as_str(),
        ],
    )
    .await?;
    Ok(())
}

pub(crate) async fn sync_proj_home_from_repo(
    repo_dir: &Path,
    work_dir: &Path,
    proj_id: i64,
) -> Result<(), ApiError> {
    let proj_repo_home = repo_dir.join(format!("proj_{proj_id}/home"));
    let proj_work_home = work_dir.join("home");
    if fs::metadata(&proj_work_home)
        .await
        .is_ok_and(|m| m.is_dir())
    {
        fs::remove_dir_all(&proj_work_home).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cleanup stale ds home failed: {e}"),
            )
        })?;
    }
    if fs::metadata(&proj_repo_home)
        .await
        .is_ok_and(|m| m.is_dir())
    {
        copy_tree(&proj_repo_home, &proj_work_home).await?;
    } else {
        fs::create_dir_all(&proj_work_home).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create empty ds home failed: {e}"),
            )
        })?;
    }
    let (home_claude, root_claude) = project_claude_paths(work_dir);
    if fs::metadata(&home_claude).await.is_ok_and(|m| m.is_file()) {
        let text = fs::read_to_string(&home_claude).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read home CLAUDE.md for mirror failed: {e}"),
            )
        })?;
        fs::write(&root_claude, &text).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("mirror home CLAUDE.md to root failed: {e}"),
            )
        })?;
    }
    Ok(())
}

pub(crate) async fn projects_git_mirror_pull_impl(
    work_root: &Path,
    cfg: &GatewayConfig,
) -> Result<PathBuf, ApiError> {
    let repo_dir = ensure_projects_repo_ready(work_root, cfg).await?;
    pull_projects_repo(&repo_dir, cfg).await?;
    Ok(repo_dir)
}

pub(crate) async fn projects_git_mirror_copy_commit_push_impl(
    cfg: &GatewayConfig,
    work_root: &Path,
    repo_dir: &Path,
    proj_id: i64,
    rel_path_under_proj: &Path,
    commit_message: &str,
) -> Result<GitSyncResponse, ApiError> {
    let work_dir = proj_work_dir(work_root, proj_id);
    let src = work_dir.join(rel_path_under_proj);
    let proj_root_in_repo = repo_dir.join(format!("proj_{proj_id}"));
    let dst = proj_root_in_repo.join(rel_path_under_proj);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create repo parent dir failed: {e}"),
            )
        })?;
    }
    fs::copy(&src, &dst).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("copy file into git repo failed: {e}"),
        )
    })?;
    let rel_git_path = format!(
        "proj_{proj_id}/{}",
        normalize_rel_for_git(rel_path_under_proj)
    );
    run_git(repo_dir, &["add", &rel_git_path]).await?;
    let dirty = run_git(repo_dir, &["status", "--porcelain", "--", &rel_git_path]).await?;
    let mut pushed = false;
    if !dirty.trim().is_empty() {
        sync_projects_git_remote(cfg, repo_dir).await?;
        let (git_name, git_email) = parse_projects_git_author(cfg.projects_git_author.as_str());
        run_git_env(
            repo_dir,
            &[
                ("GIT_AUTHOR_NAME", git_name.as_str()),
                ("GIT_AUTHOR_EMAIL", git_email.as_str()),
                ("GIT_COMMITTER_NAME", git_name.as_str()),
                ("GIT_COMMITTER_EMAIL", git_email.as_str()),
            ],
            &[
                "commit",
                "--author",
                cfg.projects_git_author.as_str(),
                "-m",
                commit_message,
            ],
        )
        .await?;

        let branch = cfg.projects_git_branch.as_str();
        for attempt in 0..PROJECTS_GIT_PUSH_MAX_ATTEMPTS {
            sync_projects_git_remote(cfg, repo_dir).await?;

            match run_git(repo_dir, &["pull", "--rebase", "origin", branch]).await {
                Ok(_) => {}
                Err(e) => {
                    let detail = e.detail();
                    if projects_git_rebase_in_progress(repo_dir).await {
                        if projects_git_try_resolve_rebase_with_workspace(
                            repo_dir,
                            cfg.projects_git_author.as_str(),
                            &src,
                            &dst,
                            &rel_git_path,
                        )
                        .await?
                        {
                            continue;
                        }
                        projects_git_abort_rebase_best_effort(repo_dir).await;
                        return Err(ApiError::new(
                            StatusCode::CONFLICT,
                            format!(
                                "projects git rebase conflict (multiple writers or overlapping paths): {detail}"
                            ),
                        ));
                    }
                    return Err(e);
                }
            }

            match run_git(repo_dir, &["push", "origin", branch]).await {
                Ok(_) => {
                    pushed = true;
                    break;
                }
                Err(e) => {
                    let detail = e.detail();
                    if projects_git_message_suggests_push_retry(detail)
                        && attempt + 1 < PROJECTS_GIT_PUSH_MAX_ATTEMPTS
                    {
                        let ms = 40_u64.saturating_mul(1_u64 << attempt.min(8));
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        if !pushed {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "projects git push exhausted retries (remote busy or concurrent writers)",
            ));
        }
    }
    let commit_id = run_git(repo_dir, &["rev-parse", "HEAD"]).await?;
    Ok(GitSyncResponse {
        repo: cfg.projects_git_url.clone(),
        branch: cfg.projects_git_branch.clone(),
        commit_id,
        pushed,
    })
}

pub(crate) async fn run_startup_project_config_apply(state: &AppState) {
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "project_config_startup_apply",
        "materializing project_config rows to ds workspaces before accepting traffic"
    );
    match tick_project_config_apply_poll(state).await {
        Ok(()) => info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "project_config_startup_apply",
            "startup project_config apply completed"
        ),
        Err(e) => warn!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "project_config_startup_apply",
            status = %e.status,
            error = %e.detail(),
            "startup project_config apply failed; gateway will still listen"
        ),
    }
}

pub(crate) async fn project_config_poll_loop(state: AppState, interval_secs: u64) {
    let start = tokio::time::Instant::now() + Duration::from_secs(interval_secs);
    let mut ticker = tokio::time::interval_at(start, Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match tick_project_config_apply_poll(&state).await {
            Ok(()) => {}
            Err(e) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "project_config_poll",
                    phase = "tick_failed",
                    status = %e.status,
                    error = %e.detail(),
                    "periodic project_config materialize failed"
                );
            }
        }
    }
}

pub(crate) async fn list_proj_ids_in_projects_mirror(repo_dir: &Path) -> Result<Vec<i64>, ApiError> {
    let mut out = Vec::new();
    let mut rd = fs::read_dir(repo_dir).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list projects mirror failed: {e}"),
        )
    })?;
    while let Some(ent) = rd.next_entry().await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read projects mirror entry failed: {e}"),
        )
    })? {
        let name = ent.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix("proj_") else {
            continue;
        };
        if let Ok(id) = rest.parse::<i64>() {
            if id >= 1 {
                out.push(id);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

pub(crate) async fn tick_project_config_apply_poll(state: &AppState) -> Result<(), ApiError> {
    let ids = state
        .session_db
        .list_project_config_proj_ids()
        .await
        .map_err(|e| session_db_err(&e))?;
    for proj_id in ids {
        let lock = get_proj_lock(state, proj_id).await;
        let Ok(_guard) = lock.try_lock() else {
            continue;
        };
        let cfg_row = state
            .session_db
            .get_project_config(proj_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        let Some(row) = cfg_row else {
            continue;
        };
        let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
        fs::create_dir_all(work_dir.join(".claw"))
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("create ds work dir failed: {e}"),
                )
            })?;
        let applied = project_config_apply::read_applied_content_rev(&work_dir).await;
        if applied.as_deref() != Some(row.content_rev.as_str()) {
            apply_project_config_for_proj(state, proj_id, false).await?;
        }
    }
    Ok(())
}

pub(crate) fn default_project_claude_md(proj_id: i64) -> String {
    format!(
        "# proj_{proj_id}\n\nAuthor: kejiqing\n\nEdit in admin **CLAUDE.md** or `PUT /v1/project/config/{proj_id}`.\n"
    )
}

pub(crate) async fn collect_known_proj_ids(state: &AppState) -> Result<Vec<i64>, ApiError> {
    state
        .session_db
        .list_project_config_proj_ids()
        .await
        .map_err(|e| session_db_err(&e))
}

pub(crate) async fn resolve_create_proj_id(state: &AppState, requested: Option<i64>) -> Result<i64, ApiError> {
    if let Some(id) = requested {
        if id < 1 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "projId must be >= 1",
            ));
        }
        return Ok(id);
    }
    let ids = collect_known_proj_ids(state).await?;
    Ok(ids.last().copied().unwrap_or(0) + 1)
}

pub(crate) const MAX_PROJECT_CODE_LEN: usize = 64;

pub(crate) const MAX_PROJECT_DESCRIPTION_LEN: usize = 500;

pub(crate) fn normalize_project_code(raw: &str) -> Result<String, ApiError> {
    let code = raw.trim();
    if code.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projectCode cannot be empty",
        ));
    }
    if code.len() > MAX_PROJECT_CODE_LEN {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("projectCode must be at most {MAX_PROJECT_CODE_LEN} characters"),
        ));
    }
    if !code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projectCode only allows [a-zA-Z0-9_-]",
        ));
    }
    if !code
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projectCode must start with a letter or digit",
        ));
    }
    Ok(code.to_string())
}

pub(crate) fn normalize_project_description(raw: Option<&str>) -> Result<String, ApiError> {
    let desc = raw.unwrap_or("").trim().to_string();
    if desc.len() > MAX_PROJECT_DESCRIPTION_LEN {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("projectDescription must be at most {MAX_PROJECT_DESCRIPTION_LEN} characters"),
        ));
    }
    Ok(desc)
}

pub(crate) async fn ensure_project_code_available(
    state: &AppState,
    code: &str,
    exclude_proj_id: Option<i64>,
) -> Result<(), ApiError> {
    let taken = state
        .session_db
        .project_code_taken(code, exclude_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if taken {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("projectCode {code:?} is already in use"),
        ));
    }
    Ok(())
}

pub(crate) async fn project_config_exists(state: &AppState, proj_id: i64) -> Result<bool, ApiError> {
    Ok(state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .is_some())
}

pub(crate) async fn write_file_if_missing(path: &Path, content: &str) -> Result<(), ApiError> {
    if fs::metadata(path).await.is_ok() {
        return Ok(());
    }
    fs::write(path, content).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write {} failed: {e}", path.display()),
        )
    })
}

pub(crate) async fn build_project_list_entry(
    state: &AppState,
    summary: &session_db::ProjectConfigSummary,
) -> ProjectListEntry {
    let proj_id = summary.proj_id;
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    let work_dir_present = fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir());
    let materialize_row = project_config_draft::row_for_materialize(&state.session_db, proj_id)
        .await
        .ok()
        .flatten();
    let environment_prepared =
        work_dir_present && proj_tree_ready(&work_dir, materialize_row.as_ref()).await;
    let (home_claude, _) = project_claude_paths(&work_dir);
    let claude_on_disk = claude_instructions_usable(&home_claude).await;
    let skills_root = work_dir.join("home/skills");
    let skills_count_disk = if fs::metadata(&skills_root).await.is_ok_and(|m| m.is_dir()) {
        count_skill_dirs(&skills_root).await
    } else {
        0
    };
    let applied_rev = project_config_apply::read_applied_content_rev(&work_dir).await;
    let stable_rev = summary
        .stable_content_rev
        .as_deref()
        .filter(|r| !project_config_draft::is_draft_content_rev(r))
        .unwrap_or(summary.content_rev.as_str());
    let db_synced_to_disk = applied_rev.as_deref() == Some(stable_rev);
    ProjectListEntry {
        proj_id: summary.proj_id,
        content_rev: stable_rev.to_string(),
        draft_open: summary.draft_open,
        updated_at_ms: summary.updated_at_ms,
        skills_count_db: summary.skills_count_db,
        claude_in_db: summary.claude_in_db,
        rules_count_db: summary.rules_count_db,
        mcp_servers_count_db: summary.mcp_servers_count_db,
        work_dir_present,
        environment_prepared,
        claude_on_disk,
        skills_count_disk,
        applied_rev,
        db_synced_to_disk,
        git_sync: git_sync_list_summary(&summary.git_sync_json),
        project_code: summary.project_code.clone(),
        project_description: summary.project_description.clone(),
    }
}

pub(crate) async fn scaffold_proj_workspace(work_dir: &Path, proj_id: i64) -> Result<(), ApiError> {
    let claude = default_project_claude_md(proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "create {}/.claw failed: {e}. \
                     Gateway must write under CLAW_WORK_ROOT on the host bind mount \
                     (pool worker mounts the same tree read-only inside the container only). \
                     Check owner matches CLAW_WORKER_UID; try: ./deploy/stack/gateway.sh fix-workspace",
                    work_dir.display()
                ),
            )
        })?;
    fs::create_dir_all(work_dir.join("home/skills"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create home/skills failed: {e}"),
            )
        })?;
    write_file_if_missing(&work_dir.join("home/CLAUDE.md"), &claude).await?;
    write_file_if_missing(&work_dir.join("CLAUDE.md"), &claude).await?;
    Ok(())
}

pub(crate) async fn projects_git_commit_and_push(
    cfg: &GatewayConfig,
    repo_dir: &Path,
    pathspec: &str,
    commit_message: &str,
) -> Result<GitSyncResponse, ApiError> {
    let dirty = run_git(repo_dir, &["status", "--porcelain", "--", pathspec]).await?;
    let mut pushed = false;
    if !dirty.trim().is_empty() {
        sync_projects_git_remote(cfg, repo_dir).await?;
        let (git_name, git_email) = parse_projects_git_author(cfg.projects_git_author.as_str());
        run_git_env(
            repo_dir,
            &[
                ("GIT_AUTHOR_NAME", git_name.as_str()),
                ("GIT_AUTHOR_EMAIL", git_email.as_str()),
                ("GIT_COMMITTER_NAME", git_name.as_str()),
                ("GIT_COMMITTER_EMAIL", git_email.as_str()),
            ],
            &[
                "commit",
                "--author",
                cfg.projects_git_author.as_str(),
                "-m",
                commit_message,
            ],
        )
        .await?;

        let branch = cfg.projects_git_branch.as_str();
        for attempt in 0..PROJECTS_GIT_PUSH_MAX_ATTEMPTS {
            sync_projects_git_remote(cfg, repo_dir).await?;
            match run_git(repo_dir, &["pull", "--rebase", "origin", branch]).await {
                Ok(_) => {}
                Err(e) => return Err(e),
            }
            match run_git(repo_dir, &["push", "origin", branch]).await {
                Ok(_) => {
                    pushed = true;
                    break;
                }
                Err(e) => {
                    let detail = e.detail();
                    if projects_git_message_suggests_push_retry(detail)
                        && attempt + 1 < PROJECTS_GIT_PUSH_MAX_ATTEMPTS
                    {
                        let ms = 40_u64.saturating_mul(1_u64 << attempt.min(8));
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        if !pushed {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "projects git push exhausted retries (remote busy or concurrent writers)",
            ));
        }
    }
    let commit_id = run_git(repo_dir, &["rev-parse", "HEAD"]).await?;
    Ok(GitSyncResponse {
        repo: cfg.projects_git_url.clone(),
        branch: cfg.projects_git_branch.clone(),
        commit_id,
        pushed,
    })
}

pub(crate) async fn projects_git_push_proj_home_from_workdir(
    cfg: &GatewayConfig,
    work_root: &Path,
    repo_dir: &Path,
    proj_id: i64,
    commit_message: &str,
) -> Result<GitSyncResponse, ApiError> {
    let work_dir = proj_work_dir(work_root, proj_id);
    let proj_root_in_repo = repo_dir.join(format!("proj_{proj_id}"));
    let dst_home = proj_root_in_repo.join("home");
    if fs::metadata(&dst_home).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&dst_home).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cleanup repo ds home failed: {e}"),
            )
        })?;
    }
    copy_tree(&work_dir.join("home"), &dst_home).await?;
    let rel_prefix = format!("proj_{proj_id}/");
    run_git(repo_dir, &["add", &rel_prefix]).await?;
    projects_git_commit_and_push(cfg, repo_dir, &rel_prefix, commit_message).await
}

pub(crate) async fn projects_git_remove_proj_tree(
    cfg: &GatewayConfig,
    repo_dir: &Path,
    proj_id: i64,
) -> Result<Option<GitSyncResponse>, ApiError> {
    let rel = format!("proj_{proj_id}");
    if !fs::metadata(repo_dir.join(&rel))
        .await
        .is_ok_and(|m| m.is_dir())
    {
        return Ok(None);
    }
    run_git(repo_dir, &["rm", "-rf", "--ignore-unmatch", &rel]).await?;
    let dirty = run_git(repo_dir, &["status", "--porcelain", "--", &rel]).await?;
    if dirty.trim().is_empty() {
        return Ok(None);
    }
    let msg = format!("chore(projects): remove {rel}");
    Ok(Some(
        projects_git_commit_and_push(cfg, repo_dir, &rel, &msg).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/v1/projects",
    tag = "Projects",
    operation_id = "list_projects",
    responses(
        (status = 200, description = "Project list with disk sync status", body = ProjectListResponse)
    )
)]
pub(crate) async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<ProjectListResponse>, ApiError> {
    let summaries = state
        .session_db
        .list_project_config_summaries()
        .await
        .map_err(|e| session_db_err(&e))?;
    let mut projects = Vec::with_capacity(summaries.len());
    for s in &summaries {
        projects.push(build_project_list_entry(&state, s).await);
    }
    projects.sort_by_key(|p| p.proj_id);
    Ok(Json(ProjectListResponse {
        projects,
        listed_at_ms: now_ms(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/git/pull",
    tag = "Projects",
    operation_id = "pull_project_git",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Git pull outcome", body = ProjectGitPullResponse),
        (status = 400, description = "Invalid projId"),
        (status = 502, description = "Git pull failed")
    )
)]
pub(crate) async fn pull_project_git(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<ProjectGitPullResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let lock = get_proj_lock(&state, proj_id).await;
    let _guard = lock.lock().await;
    let outcome = try_pull_project_git(&state, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.message))?;
    apply_project_config_for_proj_inner(&state, proj_id, true).await?;
    project_config_apply::link_claw_compat_symlinks(&proj_work_dir(&state.cfg.work_root, proj_id))
        .await
        .map_err(|e| map_project_config_apply_err(&e))?;
    let row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    Ok(Json(ProjectGitPullResponse {
        proj_id,
        outcome,
        git_sync_json: git_sync_json_for_api(&state, &row.git_sync_json).await,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/projects",
    tag = "Projects",
    operation_id = "create_project",
    request_body = CreateProjectRequest,
    responses(
        (status = 200, description = "Project created and workspace initialized", body = InitResponse),
        (status = 400, description = "Invalid projectCode or projectDescription"),
        (status = 409, description = "projId or projectCode already in use")
    )
)]
pub(crate) async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<InitResponse>, ApiError> {
    let project_code = normalize_project_code(&req.project_code)?;
    let project_description = normalize_project_description(req.project_description.as_deref())?;
    ensure_project_code_available(&state, &project_code, None).await?;
    let proj_id = resolve_create_proj_id(&state, req.proj_id).await?;
    if project_config_exists(&state, proj_id).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("ds {proj_id} already registered in project_config"),
        ));
    }
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    let lock = get_proj_lock(&state, proj_id).await;
    let _guard = lock.lock().await;
    if fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&work_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "remove existing work_dir {} failed: {e}",
                    work_dir.display()
                ),
            )
        })?;
    }
    scaffold_proj_workspace(&work_dir, proj_id).await?;
    let now = now_ms();
    let content_rev = project_config_draft::format_formal_content_rev_local_ms(now);
    let claude_md = default_project_claude_md(proj_id);
    let empty_obj = json!({});
    let empty_arr = json!([]);
    state
        .session_db
        .upsert_project_config(session_db::ProjectConfigUpsert {
            proj_id,
            content_rev: &content_rev,
            stable_content_rev: Some(content_rev.as_str()),
            draft_open: false,
            updated_at_ms: now,
            rules_json: &empty_arr,
            mcp_servers_json: &empty_obj,
            skills_sources_json: &empty_arr,
            skills_json: &empty_arr,
            allowed_tools_json: &empty_arr,
            claude_md: Some(&claude_md),
            git_sync_json: &json!({}),
            solve_preflight_json: &json!({"kind": "none"}),
            solve_orchestration_json: &json!({"kind": "single_turn"}),
            language_pipeline_json: &json!({}),
            extra_session_fields_json: &empty_arr,
            prompt_limits_json: &empty_obj,
            worker_profile_json: &pool::default_worker_profile_json(),
            project_code: &project_code,
            project_description: &project_description,
        })
        .await
        .map_err(|e| session_db_err(&e))?;
    if let Ok(Some(row)) = state.session_db.get_project_config(proj_id).await {
        archive_project_config_revision(&state, revision_row_from_active(&row)).await?;
    }
    apply_project_config_for_proj(&state, proj_id, true).await?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    write_proj_settings_json(&state, proj_id).await?;
    Ok(Json(InitResponse {
        proj_id,
        work_dir: work_dir.display().to_string(),
        initialized: true,
    }))
}

#[utoipa::path(
    patch,
    path = "/v1/projects/{proj_id}",
    tag = "Projects",
    operation_id = "patch_project",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = PatchProjectRequest,
    responses(
        (status = 200, description = "Project metadata updated", body = PatchProjectResponse),
        (status = 400, description = "Invalid request or no fields to update"),
        (status = 404, description = "Project not found"),
        (status = 409, description = "projectCode already in use")
    )
)]
pub(crate) async fn patch_project(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<PatchProjectRequest>,
) -> Result<Json<PatchProjectResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    if req.project_code.is_none() && req.project_description.is_none() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "provide projectCode and/or projectDescription",
        ));
    }
    let existing = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("project {proj_id} not registered in project_config"),
            )
        })?;
    let project_code = if let Some(raw) = req.project_code.as_deref() {
        normalize_project_code(raw)?
    } else {
        existing.project_code.clone()
    };
    let project_description = if req.project_description.is_some() {
        normalize_project_description(req.project_description.as_deref())?
    } else {
        existing.project_description.clone()
    };
    if project_code != existing.project_code {
        ensure_project_code_available(&state, &project_code, Some(proj_id)).await?;
    }
    let now = now_ms();
    let updated = state
        .session_db
        .update_project_metadata(proj_id, &project_code, &project_description, now)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !updated {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("project {proj_id} not registered in project_config"),
        ));
    }
    Ok(Json(PatchProjectResponse {
        proj_id,
        project_code,
        project_description,
        updated_at_ms: now,
    }))
}

#[utoipa::path(
    delete,
    path = "/v1/projects/{proj_id}",
    tag = "Projects",
    operation_id = "delete_project",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("purge_sessions" = bool, Query, description = "Also delete sessions for this project")
    ),
    responses(
        (status = 200, description = "Project deleted", body = DeleteProjectResponse),
        (status = 400, description = "Invalid projId"),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn delete_project(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Query(query): Query<DeleteProjectQuery>,
) -> Result<Json<DeleteProjectResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    if !project_config_exists(&state, proj_id).await? {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("project {proj_id} not registered in project_config"),
        ));
    }
    let lock = get_proj_lock(&state, proj_id).await;
    let _guard = lock.lock().await;
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    if fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&work_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("remove work_dir failed: {e}"),
            )
        })?;
    }
    let project_config_removed = state
        .session_db
        .delete_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let sessions_removed = if query.purge_sessions {
        state
            .session_db
            .delete_sessions_for_proj(proj_id)
            .await
            .map_err(|e| session_db_err(&e))?
    } else {
        0
    };
    {
        let mut injected = state.injected_mcp.lock().await;
        injected.remove(&proj_id);
    }
    Ok(Json(DeleteProjectResponse {
        proj_id,
        deleted: true,
        purge_sessions: query.purge_sessions,
        sessions_removed,
        project_config_removed,
        git_sync: None,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/init",
    tag = "Projects",
    operation_id = "init_workspace",
    request_body = InitRequest,
    responses(
        (status = 200, description = "Workspace initialized", body = InitResponse),
        (status = 400, description = "Invalid projId"),
        (status = 404, description = "No project_config row"),
        (status = 412, description = "Environment not prepared")
    )
)]
pub(crate) async fn init_workspace(
    State(state): State<AppState>,
    Json(req): Json<InitRequest>,
) -> Result<Json<InitResponse>, ApiError> {
    if req.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let work_dir = proj_work_dir(&state.cfg.work_root, req.proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    {
        let lock = get_proj_lock(&state, req.proj_id).await;
        let _guard = lock.lock().await;
        let has_project_config = state
            .session_db
            .get_project_config(req.proj_id)
            .await
            .map_err(|e| session_db_err(&e))?
            .is_some();
        if !has_project_config {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "no project_config for proj {}; POST /v1/projects or PUT /v1/project/config/{} first",
                    req.proj_id, req.proj_id
                ),
            ));
        }
        apply_project_config_for_proj(&state, req.proj_id, true).await?;
        let materialize_row =
            project_config_draft::row_for_materialize(&state.session_db, req.proj_id)
                .await
                .map_err(|e| session_db_err(&e))?;
        if !proj_tree_ready(&work_dir, materialize_row.as_ref()).await {
            return Err(proj_environment_not_prepared_error(req.proj_id, true));
        }
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        write_proj_settings_json(&state, req.proj_id).await?;
    }
    Ok(Json(InitResponse {
        proj_id: req.proj_id,
        work_dir: work_dir.display().to_string(),
        initialized: true,
    }))
}

