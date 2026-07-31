// Fragment of routes::app (include!). Author: kejiqing

pub(crate) fn default_system_date() -> String {
    match option_env!("BUILD_DATE") {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => current_utc_date(),
    }
}

pub(crate) async fn inject_http_request_id(mut req: Request, next: Next) -> Response {
    let id_claw = req
        .headers()
        .get("claw-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let id_xreq = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let (id, kind) = if let Some(id) = id_claw {
        (id, session_merge::HttpRequestIdKind::FromClientHeader)
    } else if let Some(id) = id_xreq {
        (id, session_merge::HttpRequestIdKind::FromClientHeader)
    } else {
        (
            Uuid::new_v4().simple().to_string(),
            session_merge::HttpRequestIdKind::Generated,
        )
    };
    req.extensions_mut().insert(HttpRequestId(id.clone()));
    req.extensions_mut().insert(kind);
    let mut res = next.run(req).await;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    // Handlers such as `/v1/solve` set these from the merged effective session id; do not overwrite.
    if !res.headers().contains_key(&xrid) {
        if let Ok(value) = HeaderValue::from_str(&id) {
            res.headers_mut().insert(xrid, value);
        }
    }
    if !res.headers().contains_key(&csid) {
        if let Ok(value) = HeaderValue::from_str(&id) {
            res.headers_mut().insert(csid, value);
        }
    }
    res
}

pub(crate) async fn get_session_solve_lock(
    state: &AppState,
    proj_id: i64,
    session_id: &str,
) -> Arc<Mutex<()>> {
    let mut locks = state.session_solve_locks.lock().await;
    locks
        .entry((proj_id, session_id.to_string()))
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub(crate) fn client_origin_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(client_origin::HEADER_CLIENT_ORIGIN)
        .and_then(|v| v.to_str().ok())
}

pub(crate) fn resolve_request_client_origin(
    extra_session: Option<&Value>,
    headers: &HeaderMap,
) -> Option<String> {
    client_origin::resolve_client_origin(extra_session, client_origin_from_headers(headers))
}

pub(crate) fn build_turn_entry_params_json(
    req: &SolveRequest,
    session_id: &str,
    turn_id: &str,
    client_origin: Option<&str>,
) -> Value {
    json!({
        "projId": req.proj_id,
        "userPrompt": req.user_prompt,
        "sessionId": session_id,
        "turnId": turn_id,
        "model": req.model,
        "timeoutSeconds": req.timeout_seconds,
        "maxIterations": req.max_iterations,
        "extraSession": req.extra_session,
        "allowedTools": req.allowed_tools,
        "attachments": req.attachments,
        "clientOrigin": client_origin,
    })
}

pub(crate) async fn apply_turn_pool_fields_from_db(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    session_id: &str,
    proj_id: i64,
    record: &mut TaskRecord,
) {
    if let Ok(Some(pid)) = db.get_turn_pool_id(turn_id, session_id, proj_id).await {
        let t = pid.trim();
        if !t.is_empty() {
            record.pool_id = Some(t.to_string());
        }
    }
    if let Ok(Some(wn)) = db.get_turn_worker_name(turn_id).await {
        let t = wn.trim();
        if !t.is_empty() {
            record.worker_name = Some(t.to_string());
        }
    }
    if let Ok(Some(eu)) = db.get_turn_worker_exec_user(turn_id).await {
        let t = eu.trim();
        if !t.is_empty() {
            record.worker_exec_user = Some(t.to_string());
        }
    }
    if let Ok(Some((gid, gbase))) = db
        .get_turn_gateway_owner(turn_id, session_id, proj_id)
        .await
    {
        let id = gid.trim();
        if !id.is_empty() {
            record.gateway_id = Some(id.to_string());
        }
        let base = gbase.trim();
        if !base.is_empty() {
            record.gateway_base = Some(base.to_string());
        }
    }
}

