//! Live business report from assistant stream spill + session jsonl (no polish LLM). Author: kejiqing

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gateway_solve_turn::{
    assistant_stream_spill_path, final_assistant_report_text_from_jsonl,
    spill_bytes_contain_end_marker, split_spill_end_marker,
};
use serde_json::{json, Value};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use crate::biz_advice_report::{
    report_body_from_solve_output, BizAdviceReportPayload, BizReportStreamMsg,
};
use crate::session_db::GatewaySessionDb;
use crate::{ApiError, AppState};

const POLL_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone)]
pub struct LiveReportContext {
    pub session_id: String,
    pub turn_id: String,
    pub ds_id: i64,
    pub session_home: PathBuf,
}

/// Resolve formal report text: task result `message` when succeeded, else session jsonl.
pub async fn resolve_formal_report_text(
    state: &AppState,
    ctx: &LiveReportContext,
) -> Result<String, ApiError> {
    let tasks = state.tasks.lock().await;
    if let Some(inner) = tasks.get(&ctx.session_id) {
        if inner.record.turn_id == ctx.turn_id {
            if let Some(ref result) = inner.record.result {
                if result.claw_exit_code == 0 {
                    if let Ok(body) = report_body_from_solve_output(
                        &result.output_text,
                        result.output_json.as_ref(),
                    ) {
                        if !body.trim().is_empty() {
                            return Ok(body);
                        }
                    }
                }
            }
        }
    }
    drop(tasks);
    let text = tokio::task::spawn_blocking({
        let home = ctx.session_home.clone();
        move || final_assistant_report_text_from_jsonl(&home)
    })
    .await
    .map_err(|e| {
        ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("report read join failed: {e}"),
        )
    })?
    .map_err(|detail| {
        ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("read session report failed: {detail}"),
        )
    })?;
    if text.trim().is_empty() {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "formal report not ready (empty session transcript)",
        ));
    }
    Ok(text)
}

pub async fn turn_status(
    db: &GatewaySessionDb,
    turn_id: &str,
    session_id: &str,
    ds_id: i64,
) -> Result<Option<String>, ApiError> {
    db.get_turn_status(turn_id, session_id, ds_id)
        .await
        .map_err(|e| {
            ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("turn status query failed: {e}"),
            )
        })
}

fn is_terminal_turn_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

async fn read_spill_from_offset(
    path: &Path,
    offset: u64,
) -> Result<(Vec<u8>, u64), std::io::Error> {
    let mut file = fs::File::open(path).await?;
    let len = file.metadata().await?.len();
    if offset >= len {
        return Ok((Vec::new(), offset));
    }
    file.seek(SeekFrom::Start(offset)).await?;
    let to_read = (len - offset) as usize;
    let mut buf = vec![0_u8; to_read];
    let n = file.read(&mut buf).await?;
    buf.truncate(n);
    Ok((buf, offset + u64::try_from(n).unwrap_or(0)))
}

fn emit_done(
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    ctx: &LiveReportContext,
    source_status: &str,
    report_text: String,
) {
    let message = report_text.clone();
    let _ = tx.send(BizReportStreamMsg::Done(BizAdviceReportPayload {
        task_id: ctx.session_id.clone(),
        source_request_id: ctx.session_id.clone(),
        source_ds_id: ctx.ds_id,
        source_status: source_status.to_string(),
        report_text: Some(report_text),
        report_json: Some(json!({
            "sessionId": ctx.session_id,
            "turnId": ctx.turn_id,
            "message": message,
        })),
    }));
}

fn emit_error(tx: &mpsc::UnboundedSender<BizReportStreamMsg>, detail: impl Into<String>) {
    let _ = tx.send(BizReportStreamMsg::Error(detail.into()));
}

