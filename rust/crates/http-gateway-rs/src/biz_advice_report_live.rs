//! Live business report from assistant stream spill + session jsonl (no polish LLM). Author: kejiqing

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gateway_solve_turn::{
    assistant_stream_spill_path, final_assistant_report_text_from_jsonl,
    final_assistant_report_text_from_jsonl_for_user_turn_index, spill_bytes_contain_end_marker,
    split_spill_end_marker, strip_report_start_marker, ASSISTANT_STREAM_REPORT_START_MARKER,
};
use serde_json::{json, Value};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};
use tracing::warn;

use crate::biz_advice_report::{
    report_body_from_solve_output, sanitize_external_report_text, BizAdviceReportPayload,
    BizReportStreamMsg,
};
use crate::session_db::GatewaySessionDb;
use crate::{ApiError, AppState};

/// Whether this turn's assistant stream spill file exists on disk.
#[must_use]
pub fn turn_spill_file_exists(session_home: &Path, turn_id: &str) -> bool {
    assistant_stream_spill_path(session_home, turn_id).is_file()
}

/// Live spill-tail SSE when spill exists and contains the report start marker.
/// Gated by gateway env `CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1` (spill write + report SSE + `hasReport`).
#[must_use]
pub fn turn_use_live_spill_report(session_home: &Path, turn_id: &str) -> bool {
    turn_spill_file_exists(session_home, turn_id)
        && gateway_solve_turn::spill_contains_report_start_marker(session_home, turn_id)
}

