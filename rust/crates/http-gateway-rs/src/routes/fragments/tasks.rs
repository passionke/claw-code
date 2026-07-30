// Fragment of routes::app (include!). Author: kejiqing

pub(crate) async fn try_load_task_record(
    state: &AppState,
    task_id: &str,
) -> Result<Option<(TaskRecord, i64)>, ApiError> {
    {
        let tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get(task_id) {
            return Ok(Some((inner.record.clone(), inner.proj_id)));
        }
    }
    let Some(row) = state
        .session_db
        .fetch_latest_turn_for_session(task_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Ok(None);
    };
    Ok(Some(
        task_record_from_latest_turn_row(state, task_id, row).await?,
    ))
}

pub(crate) async fn task_record_from_latest_turn_row(
    state: &AppState,
    task_id: &str,
    row: session_db::LatestTurnRow,
) -> Result<(TaskRecord, i64), ApiError> {
    let session_home_rel = state
        .session_db
        .get_session_home_rel(task_id, row.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_default();
    let work_dir = join_session_home(&state.cfg.work_root, &session_home_rel)
        .to_string_lossy()
        .to_string();
    let duration_ms = row
        .finished_at_ms
        .unwrap_or(row.created_at_ms)
        .saturating_sub(row.created_at_ms);
    let output_text = row
        .report_message
        .clone()
        .or_else(|| {
            row.output_json.as_ref().and_then(|j| {
                j.get("message")
                    .and_then(Value::as_str)
                    .map(std::string::ToString::to_string)
            })
        })
        .unwrap_or_default();
    let result = if row.status == "succeeded" {
        Some(SolveResponse {
            session_id: task_id.to_string(),
            request_id: task_id.to_string(),
            session_home_rel: session_home_rel.clone(),
            proj_id: row.proj_id,
            work_dir,
            duration_ms,
            claw_exit_code: row.claw_exit_code.unwrap_or(0),
            output_text,
            output_json: row.output_json.clone(),
            turn_id: row.turn_id.clone(),
        })
    } else {
        None
    };
    let error = if row.status == "failed" {
        row.output_json
            .clone()
            .or_else(|| Some(json!({"detail": "solve turn failed"})))
    } else if row.status == "cancelled" {
        Some(json!({"detail":"cancelled by client","outcome":"cancelled"}))
    } else {
        None
    };
    let session_home = resolve_session_home_path(state, row.proj_id, task_id).await;
    let queue = {
        let tasks = state.tasks.lock().await;
        gateway_queue_snapshot(&tasks)
    };
    let trace_paths = session_home
        .as_ref()
        .map(|home| discover_trace_paths(home, &state.cfg.work_root, task_id))
        .unwrap_or_default();
    let tool = trace_tail_suggests_tool_call(&trace_paths);
    let progress_snap =
        load_turn_progress_snapshot(state, &row.turn_id, task_id, row.proj_id, &row.status, 50)
            .await
            .unwrap_or_default();
    let current_task_desc = resolve_current_task_desc(
        &row.status,
        &queue,
        tool,
        progress_snap.task_progress.as_ref(),
    );
    let progress_updated_at_ms = progress_snap
        .task_progress
        .as_ref()
        .map(|p| p.updated_at_ms);
    let mut record = TaskRecord {
        task_id: task_id.to_string(),
        session_id: task_id.to_string(),
        request_id: task_id.to_string(),
        proj_id: row.proj_id,
        status: row.status.clone(),
        created_at_ms: row.created_at_ms,
        started_at_ms: Some(row.created_at_ms),
        finished_at_ms: row.finished_at_ms,
        current_task_desc,
        progress_updated_at_ms,
        result,
        error,
        turn_id: row.turn_id.clone(),
        progress_history: Vec::new(),
        has_report: false,
        report_time_ms: None,
        plan_title: None,
        todos: Vec::new(),
        pool_id: row.pool_id.clone(),
        worker_name: row.worker_name.clone(),
        worker_profile: state
            .session_db
            .get_worker_profile_json(row.proj_id)
            .await
            .ok()
            .map(|j| pool::profile_mode_label(&j).to_string()),
        worker_exec_user: row.worker_exec_user.clone(),
        gateway_id: None,
        gateway_base: None,
    };
    let (plan_title, todos) = pool_consumer_resolve::plan_fields_from_snapshot(&progress_snap);
    record.progress_history = progress_snap.events;
    record.plan_title = plan_title;
    record.todos = todos;
    let _ = session_home;
    let turn_id = record.turn_id.clone();
    let session_id = record.session_id.clone();
    let proj_id = record.proj_id;
    apply_turn_pool_fields_from_db(
        &state.session_db,
        &turn_id,
        &session_id,
        proj_id,
        &mut record,
    )
    .await;
    record.has_report = task_has_report(state, &record).await;
    record.report_time_ms = task_report_time_ms(state, &record).await;
    Ok((record, proj_id))
}

#[utoipa::path(
    get,
    path = "/v1/tasks/{task_id}",
    tag = "Solve",
    operation_id = "get_task",
    params(
        ("task_id" = String, Path, description = "Async task id (same as session id)")
    ),
    responses(
        (status = 200, description = "Task status", body = TaskRecord),
        (status = 404, description = "Task not found")
    )
)]
pub(crate) async fn get_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    refresh_task_progress(&state, &task_id).await;
    let (mut task, proj_id) = try_load_task_record(&state, &task_id)
        .await?
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
        })?;
    let session_home = resolve_session_home_path(&state, proj_id, &task.session_id).await;
    let progress_snap = load_turn_progress_snapshot(
        &state,
        &task.turn_id,
        &task.session_id,
        proj_id,
        &task.status,
        50,
    )
    .await?;
    let (plan_title, todos) = pool_consumer_resolve::plan_fields_from_snapshot(&progress_snap);
    task.progress_history = progress_snap.events;
    task.plan_title = plan_title;
    task.todos = todos;
    let queue = {
        let tasks = state.tasks.lock().await;
        gateway_queue_snapshot(&tasks)
    };
    let trace_paths = session_home
        .as_ref()
        .map(|home| discover_trace_paths(home, &state.cfg.work_root, &task.session_id))
        .unwrap_or_default();
    let tool = trace_tail_suggests_tool_call(&trace_paths);
    let desc = resolve_current_task_desc(
        &task.status,
        &queue,
        tool,
        progress_snap.task_progress.as_ref(),
    );
    if matches!(task.status.as_str(), "queued" | "running") {
        task.current_task_desc = desc;
        task.progress_updated_at_ms = progress_snap
            .task_progress
            .as_ref()
            .map(|p| p.updated_at_ms);
    } else if task.current_task_desc.is_none() {
        task.current_task_desc = desc;
        task.progress_updated_at_ms = progress_snap
            .task_progress
            .as_ref()
            .map(|p| p.updated_at_ms);
    }
    let turn_id = task.turn_id.clone();
    let session_id = task.session_id.clone();
    apply_turn_pool_fields_from_db(&state.session_db, &turn_id, &session_id, proj_id, &mut task)
        .await;
    task.has_report = task_has_report(&state, &task).await;
    task.report_time_ms = task_report_time_ms(&state, &task).await;
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        task_request_id = %task.request_id,
        task_status = %task.status,
        has_report = task.has_report,
        report_time_ms = ?task.report_time_ms,
        progress_events = task.progress_history.len(),
        endpoint = "/v1/tasks/{task_id}",
        phase = "poll",
        "gateway_task"
    );
    Ok(Json(task))
}

