// Fragment of routes::app (include!). AskUser HITL answer API. Author: kejiqing

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct AskUserAnswerBody {
    #[serde(rename = "projId", alias = "proj_id")]
    pub proj_id: i64,
    #[serde(rename = "questionId")]
    pub question_id: String,
    /// Free-text or selected option label. Author: kejiqing
    #[serde(default)]
    pub answer: Option<String>,
    /// Preferred when user picked a MultipleChoice option. Author: kejiqing
    #[serde(default)]
    pub selected: Option<String>,
}

/// Attach pending AskUserQuestion from live hub onto task poll. Author: kejiqing
pub(crate) fn enrich_task_record_with_ask_user(state: &AppState, record: &mut TaskRecord) {
    let Some(pending) = state.live_report_hub.pending_ask_for_turn(&record.turn_id) else {
        if record.status == "awaiting_user" {
            // Answer already consumed; worker still running.
            record.status = "running".into();
            record.ask_user_question_id = None;
            record.ask_user_question = None;
            record.ask_user_options = None;
            record.ask_user_a2ui = None;
        }
        return;
    };
    if matches!(record.status.as_str(), "running" | "awaiting_user" | "queued") {
        record.status = "awaiting_user".into();
        record.current_task_desc = Some("等待用户回答".into());
        record.ask_user_question_id = Some(pending.question_id);
        record.ask_user_question = Some(pending.question);
        record.ask_user_options = pending.options;
        record.ask_user_a2ui = Some(pending.a2ui);
    }
}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/turns/{turn_id}/ask-user-answer",
    tag = "Sessions",
    operation_id = "post_ask_user_answer",
    params(
        ("session_id" = String, Path, description = "Session id"),
        ("turn_id" = String, Path, description = "Turn id")
    ),
    request_body = AskUserAnswerBody,
    responses(
        (status = 200, description = "Answer accepted; worker resumes"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Turn / pending ask not found")
    )
)]
pub(crate) async fn post_ask_user_answer(
    State(state): State<AppState>,
    AxumPath((session_id, turn_id)): AxumPath<(String, String)>,
    Json(body): Json<AskUserAnswerBody>,
) -> Result<Json<Value>, ApiError> {
    if body.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let question_id = body.question_id.trim();
    if question_id.is_empty()
        || question_id.contains('/')
        || question_id.contains("..")
        || question_id.contains('\\')
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid questionId",
        ));
    }
    let answer = body
        .answer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let selected = body
        .selected
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if answer.is_none() && selected.is_none() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "answer or selected is required",
        ));
    }

    let home_rel = state
        .session_db
        .get_session_home_rel(&session_id, body.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("session not found: {session_id}"),
            )
        })?;
    session_merge::validate_session_home_rel(&home_rel).map_err(|_| {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid session home")
    })?;

    let pending = state.live_report_hub.pending_ask_for_turn(&turn_id);
    if let Some(ref p) = pending {
        if p.question_id != question_id {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "questionId mismatch: pending={}, got={question_id}",
                    p.question_id
                ),
            ));
        }
    }

    let cluster_id = crate::cluster_identity::gateway_cluster_id().map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cluster id: {e}"),
        )
    })?;
    let segment = session_merge::sessions_directory_segment(&session_id);
    let ask_dir = format!(
        "{}/.claw/ask-user",
        claw_e2b_sandbox_client::session_rel(&cluster_id, body.proj_id, &segment)
    );
    let answer_rel = format!("{ask_dir}/{question_id}.answer.json");
    let payload = gateway_solve_turn::ask_user::AskUserAnswerFile {
        question_id: question_id.to_string(),
        answer: answer.clone().unwrap_or_default(),
        selected: selected.clone(),
        answered_at_ms: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize answer: {e}"),
        )
    })?;

    let _ = state.nas_api.mkdir(&ask_dir, true).await;
    state
        .nas_api
        .put_file(&answer_rel, &bytes)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("NAS put_file ask answer failed: {e}"),
            )
        })?;

    let local_home = join_session_home(&state.cfg.work_root, &home_rel);
    let local_dir = local_home.join(".claw").join("ask-user");
    let _ = tokio::fs::create_dir_all(&local_dir).await;
    let local_path = local_dir.join(format!("{question_id}.answer.json"));
    let _ = tokio::fs::write(&local_path, &bytes).await;

    state.live_report_hub.clear_pending_ask(&turn_id);
    {
        let mut tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.values_mut().find(|t| t.record.turn_id == turn_id) {
            if inner.record.status == "awaiting_user" {
                inner.record.status = "running".into();
            }
            inner.record.ask_user_question_id = None;
            inner.record.ask_user_question = None;
            inner.record.ask_user_options = None;
            inner.record.ask_user_a2ui = None;
            inner.record.current_task_desc = Some("继续执行…".into());
        }
    }

    Ok(Json(json!({
        "ok": true,
        "sessionId": session_id,
        "turnId": turn_id,
        "questionId": question_id,
        "status": "running",
    })))
}
