//! Live business report from PostgreSQL `gateway_turn_live_chunks` + per-turn snapshot. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gateway_solve_turn::strip_report_start_marker;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};
use tracing::{info, warn};

use crate::biz_advice_report::{
    report_body_from_solve_output, sanitize_external_report_text, BizAdviceReportPayload,
    BizReportStreamMsg, ReportExportSanitizer,
};
use crate::live_report_audit;
use crate::live_report_ports::{LiveReportPort, SessionDbLiveReportAdapter};
use crate::session_db::{GatewaySessionDb, LiveChunkRow};
use crate::turn_live::LiveNotifyHub;
use crate::{ApiError, AppState};

/// Fallback when `LISTEN` is quiet but ingest may still be running. Author: kejiqing
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Max Unicode scalars per `biz.report.delta` so one poll burst is not a single huge frame.
const MAX_DELTA_CHARS: usize = 128;

#[derive(Debug, Clone)]
pub struct LiveReportContext {
    pub session_id: String,
    pub turn_id: String,
    pub ds_id: i64,
    pub session_home: PathBuf,
}

/// Pure merge of `gateway_turns` snapshot fields — same policy as [`resolve_formal_report_text`].
/// `report_message` wins when non-empty after trim; else `output_json` via [`report_body_from_solve_output`].
/// Unit-test this without `PostgreSQL` or a full gateway `AppState`. Author: kejiqing
#[must_use]
pub fn formal_report_text_from_db_snapshot(
    report_message: Option<&str>,
    output_json: Option<&Value>,
) -> Option<String> {
    if let Some(text) = report_message {
        if !text.trim().is_empty() {
            return Some(strip_report_start_marker(text));
        }
    }
    if let Some(json) = output_json {
        if let Ok(body) = report_body_from_solve_output("", Some(json)) {
            if !body.trim().is_empty() {
                return Some(strip_report_start_marker(&body));
            }
        }
    }
    None
}

/// Resolve formal report text from `gateway_turns` only (`report_message` or `output_json.message`).
/// Does not read in-memory task state or session jsonl (avoids multi-turn transcript merge). Author: kejiqing
pub async fn resolve_formal_report_text(
    state: &AppState,
    ctx: &LiveReportContext,
) -> Result<String, ApiError> {
    let report_message = match state
        .session_db
        .get_turn_report_message(&ctx.turn_id, &ctx.session_id, ctx.ds_id)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("turn report_message query failed: {e}"),
            ));
        }
    };

    let output_json = if report_message
        .as_ref()
        .is_some_and(|t| !t.trim().is_empty())
    {
        None
    } else {
        match state
            .session_db
            .get_turn_output_json(&ctx.turn_id, &ctx.session_id, ctx.ds_id)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return Err(ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("turn output_json query failed: {e}"),
                ));
            }
        }
    };

    formal_report_text_from_db_snapshot(report_message.as_deref(), output_json.as_ref()).ok_or_else(
        || {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "formal report not ready (no turn snapshot in database)",
            )
        },
    )
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

/// Live report path: worker SSE relay (preferred) or legacy PG tail. Author: kejiqing
pub async fn should_use_live_pg_report(
    state: &AppState,
    ctx: &LiveReportContext,
) -> Result<bool, ApiError> {
    if state.report_relay.has_report(&ctx.turn_id)
        || state.report_relay.is_active(&ctx.turn_id)
    {
        return Ok(true);
    }
    if state.cfg.live_biz_report_spill_enabled {
        let status =
            turn_status(&state.session_db, &ctx.turn_id, &ctx.session_id, ctx.ds_id).await?;
        if matches!(status.as_deref(), Some("running") | Some("queued")) {
            return Ok(true);
        }
    }
    if state
        .session_db
        .turn_has_live_chunks(&ctx.turn_id)
        .await
        .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        return Ok(true);
    }
    Ok(false)
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

