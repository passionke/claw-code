// Fragment of routes::app (include!). Plan mode persist + confirm. Author: kejiqing

#[derive(Debug, Deserialize)]
pub(crate) struct PlanProjQuery {
    #[serde(rename = "projId", alias = "proj_id")]
    pub proj_id: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfirmPlanBody {
    #[serde(rename = "projId", alias = "proj_id")]
    pub proj_id: i64,
}

fn plan_row_json(row: &session_db::SessionPlanRow) -> Value {
    json!({
        "planId": row.plan_id,
        "sessionId": row.session_id,
        "projId": row.proj_id,
        "title": row.title,
        "bodyMarkdown": row.body_markdown,
        "status": row.status,
        "planTurnId": row.plan_turn_id,
        "executeTurnId": row.execute_turn_id,
        "sealedAtMs": row.sealed_at_ms,
        "createdAtMs": row.created_at_ms,
        "updatedAtMs": row.updated_at_ms,
        "createdByPrompt": row.created_by_prompt,
    })
}

/// After a successful Plan-mode solve, persist markdown into `gateway_session_plans`. Author: kejiqing
pub(crate) async fn maybe_persist_plan_after_solve(
    state: &AppState,
    req: &SolveRequest,
    session_id: &str,
    turn_id: &str,
    result: &SolveResponse,
) {
    if gateway_solve_turn::InteractionMode::parse(req.interaction_mode.as_deref())
        != gateway_solve_turn::InteractionMode::Plan
    {
        return;
    }
    let body = result
        .output_json
        .as_ref()
        .and_then(|j| j.get("message").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let t = result.output_text.trim();
            if t.is_empty() {
                None
            } else if let Ok(v) = serde_json::from_str::<Value>(t) {
                v.get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                Some(result.output_text.clone())
            }
        });
    let Some(body) = body else {
        warn!(turn_id = %turn_id, "plan mode succeeded without markdown body");
        return;
    };
    let title = gateway_solve_turn::plan_title_from_markdown(&body);
    let plan_id = format!("plan_{}", uuid::Uuid::new_v4().simple());
    let prompt = req.user_prompt.trim();
    let created_by = (!prompt.is_empty()).then_some(prompt);
    if let Err(e) = state
        .session_db
        .insert_awaiting_session_plan(
            &plan_id,
            session_id,
            req.proj_id,
            &title,
            &body,
            turn_id,
            created_by,
            now_ms(),
        )
        .await
    {
        warn!(turn_id = %turn_id, plan_id = %plan_id, error = %e, "persist session plan failed");
        return;
    }
    if let Ok(Some(rel)) = state
        .session_db
        .get_session_home_rel(session_id, req.proj_id)
        .await
    {
        let home = join_session_home(&state.cfg.work_root, &rel);
        let dir = home.join(".claw").join("plans");
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join(format!("{plan_id}.md")), body.as_bytes());
        }
    }
}

pub(crate) async fn enrich_task_record_with_plan(state: &AppState, record: &mut TaskRecord) {
    if let Ok(Some(plan)) = state
        .session_db
        .get_session_plan_by_turn(&record.session_id, record.proj_id, &record.turn_id)
        .await
    {
        record.plan_id = Some(plan.plan_id.clone());
        record.plan_title = Some(plan.title.clone());
        record.plan_markdown = Some(plan.body_markdown.clone());
        record.plan_turn_id = Some(plan.plan_turn_id.clone());
        record.interaction_mode = Some("plan".into());
        record.plan_phase = Some(match plan.status.as_str() {
            "awaiting_confirm" => "awaiting_confirm".into(),
            "sealed" => "confirmed".into(),
            "superseded" => "superseded".into(),
            other => other.to_string(),
        });
        if plan.status == "awaiting_confirm" {
            record.current_task_desc = Some(
                record
                    .current_task_desc
                    .clone()
                    .unwrap_or_else(|| "方案待确认".into()),
            );
        }
        return;
    }
    if let Ok(Some(plan)) = state
        .session_db
        .get_session_plan_by_execute_turn(&record.session_id, record.proj_id, &record.turn_id)
        .await
    {
        record.plan_id = Some(plan.plan_id.clone());
        record.plan_title = Some(plan.title.clone());
        record.plan_markdown = Some(plan.body_markdown.clone());
        record.plan_turn_id = Some(plan.plan_turn_id.clone());
        record.plan_phase = Some(match plan.status.as_str() {
            "sealed" if record.status == "succeeded" => "done".into(),
            "sealed" => "executing".into(),
            other => other.to_string(),
        });
        record.interaction_mode = Some("agent".into());
    }
}

pub(crate) async fn list_session_plans(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<PlanProjQuery>,
) -> Result<Json<Value>, ApiError> {
    let rows = state
        .session_db
        .list_session_plans(&session_id, q.proj_id, 50)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(json!({
        "sessionId": session_id,
        "projId": q.proj_id,
        "plans": rows.iter().map(plan_row_json).collect::<Vec<_>>(),
    })))
}

pub(crate) async fn get_session_plan(
    State(state): State<AppState>,
    AxumPath((session_id, plan_id)): AxumPath<(String, String)>,
    Query(q): Query<PlanProjQuery>,
) -> Result<Json<Value>, ApiError> {
    let row = state
        .session_db
        .get_session_plan(&session_id, q.proj_id, &plan_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "plan not found"))?;
    Ok(Json(plan_row_json(&row)))
}

pub(crate) async fn confirm_session_plan(
    State(state): State<AppState>,
    AxumPath((session_id, plan_id)): AxumPath<(String, String)>,
    Json(body): Json<ConfirmPlanBody>,
) -> Result<Json<SolveAsyncResponse>, ApiError> {
    let plan = state
        .session_db
        .get_session_plan(&session_id, body.proj_id, &plan_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "plan not found"))?;
    if plan.status != "awaiting_confirm" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("plan status is {}, expected awaiting_confirm", plan.status),
        ));
    }

    let execute_turn_id = turn_id::mint_turn_id();
    let sealed = state
        .session_db
        .seal_session_plan(
            &session_id,
            body.proj_id,
            &plan_id,
            &execute_turn_id,
            now_ms(),
        )
        .await
        .map_err(|e| session_db_err(&e))?;
    if !sealed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "plan could not be sealed (already confirmed or superseded)",
        ));
    }

    let execute_prompt = format!(
        "按已确认方案执行（planId={}，title={}）。不要重新规划；直接实施封板步骤并完成验收。",
        plan.plan_id, plan.title
    );
    let req = SolveRequest {
        proj_id: body.proj_id,
        user_prompt: execute_prompt,
        session_id: Some(session_id.clone()),
        model: None,
        timeout_seconds: None,
        extra_session: None,
        allowed_tools: None,
        max_iterations: None,
        attachments: None,
        interaction_mode: Some("agent".into()),
        sealed_plan_id: Some(plan.plan_id.clone()),
        sealed_plan_markdown: Some(plan.body_markdown.clone()),
        force_single_turn: Some(true),
    };

    let http_request_id = HttpRequestId(session_id.clone());
    let async_res = enqueue_solve_async_with_turn(
        state,
        http_request_id,
        session_merge::HttpRequestIdKind::FromClientHeader,
        req,
        "/v1/sessions/{session_id}/plans/{plan_id}/confirm",
        Some(client_origin::CLIENT_ORIGIN_GATEWAY_ADMIN.to_string()),
        Some(execute_turn_id),
    )
    .await?;

    Ok(Json(async_res))
}