pub(crate) fn task_has_report_for_status(status: &str, live_biz_report_spill_enabled: bool) -> bool {
    if live_biz_report_spill_enabled {
        return status == "succeeded";
    }
    status == "succeeded"
}

pub(crate) async fn task_has_report(state: &AppState, task: &TaskRecord) -> bool {
    if task_has_report_for_status(&task.status, state.cfg.live_biz_report_spill_enabled) {
        return true;
    }
    matches!(task.status.as_str(), "running" | "queued")
        && !task.turn_id.is_empty()
        && state
            .pool_clients
            .has_report_for_turn(&state.session_db, &task.turn_id)
            .await
}

pub(crate) async fn task_report_time_ms(state: &AppState, task: &TaskRecord) -> Option<i64> {
    if !task_has_report(state, task).await {
        return None;
    }
    if !task.turn_id.is_empty() {
        if let Some(ts) = state
            .pool_clients
            .first_report_at_ms_for_turn(&state.session_db, &task.turn_id)
            .await
        {
            return Some(ts);
        }
    }
    task.started_at_ms.or(task.finished_at_ms)
}

pub(crate) fn task_status_is_terminal_for_cancel(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

pub(crate) fn task_cancel_idempotent_response(record: TaskRecord) -> TaskRecord {
    let status_at_cancel = record.status.clone();
    let previous_error = record.error.clone();
    let detail = match status_at_cancel.as_str() {
        "cancelled" => "task already cancelled; duplicate cancel ignored".to_string(),
        "succeeded" => "task already succeeded; cancel had no effect".to_string(),
        "failed" => "task already failed; cancel had no effect".to_string(),
        other => format!("task already in terminal state ({other}); cancel had no effect"),
    };
    let mut out = record;
    out.error = Some(json!({
        "detail": detail,
        "outcome": "idempotent",
        "cancelApplied": false,
        "statusAtCancel": status_at_cancel,
        "previousError": previous_error,
    }));
    out
}

pub(crate) async fn cancel_task_cold_db(
    state: &AppState,
    task_id: &str,
    http_request_id: &HttpRequestId,
) -> Result<Json<TaskRecord>, ApiError> {
    let Some(row) = state
        .session_db
        .fetch_latest_turn_for_session(task_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("task not found: {task_id}"),
        ));
    };
    if task_status_is_terminal_for_cancel(&row.status) {
        let (record, _) = task_record_from_latest_turn_row(state, task_id, row).await?;
        let task_status = record.status.clone();
        let out = task_cancel_idempotent_response(record);
        info!(
            request_id = %http_request_id.0,
            task_id = %task_id,
            task_status = %task_status,
            endpoint = "/v1/tasks/{task_id}/cancel",
            phase = "cancel_idempotent_db",
            "gateway_task"
        );
        return Ok(Json(out));
    }
    finalize_solve_turn_cancelled(&state.session_db, &row.turn_id).await;
    let Some(row2) = state
        .session_db
        .fetch_latest_turn_for_session(task_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "task row missing after cancel",
        ));
    };
    let (record, _) = task_record_from_latest_turn_row(state, task_id, row2).await?;
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        endpoint = "/v1/tasks/{task_id}/cancel",
        phase = "cancel_cold_db",
        "gateway_task"
    );
    Ok(Json(record))
}

