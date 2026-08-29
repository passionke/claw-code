// Fragment of routes::app (include!). Author: kejiqing

pub(crate) async fn count_skill_dirs(skills_root: &Path) -> u64 {
    let mut rd = match fs::read_dir(skills_root).await {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut n = 0u64;
    while let Ok(Some(ent)) = rd.next_entry().await {
        if ent.file_type().await.is_ok_and(|t| t.is_dir()) {
            n += 1;
        }
    }
    n
}

pub(crate) async fn build_proj_workspaces_health(state: &AppState) -> Value {
    let work_root = &state.cfg.work_root;
    let ids = state
        .session_db
        .list_project_config_proj_ids()
        .await
        .unwrap_or_default();

    let mut workspaces = Vec::new();
    let mut prepared_count = 0u64;
    for proj_id in ids {
        let work_dir = proj_work_dir(work_root, proj_id);
        let work_dir_present = fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir());
        let materialize_row = project_config_draft::row_for_materialize(&state.session_db, proj_id)
            .await
            .ok()
            .flatten();
        let environment_prepared =
            work_dir_present && proj_tree_ready(&work_dir, materialize_row.as_ref()).await;
        if environment_prepared {
            prepared_count += 1;
        }
        let (home_claude, root_claude) = project_claude_paths(&work_dir);
        let claude_home_present = fs::metadata(&home_claude).await.is_ok_and(|m| m.is_file());
        let claude_root_present = fs::metadata(&root_claude).await.is_ok_and(|m| m.is_file());
        let claude_home_bytes = if claude_home_present {
            fs::metadata(&home_claude).await.ok().map(|m| m.len())
        } else {
            None
        };
        let skills_root = work_dir.join("home/skills");
        let skills_count = if fs::metadata(&skills_root).await.is_ok_and(|m| m.is_dir()) {
            count_skill_dirs(&skills_root).await
        } else {
            0
        };

        workspaces.push(json!({
            "projId": proj_id,
            "workDir": work_dir.display().to_string(),
            "workDirPresent": work_dir_present,
            "environmentPrepared": environment_prepared,
            "claudeHomePath": home_claude.display().to_string(),
            "claudeHomePresent": claude_home_present,
            "claudeHomeBytes": claude_home_bytes,
            "claudeRootPresent": claude_root_present,
            "skillsCount": skills_count,
        }));
    }

    json!({
        "projWorkspaceCount": workspaces.len(),
        "environmentPreparedCount": prepared_count,
        "projWorkspaces": workspaces,
    })
}

#[utoipa::path(
    get,
    path = "/healthz",
    tag = "System",
    operation_id = "healthz",
    responses(
        (status = 200, description = "Gateway health and runtime settings", body = Object)
    )
)]
pub(crate) async fn healthz(State(state): State<AppState>) -> Json<Value> {
    let proj_workspaces = build_proj_workspaces_health(&state).await;
    let deploy_image_ref = crate::deploy_image::image_ref_from_env();
    let deploy_image_tag = crate::deploy_image::deploy_image_tag(&deploy_image_ref);
    let cluster_snap = claw_tap_cluster_state::snapshot_from_handle(&state.claw_tap_cluster).await;
    Json(json!({
        "ok": true,
        "deployImageRef": deploy_image_ref,
        "deployImageTag": deploy_image_tag,
        "clawBin": state.cfg.claw_bin,
        "workRoot": state.cfg.work_root.display().to_string(),
        "registryPath": state.cfg.ds_registry_path.display().to_string(),
        "defaultTimeoutSeconds": state.cfg.default_timeout_seconds,
        "defaultMaxIterations": state.cfg.default_max_iterations,
        "defaultHttpMcpName": state.cfg.default_http_mcp_name,
        "defaultHttpMcpUrl": state.cfg.default_http_mcp_url,
        "defaultHttpMcpTransport": state.cfg.default_http_mcp_transport,
        "solveBackend": "e2b",
        "e2bSandbox": true,
        "poolRpcHostWorkRoot": state.cfg.pool_rpc_host_work_root.as_ref().map(|p| p.display().to_string()),
        "sessionDatabaseBackend": "postgresql",
        "gatewayDatabaseUrl": state.session_db.database_url_redacted(),
        "projectsGitUrl": state.cfg.projects_git_url.clone(),
        "projectsGitBranch": state.cfg.projects_git_branch.clone(),
        "projectsGitDsHomePollIntervalSecs": state.cfg.projects_git_proj_home_poll_interval_secs,
        "projectsGitMirror": proj_workspaces,
        "reportPolishUsesDeepseek": state.cfg.report_polish_deepseek.is_some(),
        "reportDeepseekModel": state.cfg.report_polish_deepseek.as_ref().map(|d| d.model.clone()),
        "liveBizReportSpillEnabled": state.cfg.live_biz_report_spill_enabled,
        "liveReport": {
            "contract": if state.cfg.live_biz_report_spill_enabled {
                "legacy-spill-polish"
            } else {
                crate::live_report_audit::LIVE_REPORT_CONTRACT
            },
            "producer": "worker:claw gateway-solve-once stdout __CLAW_GATEWAY_STDOUT__ report.delta",
            "ingest": "gateway LiveReportHub (e2b worker stdout)",
            "terminalSnapshot": "gateway-db (GET biz_advice_report stream when succeeded)",
            "live": if state.cfg.live_biz_report_spill_enabled {
                "LLM polish SSE (biz_advice_report after succeeded)"
            } else {
                "gateway LiveReportHub SSE"
            },
        },
        "clawTapCluster": cluster_snap,
    }))
}

#[utoipa::path(
    get,
    path = "/readyz",
    tag = "System",
    operation_id = "readyz",
    responses(
        (status = 200, description = "E2B core readiness probe passed", body = Object),
        (status = 503, description = "E2B core components not ready")
    )
)]
pub(crate) async fn readyz(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let core = gateway_e2b_core_readiness::load_core_readiness_snapshot(
        &state.session_db,
        state.pool_clients.e2b_sandbox_client().map(|v| &**v),
        &state.claw_tap_cluster,
    )
    .await
    .map_err(|e| session_db_err(&e))?;
    let landlock = probe_landlock();
    let bootstrap = gateway_cluster_bootstrap::cluster_bootstrap_status(
        &state.session_db,
        state.pool_clients.e2b_sandbox_client().map(|v| &**v),
        Some(&state.claw_tap_cluster),
    )
    .await
    .ok();
    if core.ready {
        return Ok((
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "e2bCore": core,
                "clusterBootstrap": bootstrap,
                "strictLandlockProbe": {
                    "supported": landlock.supported,
                    "enforcing": landlock.enforcing,
                    "message": landlock.message,
                },
            })),
        ));
    }
    Err(ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        {
            let mut reason = core.reason.unwrap_or_else(|| {
                "e2b core components not ready (nas-api / observe / clawTap)".into()
            });
            if bootstrap.as_ref().is_some_and(|b| b.needs_bootstrap) {
                if let Some(br) = bootstrap
                    .as_ref()
                    .and_then(|b| b.blocking_reason.clone())
                {
                    reason = format!("{reason}; bootstrap: {br}");
                }
            }
            reason
        },
    ))
}