/// UTF-8-safe slices of `full_export[emitted..]` (≤ [`MAX_DELTA_CHARS`] scalars each). Author: kejiqing
fn collect_export_delta_pieces(full_export: &str, emitted_len: &mut usize) -> Vec<String> {
    let mut pieces = Vec::new();
    while *emitted_len < full_export.len() {
        let rest = &full_export[*emitted_len..];
        let chunk_end = rest
            .char_indices()
            .nth(MAX_DELTA_CHARS)
            .map_or(rest.len(), |(byte_idx, _)| byte_idx);
        let piece = &rest[..chunk_end];
        *emitted_len += chunk_end;
        if !piece.is_empty() {
            pieces.push(piece.to_string());
        }
    }
    pieces
}

/// Emit `full_export[emitted..]` as multiple SSE deltas (UTF-8 safe, `max_chars` per frame).
fn emit_export_deltas(
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    full_export: &str,
    emitted_len: &mut usize,
    delta_sent: &mut String,
) {
    for piece in collect_export_delta_pieces(full_export, emitted_len) {
        delta_sent.push_str(&piece);
        let _ = tx.send(BizReportStreamMsg::Delta(piece));
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

/// After PG live deltas, avoid replaying almost the whole formal report on whitespace/format drift.
fn should_skip_formal_flush_after_live_pg(delta_sent: &str, report: &str) -> bool {
    if delta_sent.is_empty() {
        return false;
    }
    if delta_sent == report {
        return true;
    }
    delta_sent.len().saturating_add(64) >= report.len()
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

/// PG live rows → SSE deltas (per-chunk export; avoids cumulative+marker cursor stall). Author: kejiqing
fn export_pg_chunk_rows(
    turn_id: &str,
    phase: &'static str,
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    rows: &[LiveChunkRow],
    sanitizer: &mut ReportExportSanitizer,
    delta_sent: &mut String,
    last_emitted_seq: &mut i64,
) {
    let trace = live_report_audit::enabled();
    for row in rows {
        *last_emitted_seq = row.seq;
        let piece = sanitizer.push_chunk(&row.chunk);
        if trace {
            let emitted_at_ms = live_report_audit::now_ms();
            let lag_ms = emitted_at_ms.saturating_sub(row.created_at_ms);
            info!(
                target: "claw_gateway_orchestration",
                component = "biz_advice_report_live",
                phase,
                turn_id = %turn_id,
                seq = row.seq,
                pg_created_at_ms = row.created_at_ms,
                sse_emitted_at_ms = emitted_at_ms,
                lag_ms,
                chunk_len = row.chunk.len(),
                export_len = piece.len(),
                delta_sent_len = delta_sent.len(),
                "live report SSE emitted PG chunk"
            );
        }
        if piece.is_empty() {
            continue;
        }
        let mut emitted = 0usize;
        emit_export_deltas(tx, &piece, &mut emitted, delta_sent);
    }
}

fn log_sse_chunk_batch(turn_id: &str, phase: &'static str, after_seq: i64, rows: &[LiveChunkRow]) {
    if rows.is_empty() {
        return;
    }
    let min_seq = rows.first().map(|r| r.seq).unwrap_or(0);
    let max_seq = rows.last().map(|r| r.seq).unwrap_or(0);
    info!(
        target: "claw_gateway_orchestration",
        component = "biz_advice_report_live",
        phase,
        turn_id = %turn_id,
        after_seq,
        row_count = rows.len(),
        min_seq,
        max_seq,
        logged_at_ms = live_report_audit::now_ms(),
        "live report SSE PG batch"
    );
}

async fn query_live_chunks_since(
    port: &dyn LiveReportPort,
    turn_id: &str,
    phase: &'static str,
    after_seq: i64,
) -> Result<Vec<LiveChunkRow>, String> {
    let query_start_ms = live_report_audit::now_ms();
    let rows = port.stream_live_chunks_since(turn_id, after_seq).await?;
    let query_done_ms = live_report_audit::now_ms();
    if live_report_audit::enabled() {
        let min_seq = rows.first().map(|r| r.seq).unwrap_or(0);
        let max_seq = rows.last().map(|r| r.seq).unwrap_or(0);
        info!(
            target: "claw_gateway_orchestration",
            component = "biz_advice_report_live",
            phase,
            turn_id = %turn_id,
            after_seq,
            row_count = rows.len(),
            min_seq,
            max_seq,
            query_start_ms,
            query_done_ms,
            query_elapsed_ms = query_done_ms.saturating_sub(query_start_ms),
            "live report SSE PG query"
        );
    }
    Ok(rows)
}

/// Bootstrap rows + max `seq` (tail uses `seq > last_emitted_seq`). Author: kejiqing
async fn bootstrap_pg_rows(
    port: &dyn LiveReportPort,
    turn_id: &str,
) -> Result<(Vec<LiveChunkRow>, i64), String> {
    let rows = query_live_chunks_since(port, turn_id, "sse_bootstrap_query", 0).await?;
    let last_emitted_seq = rows.last().map(|r| r.seq).unwrap_or(0);
    Ok((rows, last_emitted_seq))
}

async fn try_finish_formal_port(
    port: &dyn LiveReportPort,
    ctx: &LiveReportContext,
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    status: &Option<String>,
    delta_sent: &mut String,
) -> bool {
    let formal = match port
        .formal_report_text(&ctx.turn_id, &ctx.session_id, ctx.ds_id)
        .await
    {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            emit_error(tx, e);
            return true;
        }
    };
    if status.as_deref() == Some("succeeded") && !formal.trim().is_empty() {
        let report = sanitize_external_report_text(&formal);
        // PG live already streamed the body; skip formal re-delta when only minor drift (e.g. `\n`).
        if delta_sent.as_str() != report && !should_skip_formal_flush_after_live_pg(delta_sent, &report) {
            flush_remaining_deltas(tx, &report, delta_sent);
        }
        let st = status.as_deref().unwrap_or("succeeded");
        emit_done(tx, ctx, st, &report);
        return true;
    }
    false
}

/// SSE worker dependencies (mock [`LiveReportPort`] in tests). Author: kejiqing
pub struct LiveReportWorkerDeps {
    pub port: Arc<dyn LiveReportPort>,
    pub notify_hub: Arc<LiveNotifyHub>,
    pub max_wait: Duration,
}

/// `LISTEN/NOTIFY` + `SELECT` live chunks; terminal formal from turn snapshot. Author: kejiqing
pub fn spawn_live_report_sse_worker(
    state: Arc<AppState>,
    ctx: LiveReportContext,
) -> mpsc::UnboundedReceiver<BizReportStreamMsg> {
    spawn_live_report_sse_worker_deps(
        LiveReportWorkerDeps {
            port: Arc::new(SessionDbLiveReportAdapter(Arc::clone(&state.session_db))),
            notify_hub: Arc::clone(&state.live_notify_hub),
            max_wait: Duration::from_secs(state.cfg.default_timeout_seconds.saturating_add(60)),
        },
        ctx,
    )
}

pub fn spawn_live_report_sse_worker_deps(
    deps: LiveReportWorkerDeps,
    ctx: LiveReportContext,
) -> mpsc::UnboundedReceiver<BizReportStreamMsg> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let notify = deps.notify_hub.subscribe(&ctx.turn_id).await;
        let bootstrap = bootstrap_pg_rows(deps.port.as_ref(), &ctx.turn_id).await;
        let (bootstrap_rows, mut last_emitted_seq) = match bootstrap {
            Ok(s) => s,
            Err(e) => {
                emit_error(&tx, e);
                return;
            }
        };
        let mut delta_sent = String::new();
        let mut export_sanitizer = ReportExportSanitizer::new(true);
        info!(
            target: "claw_gateway_orchestration",
            component = "biz_advice_report_live",
            phase = "sse_worker_start",
            turn_id = %ctx.turn_id,
            session_id = %ctx.session_id,
            started_at_ms = live_report_audit::now_ms(),
            "live report SSE worker started"
        );
        log_sse_chunk_batch(&ctx.turn_id, "sse_bootstrap_batch", 0, &bootstrap_rows);
        export_pg_chunk_rows(
            &ctx.turn_id,
            "sse_bootstrap_emit",
            &tx,
            &bootstrap_rows,
            &mut export_sanitizer,
            &mut delta_sent,
            &mut last_emitted_seq,
        );
        let started = Instant::now();
        let mut loop_wake_reason = "after_bootstrap";

        loop {
            if live_report_audit::enabled() {
                info!(
                    target: "claw_gateway_orchestration",
                    component = "biz_advice_report_live",
                    phase = "sse_loop_wake",
                    turn_id = %ctx.turn_id,
                    wake_reason = loop_wake_reason,
                    wake_at_ms = live_report_audit::now_ms(),
                    last_emitted_seq,
                    "live report SSE loop wake"
                );
            }

            if started.elapsed() > deps.max_wait {
                emit_error(&tx, "live report stream timed out");
                return;
            }

            let status = match deps
                .port
                .turn_status(&ctx.turn_id, &ctx.session_id, ctx.ds_id)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    emit_error(&tx, e);
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

            if try_finish_formal_port(deps.port.as_ref(), &ctx, &tx, &status, &mut delta_sent).await
            {
                return;
            }

            let rows = match query_live_chunks_since(
                deps.port.as_ref(),
                &ctx.turn_id,
                "sse_tail_query",
                last_emitted_seq,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    emit_error(&tx, format!("live chunks query failed: {e}"));
                    return;
                }
            };
            if !rows.is_empty() {
                log_sse_chunk_batch(&ctx.turn_id, "sse_tail_batch", last_emitted_seq, &rows);
                export_pg_chunk_rows(
                    &ctx.turn_id,
                    "sse_tail_emit",
                    &tx,
                    &rows,
                    &mut export_sanitizer,
                    &mut delta_sent,
                    &mut last_emitted_seq,
                );
            } else if try_finish_formal_port(deps.port.as_ref(), &ctx, &tx, &status, &mut delta_sent)
                .await
            {
                return;
            }

            loop_wake_reason = if tokio::select! {
                _ = notify.notified() => true,
                _ = sleep(STATUS_POLL_INTERVAL) => false,
            } {
                "pg_notify"
            } else {
                "poll_timer_2s"
            };
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
    fn skip_formal_flush_when_live_pg_already_streamed_most_of_report() {
        let spill = "a".repeat(100);
        let formal = format!("{spill}\n");
        assert!(should_skip_formal_flush_after_live_pg(&spill, &formal));
        assert!(!should_skip_formal_flush_after_live_pg("", &formal));
        assert!(!should_skip_formal_flush_after_live_pg("tiny", &"x".repeat(500)));
    }

    #[test]
    fn cumulative_sanitize_stalls_when_marker_strips_prefix() {
        let marker = gateway_solve_turn::ASSISTANT_STREAM_REPORT_START_MARKER;
        let mut cumulative = String::from("thinking-body");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut emitted = cumulative.len();
        let mut delta_sent = cumulative.clone();
        emit_export_deltas(&tx, &cumulative, &mut emitted, &mut delta_sent);
        cumulative.push_str(marker);
        cumulative.push_str("\n# report");
        let visible = sanitize_external_report_text(&cumulative);
        assert!(emitted > visible.len());
        emit_export_deltas(&tx, &visible, &mut emitted, &mut delta_sent);
        assert!(
            rx.try_recv().is_err(),
            "cumulative+sanitize path must stall after marker shrinks visible"
        );
    }

    #[test]
    fn export_pg_chunk_rows_emits_after_marker_without_stall() {
        let marker = gateway_solve_turn::ASSISTANT_STREAM_REPORT_START_MARKER;
        let rows = vec![
            LiveChunkRow {
                seq: 1,
                chunk: "thinking-body".into(),
                created_at_ms: 0,
            },
            LiveChunkRow {
                seq: 2,
                chunk: format!("{marker}\n# report"),
                created_at_ms: 0,
            },
        ];
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut sanitizer = ReportExportSanitizer::new(true);
        let mut delta_sent = String::new();
        let mut last_seq = 0i64;
        export_pg_chunk_rows(
            "T_test",
            "sse_test_emit",
            &tx,
            &rows,
            &mut sanitizer,
            &mut delta_sent,
            &mut last_seq,
        );
        let mut deltas = Vec::new();
        while let Ok(BizReportStreamMsg::Delta(d)) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(last_seq, 2);
        assert_eq!(deltas.join(""), "thinking-body# report");
    }

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
    fn formal_db_snapshot_prefers_report_message_over_output_json() {
        let j = json!({"message": "ignored"});
        assert_eq!(
            formal_report_text_from_db_snapshot(Some("body-from-column"), Some(&j)).as_deref(),
            Some("body-from-column")
        );
    }

    #[test]
    fn formal_db_snapshot_falls_back_to_output_json_message() {
        assert_eq!(
            formal_report_text_from_db_snapshot(None, Some(&json!({"message": "from-json"})))
                .as_deref(),
            Some("from-json")
        );
    }

    #[test]
    fn formal_db_snapshot_whitespace_only_message_uses_json() {
        assert_eq!(
            formal_report_text_from_db_snapshot(
                Some(" \t "),
                Some(&json!({"message": "from-json"})),
            )
            .as_deref(),
            Some("from-json")
        );
    }

    #[test]
    fn formal_db_snapshot_none_when_no_usable_fields() {
        assert!(formal_report_text_from_db_snapshot(None, None).is_none());
        assert!(formal_report_text_from_db_snapshot(None, Some(&json!({}))).is_none());
        assert!(formal_report_text_from_db_snapshot(None, Some(&json!({"message": ""}))).is_none());
    }

    #[test]
    fn formal_db_snapshot_strips_internal_start_marker() {
        let marked = format!(
            "{}\n# 标题",
            gateway_solve_turn::ASSISTANT_STREAM_REPORT_START_MARKER
        );
        assert_eq!(
            formal_report_text_from_db_snapshot(Some(marked.as_str()), None).as_deref(),
            Some("# 标题")
        );
        assert_eq!(
            formal_report_text_from_db_snapshot(None, Some(&json!({"message": marked}))).as_deref(),
            Some("# 标题")
        );
    }

    #[tokio::test]
    async fn live_report_sse_worker_emits_delta_then_done_with_mock_port() {
        use crate::live_report_mocks::MockLiveReportPort;

        let mock = Arc::new(MockLiveReportPort::default());
        let mock_ctl = Arc::clone(&mock);
        *mock.status.lock().await = Some("running".into());
        let port: Arc<dyn LiveReportPort> = mock.clone();
        let hub = Arc::new(crate::turn_live::LiveNotifyHub::new());
        let tid = "T_10000000000000000000000000000001";
        let ctx = LiveReportContext {
            session_id: "sess-mock".into(),
            turn_id: tid.into(),
            ds_id: 1,
            session_home: std::path::PathBuf::from("/tmp"),
        };
        let mut rx = spawn_live_report_sse_worker_deps(
            LiveReportWorkerDeps {
                port: Arc::clone(&port),
                notify_hub: Arc::clone(&hub),
                max_wait: Duration::from_secs(30),
            },
            ctx,
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        mock.push_chunk(tid, 1, "流式").await;
        hub.signal_turn(tid).await;
        let mut saw_delta = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if let Ok(BizReportStreamMsg::Delta(d)) = rx.try_recv() {
                if !d.is_empty() {
                    saw_delta = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(saw_delta, "mock live chunks should produce biz.report.delta");
        mock_ctl.set_succeeded("流式正文").await;
        hub.signal_turn(tid).await;
        let mut saw_done = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if let Ok(BizReportStreamMsg::Done(payload)) = rx.try_recv() {
                assert_eq!(payload.report_text.as_deref(), Some("流式正文"));
                saw_done = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(saw_done, "mock formal snapshot + signal_turn should yield Done");
    }
}