#[utoipa::path(
    post,
    path = "/v1/tasks/{task_id}/cancel",
    tag = "Solve",
    operation_id = "cancel_task",
    params(
        ("task_id" = String, Path, description = "Async task id")
    ),
    responses(
        (status = 200, description = "Task cancelled or idempotent no-op", body = TaskRecord),
        (status = 404, description = "Task not found")
    )
)]
pub(crate) async fn cancel_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    let cancel_handle = {
        let mut tasks = state.tasks.lock().await;
        let Some(inner) = tasks.get_mut(&task_id) else {
            return cancel_task_cold_db(&state, &task_id, &http_request_id).await;
        };
        if task_status_is_terminal_for_cancel(&inner.record.status) {
            let task_status = inner.record.status.clone();
            let record = task_cancel_idempotent_response(inner.record.clone());
            info!(
                request_id = %http_request_id.0,
                task_id = %task_id,
                task_status = %task_status,
                endpoint = "/v1/tasks/{task_id}/cancel",
                phase = "cancel_idempotent",
                "gateway_task"
            );
            return Ok(Json(record));
        }
        let h = inner.cancel.take();
        inner.record.status = "cancelled".to_string();
        inner.record.finished_at_ms = Some(now_ms());
        inner.record.result = None;
        inner.record.error = Some(json!({
            "detail": "cancelled by client",
            "outcome": "cancelled",
            "cancelApplied": true,
        }));
        h
    };
    // Stop the container worker before aborting the host task: `kill_on_drop` then tears down
    // the `docker exec` client, and in-flight stderr can still flush while the container exits.
    if let Some((pool, idx)) = state.docker_slots.lock().await.remove(&task_id) {
        let _ = pool.force_kill_slot(idx).await;
    }
    if let Some(h) = cancel_handle {
        h.abort();
    }
    let record = {
        let mut tasks = state.tasks.lock().await;
        let Some(inner) = tasks.remove(&task_id) else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("task not found: {task_id}"),
            ));
        };
        inner.record
    };
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        endpoint = "/v1/tasks/{task_id}/cancel",
        phase = "cancel",
        "gateway_task"
    );
    finalize_solve_turn_cancelled(&state.session_db, &record.turn_id).await;
    Ok(Json(record))
}