pub(crate) async fn register_solve_turn(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    session_id: &str,
    req: &SolveRequest,
    _co_located_pool_id: Option<&str>,
    client_origin: Option<&str>,
    gateway_identity: Option<&crate::gateway_endpoint::GatewayEndpointIdentity>,
) -> Result<(), ApiError> {
    let prompt = req.user_prompt.trim();
    let user_prompt = (!prompt.is_empty()).then_some(prompt);
    let entry_params = build_turn_entry_params_json(req, session_id, turn_id, client_origin);
    db.insert_turn(
        turn_id,
        session_id,
        req.proj_id,
        "queued",
        now_ms(),
        user_prompt,
        client_origin,
        Some(&entry_params),
    )
    .await
    .map_err(|e| session_db_err(&e))?;
    // Backend marker only — not machine ingress. Author: kejiqing
    if let Err(e) = db.assign_turn_pool_id(turn_id, pool::E2B_POOL_ID).await {
        warn!(
            target: "claw_live_report",
            turn_id = %turn_id,
            error = %e,
            "gateway_turns pool_id=e2b-cloud bind failed"
        );
    }
    if let Some(identity) = gateway_identity {
        if let Err(e) = db
            .assign_turn_gateway(turn_id, &identity.gateway_id, &identity.gateway_base)
            .await
        {
            warn!(
                target: "claw_gateway_endpoint",
                turn_id = %turn_id,
                error = %e,
                "assign_turn_gateway failed"
            );
        } else {
            info!(
                target: "claw_gateway_endpoint",
                turn_id = %turn_id,
                gateway_id = %identity.gateway_id,
                gateway_base = %identity.gateway_base,
                "turn ingress gateway bound at enqueue"
            );
        }
    }
    Ok(())
}

pub(crate) async fn set_solve_turn_status(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    status: &str,
    finished: bool,
) {
    let finished_at = finished.then_some(now_ms());
    if let Err(e) = db.update_turn_status(turn_id, status, finished_at).await {
        warn!(turn_id = %turn_id, error = %e, "update gateway_turns status failed");
    }
}

pub(crate) async fn finalize_solve_turn_success(
    db: Arc<session_db::GatewaySessionDb>,
    turn_id: &str,
    result: &SolveResponse,
) {
    // e2b solve readback can mark `status=succeeded` + `artifacts_ready=true` before this handoff.
    // Gateway must not overwrite that with `finalize_turn_terminal` (which omits `artifacts_ready`).
    match db.turn_artifacts_ready(turn_id).await {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            warn!(
                turn_id = %turn_id,
                error = %e,
                "turn_artifacts_ready lookup failed; falling back to gateway finalize"
            );
        }
    }
    let finished_at = Some(now_ms());
    let report =
        report_body_from_solve_output(&result.output_text, result.output_json.as_ref()).ok();
    if let Err(e) = db
        .finalize_turn_with_artifacts_ready(
            turn_id,
            "succeeded",
            finished_at,
            result.claw_exit_code,
            report.as_deref(),
            result.output_json.as_ref(),
            true,
        )
        .await
    {
        warn!(
            turn_id = %turn_id,
            error = %e,
            "finalize gateway_turns succeeded snapshot failed"
        );
    }
}

pub(crate) async fn finalize_solve_turn_failed(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    err: &ApiError,
) {
    let detail = json!({"status_code": err.status.as_u16(), "detail": err.message});
    if let Err(e) = db
        .finalize_turn_terminal(turn_id, "failed", Some(now_ms()), None, Some(&detail), None)
        .await
    {
        warn!(
            turn_id = %turn_id,
            error = %e,
            "finalize gateway_turns failed snapshot failed"
        );
    }
}

pub(crate) async fn finalize_solve_turn_cancelled(db: &session_db::GatewaySessionDb, turn_id: &str) {
    if let Err(e) = db
        .finalize_turn_terminal(turn_id, "cancelled", Some(now_ms()), None, None, None)
        .await
    {
        warn!(
            turn_id = %turn_id,
            error = %e,
            "finalize gateway_turns cancelled snapshot failed"
        );
    }
}

pub(crate) fn session_db_err(e: &sqlx::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("gateway session database error: {e}"),
    )
}

pub(crate) fn pool_host_bind_root(work_root: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var("CLAW_POOL_WORK_ROOT_HOST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.exists() {
                return p;
            }
            warn!(
                target: "claw_gateway_orchestration",
                component = "startup",
                phase = "pool_host_bind_root_fallback",
                configured = %trimmed,
                fallback = %work_root.display(),
                "CLAW_POOL_WORK_ROOT_HOST not found in this filesystem; using CLAW_WORK_ROOT"
            );
        }
    }
    work_root.to_path_buf()
}