/// Poll spill growth frequently enough to tail model output smoothly.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Max Unicode scalars per `biz.report.delta` so one poll burst is not a single huge frame.
const MAX_DELTA_CHARS: usize = 128;

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

    match state
        .session_db
        .get_turn_report_message(&ctx.turn_id, &ctx.session_id, ctx.ds_id)
        .await
    {
        Ok(Some(text)) if !text.trim().is_empty() => {
            return Ok(strip_report_start_marker(&text));
        }
        Ok(_) => {}
        Err(e) => {
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("turn report_message query failed: {e}"),
            ));
        }
    }

    if let Ok(Some(t_ms)) = state
        .session_db
        .get_turn_created_at_ms(&ctx.turn_id, &ctx.session_id, ctx.ds_id)
        .await
    {
        if let Ok(idx) = state
            .session_db
            .turn_index_in_session(&ctx.turn_id, &ctx.session_id, ctx.ds_id, t_ms)
            .await
        {
            let idx_usize = usize::try_from(idx).unwrap_or(1);
            let home = ctx.session_home.clone();
            let scoped = tokio::task::spawn_blocking(move || {
                final_assistant_report_text_from_jsonl_for_user_turn_index(&home, idx_usize)
            })
            .await
            .map_err(|e| {
                ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("report scoped jsonl join failed: {e}"),
                )
            })?
            .map_err(|detail| {
                ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("read scoped session report failed: {detail}"),
                )
            })?;
            if !scoped.trim().is_empty() {
                return Ok(strip_report_start_marker(&scoped));
            }
        }
    }

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
    Ok(strip_report_start_marker(&text))
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
    report_text: &str,
) {
    let report_text = sanitize_external_report_text(report_text);
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

/// Consumer-visible report body from cumulative spill (after end-marker split).
#[must_use]
fn spill_visible_export(visible: &str) -> String {
    if !visible.contains(ASSISTANT_STREAM_REPORT_START_MARKER) {
        return String::new();
    }
    sanitize_external_report_text(visible)
}

/// Emit `full_export[emitted..]` as multiple SSE deltas (UTF-8 safe, `max_chars` per frame).
fn emit_export_deltas(
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    full_export: &str,
    emitted_len: &mut usize,
    delta_sent: &mut String,
) {
    while *emitted_len < full_export.len() {
        let rest = &full_export[*emitted_len..];
        let chunk_end = rest
            .char_indices()
            .nth(MAX_DELTA_CHARS)
            .map_or(rest.len(), |(byte_idx, _)| byte_idx);
        let piece = &rest[..chunk_end];
        *emitted_len += chunk_end;
        if !piece.is_empty() {
            delta_sent.push_str(piece);
            let _ = tx.send(BizReportStreamMsg::Delta(piece.to_string()));
        }
    }
}

#[must_use]
fn longest_common_prefix_len(a: &str, b: &str) -> usize {
    let mut last_end = 0usize;
    for ((ia, ca), (_, cb)) in a.char_indices().zip(b.char_indices()) {
        if ca != cb {
            break;
        }
        last_end = ia + ca.len_utf8();
    }
    last_end
}

/// Before `done`, ensure SSE deltas cover all of `report` (formal jsonl may be longer than spill).
fn flush_remaining_deltas(
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    report: &str,
    delta_sent: &mut String,
) {
    if report.is_empty() {
        return;
    }
    if delta_sent == report {
        return;
    }
    let mut emit_from = if report.starts_with(delta_sent.as_str()) {
        delta_sent.len()
    } else {
        let common = longest_common_prefix_len(report, delta_sent);
        if common < delta_sent.len() {
            warn!(
                target: "claw_gateway_orchestration",
                component = "biz_advice_report_live",
                delta_sent_len = delta_sent.len(),
                report_len = report.len(),
                common_prefix_len = common,
                "spill deltas diverged from formal report; resyncing tail from common prefix"
            );
            delta_sent.truncate(common);
        }
        common
    };
    emit_export_deltas(tx, report, &mut emit_from, delta_sent);
    debug_assert_eq!(delta_sent.as_str(), report);
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
        let mut raw_spill = String::new();
        let mut emitted_export_len: usize = 0;
        let mut delta_sent = String::new();
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
                        if !chunk.is_empty() {
                            let piece = String::from_utf8_lossy(&chunk);
                            raw_spill.push_str(&piece);
                        }
                        let (visible, saw_end) = split_spill_end_marker(&raw_spill);
                        if saw_end {
                            switch_to_formal = true;
                        }
                        let full_export = spill_visible_export(&visible);
                        if !switch_to_formal {
                            emit_export_deltas(
                                &tx,
                                &full_export,
                                &mut emitted_export_len,
                                &mut delta_sent,
                            );
                        }
                    }
                    Err(e) => {
                        emit_error(&tx, format!("read spill failed: {e}"));
                        return;
                    }
                }
            } else if !formal.trim().is_empty()
                && status.as_deref().is_some_and(is_terminal_turn_status)
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
                let report = sanitize_external_report_text(&report);
                flush_remaining_deltas(&tx, &report, &mut delta_sent);
                let st = status.as_deref().unwrap_or("succeeded");
                emit_done(&tx, &ctx, st, &report);
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
    let report_text = resolve_formal_report_text(state, &ctx).await?;
    let report_json = json!({
        "sessionId": ctx.session_id,
        "turnId": ctx.turn_id,
        "message": report_text,
    });
    Ok((report_text, report_json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_deltas_splits_by_max_chars() {
        let text: String = "字".repeat(300);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut emitted = 0usize;
        let mut delta_sent = String::new();
        emit_export_deltas(&tx, &text, &mut emitted, &mut delta_sent);
        assert_eq!(emitted, text.len());
        assert_eq!(delta_sent, text);
        let mut deltas = Vec::new();
        while let Ok(BizReportStreamMsg::Delta(d)) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas.len(), 3);
        assert_eq!(deltas[0].chars().count(), MAX_DELTA_CHARS);
        assert_eq!(deltas[1].chars().count(), MAX_DELTA_CHARS);
        assert_eq!(deltas[2].chars().count(), 300 - 2 * MAX_DELTA_CHARS);
    }

    #[test]
    fn flush_remaining_emits_formal_tail_after_partial_spill() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let spill_part = "报告开头";
        let full_report = "报告开头报告结尾";
        let mut delta_sent = spill_part.to_string();
        let mut emitted = spill_part.len();
        emit_export_deltas(&tx, spill_part, &mut emitted, &mut delta_sent);
        while rx.try_recv().is_ok() {}
        flush_remaining_deltas(&tx, full_report, &mut delta_sent);
        let mut tail = String::new();
        while let Ok(BizReportStreamMsg::Delta(d)) = rx.try_recv() {
            tail.push_str(&d);
        }
        assert_eq!(tail, "报告结尾");
        assert_eq!(delta_sent, full_report);
    }

    #[test]
    fn spill_visible_export_strips_marker() {
        let raw = format!("{ASSISTANT_STREAM_REPORT_START_MARKER}\n# 标题\n正文");
        assert_eq!(spill_visible_export(&raw), "# 标题\n正文");
        assert!(spill_visible_export("分析中…").is_empty());
    }
}