/// Poll spill then fall back to formal jsonl / task output; SSE uses existing `biz.report.*` events.
pub fn spawn_live_report_sse_worker(
    state: Arc<AppState>,
    ctx: LiveReportContext,
) -> mpsc::UnboundedReceiver<BizReportStreamMsg> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let spill_path = assistant_stream_spill_path(&ctx.session_home, &ctx.turn_id);
        let mut spill_offset: u64 = 0;
        let mut emitted_spill_len: usize = 0;
        let started = Instant::now();
        let max_wait = Duration::from_secs(state.cfg.default_timeout_seconds.saturating_add(60));

        loop {
            if started.elapsed() > max_wait {
                emit_error(&tx, "live report stream timed out");
                return;
            }

            let status = match turn_status(
                &state.session_db,
                &ctx.turn_id,
                &ctx.session_id,
                ctx.ds_id,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    emit_error(&tx, e.detail());
                    return;
                }
            };

            if status.as_deref() == Some("failed") {
                emit_error(&tx, "solve turn failed");
                return;
            }
            if status.as_deref() == Some("cancelled") {
                emit_error(&tx, "solve turn cancelled");
                return;
            }

            let formal = match resolve_formal_report_text(&state, &ctx).await {
                Ok(t) => t,
                Err(e) if e.status == axum::http::StatusCode::BAD_REQUEST => String::new(),
                Err(e) => {
                    emit_error(&tx, e.detail());
                    return;
                }
            };

            let mut switch_to_formal = false;
            if status.as_deref() == Some("succeeded") && !formal.trim().is_empty() {
                switch_to_formal = true;
            }

            if spill_path.is_file() {
                match read_spill_from_offset(&spill_path, spill_offset).await {
                    Ok((chunk, new_off)) => {
                        spill_offset = new_off;
                        if spill_bytes_contain_end_marker(&chunk) {
                            switch_to_formal = true;
                        }
                        if let Ok(s) = String::from_utf8(chunk) {
                            let (visible, saw_marker) = split_spill_end_marker(&s);
                            if saw_marker {
                                switch_to_formal = true;
                            }
                            let delta = if visible.len() > emitted_spill_len {
                                visible[emitted_spill_len..].to_string()
                            } else {
                                String::new()
                            };
                            emitted_spill_len = visible.len();
                            if !delta.is_empty() && !switch_to_formal {
                                let _ = tx.send(BizReportStreamMsg::Delta(delta));
                            }
                        }
                    }
                    Err(e) => {
                        emit_error(&tx, format!("read spill failed: {e}"));
                        return;
                    }
                }
            } else if !formal.trim().is_empty()
                && status
                    .as_deref()
                    .is_some_and(|s| is_terminal_turn_status(s))
            {
                switch_to_formal = true;
            }

            if switch_to_formal {
                let report = if formal.trim().is_empty() {
                    match resolve_formal_report_text(&state, &ctx).await {
                        Ok(t) => t,
                        Err(e) => {
                            emit_error(&tx, e.detail());
                            return;
                        }
                    }
                } else {
                    formal
                };
                let st = status.as_deref().unwrap_or("succeeded");
                emit_done(&tx, &ctx, st, report);
                return;
            }

            sleep(POLL_INTERVAL).await;
        }
    });
    rx
}

pub async fn live_report_json_response(
    state: &AppState,
    ctx: LiveReportContext,
) -> Result<(String, Value), ApiError> {
    let status = turn_status(&state.session_db, &ctx.turn_id, &ctx.session_id, ctx.ds_id).await?;
    let Some(status) = status else {
        return Err(ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "unknown turnId for session",
        ));
    };
    if !is_terminal_turn_status(&status) {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            format!("turn not finished yet (status: {status})"),
        ));
    }
    if status == "failed" {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "solve turn failed",
        ));
    }
    let report_text = resolve_formal_report_text(&state, &ctx).await?;
    let report_json = json!({
        "sessionId": ctx.session_id,
        "turnId": ctx.turn_id,
        "message": report_text,
    });
    Ok((report_text, report_json))
}