pub(crate) fn mandatory_nonempty_env(var: &'static str) -> String {
    if let Ok(value) = std::env::var(var) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            eprintln!(
                "http-gateway-rs: {var} is set but empty; set a non-empty value (e.g. in deploy .env)."
            );
            std::process::exit(1);
        }
        trimmed.to_string()
    } else {
        eprintln!(
            "http-gateway-rs: {var} is required for project Git sync; set it in the environment (see repo root .env.example)."
        );
        std::process::exit(1);
    }
}

pub(crate) fn validate_projects_git_at_startup(url: &str, token: Option<&str>) {
    let base = url.trim();
    let needs_creds = base.starts_with("https://") || base.starts_with("http://");
    if !needs_creds {
        return;
    }
    let rest = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or("");
    let has_userinfo = rest.contains('@');
    let has_token = token.is_some_and(|t| !t.trim().is_empty());
    if !has_userinfo && !has_token {
        eprintln!(
            "http-gateway-rs: CLAW_PROJECTS_GIT_URL is HTTP(S) without embedded credentials (no userinfo before host) and CLAW_PROJECTS_GIT_TOKEN is unset or empty; set CLAW_PROJECTS_GIT_TOKEN or embed user:token@ in the URL."
        );
        std::process::exit(1);
    }
}

pub(crate) fn proj_work_dir(work_root: &Path, proj_id: i64) -> PathBuf {
    work_root.join(format!("proj_{proj_id}"))
}

pub(crate) fn projects_repo_dir(work_root: &Path) -> PathBuf {
    work_root.join(".claw-code-projects")
}

pub(crate) fn project_claude_paths(work_dir: &Path) -> (PathBuf, PathBuf) {
    (work_dir.join("home/CLAUDE.md"), work_dir.join("CLAUDE.md"))
}

pub(crate) fn map_project_config_apply_err(e: &project_config_apply::ProjectConfigApplyError) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub(crate) fn normalize_rel_for_git(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn parse_projects_git_author(author: &str) -> (String, String) {
    let s = author.trim();
    if let (Some(i), Some(j)) = (s.find('<'), s.rfind('>')) {
        if i < j {
            let name = s[..i].trim();
            let email = s[i + 1..j].trim();
            if !email.is_empty() {
                let name_owned = if name.is_empty() {
                    "claw-gateway".to_string()
                } else {
                    name.to_string()
                };
                return (name_owned, email.to_string());
            }
        }
    }
    (s.to_string(), "noreply@claw.local".to_string())
}

pub(crate) fn validate_skill_name(skill_name: &str) -> Result<(), ApiError> {
    if skill_name.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillName cannot be empty",
        ));
    }
    if skill_name
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillName only allows [a-zA-Z0-9._-]",
        ));
    }
    Ok(())
}

pub(crate) fn entity_revision_err(e: project_entity_revision::EntityRevisionError) -> ApiError {
    ApiError::new(e.status, e.message)
}

pub(crate) fn draft_err(e: project_config_draft::DraftError) -> ApiError {
    ApiError::new(e.status, e.message)
}

pub(crate) async fn copy_tree(src_root: &Path, dst_root: &Path) -> Result<(), ApiError> {
    if !fs::metadata(src_root).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src_root.to_path_buf(), dst_root.to_path_buf())];
    while let Some((src_dir, dst_dir)) = stack.pop() {
        fs::create_dir_all(&dst_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create dir during sync failed: {e}"),
            )
        })?;
        let mut entries = fs::read_dir(&src_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read dir during sync failed: {e}"),
            )
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("iterate dir during sync failed: {e}"),
            )
        })? {
            let entry_path = entry.path();
            let dst_path = dst_dir.join(entry.file_name());
            let file_type = entry.file_type().await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("read file type during sync failed: {e}"),
                )
            })?;
            if file_type.is_dir() {
                stack.push((entry_path, dst_path));
            } else if file_type.is_file() {
                if let Some(parent) = dst_path.parent() {
                    fs::create_dir_all(parent).await.map_err(|e| {
                        ApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("create parent dir during sync failed: {e}"),
                        )
                    })?;
                }
                fs::copy(&entry_path, &dst_path).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("copy file during sync failed: {e}"),
                    )
                })?;
            }
        }
    }
    Ok(())
}

pub(crate) async fn run_git(cwd: &Path, args: &[&str]) -> Result<String, ApiError> {
    run_git_env(cwd, &[], args).await
}

pub(crate) async fn ensure_projects_git_safe_directory(work_root: &Path) {
    let repo_dir = projects_repo_dir(work_root);
    let path = repo_dir.display().to_string();
    if let Err(e) = run_git(
        work_root,
        &["config", "--global", "--add", "safe.directory", &path],
    )
    .await
    {
        warn!(
            target: "claw_gateway_orchestration",
            component = "projects_git",
            phase = "safe_directory",
            repo_dir = %repo_dir.display(),
            error = %e.detail(),
            "git safe.directory failed; mirror pull/init may fail with dubious ownership"
        );
    }
}

pub(crate) async fn git_rev_parse_optional(cwd: &Path, spec: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["rev-parse", spec]);
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

pub(crate) async fn run_git_env(
    cwd: &Path,
    env_pairs: &[(&str, &str)],
    args: &[&str],
) -> Result<String, ApiError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["-c", "http.version=HTTP/1.1"]);
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    cmd.args(args);
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("git command failed to start: {e}"),
            )
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        let shown = if args.is_empty() {
            "-c http.version=HTTP/1.1".to_string()
        } else {
            format!("-c http.version=HTTP/1.1 {}", args.join(" "))
        };
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git {shown} failed: {detail}"),
        ));
    }
    Ok(stdout)
}

pub(crate) fn map_gateway_solve_turn_err(e: gateway_solve_turn::GatewaySolveTurnError) -> ApiError {
    let status = match e.status {
        504 => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    };
    ApiError::new(status, e.message)
}

pub(crate) async fn get_proj_lock(state: &AppState, proj_id: i64) -> Arc<Mutex<()>> {
    let mut locks = state.proj_locks.lock().await;
    locks
        .entry(proj_id)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub(crate) async fn validate_proj_exists(proj_id: i64, path: &Path) -> Result<(), ApiError> {
    if fs::metadata(path).await.is_err() {
        warn!("datasource registry not found: {}", path.display());
        return Ok(());
    }
    let text = fs::read_to_string(path).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read datasource registry failed: {e}"),
        )
    })?;
    let parsed = serde_yaml::from_str::<Value>(&text).unwrap_or_else(|_| json!({}));
    if let Some(ds) = parsed
        .get("datasources")
        .and_then(Value::as_object)
        .and_then(|m| m.get(&proj_id.to_string()))
    {
        if ds.is_object() {
            return Ok(());
        }
    }
    Ok(())
}

pub(crate) fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

pub(crate) fn current_utc_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let days_since_epoch = (now.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = u64::try_from(z - era * 146_097).expect("day-of-era is non-negative"); // [0, 146_096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

pub(crate) fn load_mcp_servers_from_claw_config() -> HashMap<String, Value> {
    let Ok(path) = std::env::var("CLAW_CONFIG_FILE") else {
        return HashMap::new();
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return HashMap::new(),
    };
    let parsed = match serde_json::from_str::<Value>(&raw) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let mut out = HashMap::new();
    let Some(mcp) = parsed.get("mcpServers").and_then(Value::as_object) else {
        return out;
    };
    for (name, cfg) in mcp {
        out.insert(name.clone(), cfg.clone());
    }
    out
}

pub(crate) fn gateway_env_enabled(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|v| {
        let s = v.trim().to_ascii_lowercase();
        matches!(s.as_str(), "1" | "true" | "yes" | "on")
    })
}

pub(crate) fn resolve_effective_allowed_tools_for_ds(
    project_selected: Option<&[String]>,
    requested_allowed_tools: Option<&[String]>,
) -> Result<Vec<String>, ApiError> {
    project_tools::resolve_effective_allowed_tools_for_ds(project_selected, requested_allowed_tools)
        .map_err(|msg| ApiError::new(StatusCode::BAD_REQUEST, msg))
}

#[cfg(test)]
mod max_iterations_entry_params_tests {
    use super::*;

    fn request(max_iterations: Option<usize>) -> SolveRequest {
        SolveRequest {
            proj_id: 1,
            user_prompt: "test".into(),
            session_id: None,
            model: None,
            timeout_seconds: None,
            extra_session: None,
            allowed_tools: None,
            max_iterations,
            attachments: None,
        }
    }

    #[test]
    fn entry_params_persists_request_max_iterations() {
        let entry =
            build_turn_entry_params_json(&request(Some(4)), "session-1", "T_1", None);
        assert_eq!(entry["maxIterations"], 4);
    }

    #[test]
    fn entry_params_keeps_unset_request_as_null() {
        let entry = build_turn_entry_params_json(&request(None), "session-1", "T_1", None);
        assert!(entry["maxIterations"].is_null());
    }
}

