//! Live SSE from pool-local stdout hub (`GET /v1/biz_advice_report/live`). Author: kejiqing
//!
//! Router and normal turns share one path: subscribe the **router/own** turn Hub only.
//! Specialist body reaches that Hub via worker `delegate_project_tool` passthrough
//! (`report.delta` on router stdout). Author: kejiqing

use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::biz_advice_report::{
    biz_report_sse_event_stream, sanitize_external_report_text, BizAdviceReportPayload,
    BizReportDeltaChunk, BizReportStreamMsg,
};
use crate::pool::live_report_hub::{HubDeltaChunk, HubMsg, LiveReportHub};

pub fn live_report_sse_response(
    hub: Arc<LiveReportHub>,
    turn_id: &str,
    task_id: String,
    source_request_id: String,
    source_proj_id: i64,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<BizReportStreamMsg>();
    let turn_id_worker = turn_id.to_string();
    let hub_done = Arc::clone(&hub);
    tokio::spawn(async move {
        let mut acc = String::new();
        let _ = follow_turn_deltas(
            hub.as_ref(),
            &turn_id_worker,
            &tx,
            &mut acc,
            std::future::pending::<()>(),
        )
        .await;
        let text = if acc.is_empty() {
            hub.snapshot_text(&turn_id_worker)
        } else {
            acc
        };
        send_done(&tx, &task_id, &source_request_id, source_proj_id, &text);
        hub_done.try_remove_turn(&turn_id_worker);
    });
    sse_response(turn_id, rx)
}

fn sse_response(stream_id: &str, rx: mpsc::UnboundedReceiver<BizReportStreamMsg>) -> Response {
    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = HeaderValue::from_static("no");
    (
        AppendHeaders([(no_buffer, no_buffer_val)]),
        Sse::new(biz_report_sse_event_stream(stream_id, rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowEnd {
    HubDone,
    Rebound,
}

async fn follow_turn_deltas(
    hub: &LiveReportHub,
    turn_id: &str,
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    acc: &mut String,
    rebound: impl std::future::Future<Output = ()>,
) -> FollowEnd {
    tokio::pin!(rebound);
    let (mut sub, snapshot_chunks) = hub.subscribe_with_snapshot(turn_id);
    for chunk in snapshot_chunks {
        if chunk.text.is_empty() {
            continue;
        }
        acc.push_str(&chunk.text);
        if tx
            .send(BizReportStreamMsg::Delta(BizReportDeltaChunk {
                text: chunk.text,
                emit_seq: chunk.emit_seq,
            }))
            .is_err()
        {
            return FollowEnd::HubDone;
        }
    }
    if hub.is_solve_done(turn_id) {
        return FollowEnd::HubDone;
    }
    loop {
        tokio::select! {
            biased;
            () = &mut rebound => {
                return FollowEnd::Rebound;
            }
            msg = sub.recv() => {
                match msg {
                    Ok(HubMsg::Delta(HubDeltaChunk { text, emit_seq })) => {
                        if text.is_empty() {
                            continue;
                        }
                        acc.push_str(&text);
                        if tx
                            .send(BizReportStreamMsg::Delta(BizReportDeltaChunk {
                                text,
                                emit_seq,
                            }))
                            .is_err()
                        {
                            return FollowEnd::HubDone;
                        }
                    }
                    Ok(HubMsg::SolveDone) | Err(RecvError::Closed) => {
                        return FollowEnd::HubDone;
                    }
                    Err(RecvError::Lagged(_)) => {
                        if hub.is_solve_done(turn_id) {
                            return FollowEnd::HubDone;
                        }
                    }
                }
            }
        }
    }
}

fn send_done(
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    task_id: &str,
    source_request_id: &str,
    source_proj_id: i64,
    text: &str,
) {
    let final_text = sanitize_external_report_text(text);
    let done = BizAdviceReportPayload {
        task_id: task_id.to_string(),
        source_request_id: source_request_id.to_string(),
        source_proj_id,
        source_status: "running".into(),
        report_text: Some(final_text.clone()),
        report_json: Some(json!({ "message": final_text })),
    };
    let _ = tx.send(BizReportStreamMsg::Done(done));
}

#[cfg(test)]
mod tests {
    use super::{follow_turn_deltas, send_done, BizReportStreamMsg, FollowEnd};
    use crate::pool::live_report_hub::LiveReportHub;
    use crate::session_db::ActiveDelegateRecord;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::{mpsc, watch};
    use tokio::time::{timeout, Duration};

    fn delta(hub: &LiveReportHub, turn_id: &str, text: &str) {
        hub.ingest_json(turn_id, &json!({ "ev": "report.delta", "text": text }));
    }

    fn solve_done(hub: &LiveReportHub, turn_id: &str) {
        hub.ingest_json(turn_id, &json!({ "ev": "solve.done" }));
    }

    async fn next_delta(rx: &mut mpsc::UnboundedReceiver<BizReportStreamMsg>) -> String {
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Delta(t))) => t.text,
            _ => panic!("expected delta, got non-delta or timeout"),
        }
    }

    #[tokio::test]
    async fn follow_turn_deltas_replays_snapshot_then_exits_on_solve_done() {
        let hub = LiveReportHub::default();
        delta(&hub, "T_snap", "snap-");
        delta(&hub, "T_snap", "shot");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub_task = hub.clone();
        let join = tokio::spawn(async move {
            let mut acc = String::new();
            let end = follow_turn_deltas(
                &hub_task,
                "T_snap",
                &tx,
                &mut acc,
                std::future::pending::<()>(),
            )
            .await;
            (end, acc)
        });
        assert_eq!(next_delta(&mut rx).await, "snap-");
        assert_eq!(next_delta(&mut rx).await, "shot");
        solve_done(&hub, "T_snap");
        let (end, acc) = join.await.expect("join");
        assert_eq!(end, FollowEnd::HubDone);
        assert_eq!(acc, "snap-shot");
    }

    #[tokio::test]
    async fn follow_turn_deltas_returns_rebound_when_signaled() {
        let hub = LiveReportHub::default();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (trigger_tx, trigger_rx) = watch::channel(false);
        let hub_for_task = hub.clone();
        let join = tokio::spawn(async move {
            let mut acc = String::new();
            let rebound = async move {
                let mut rx = trigger_rx;
                loop {
                    if *rx.borrow() {
                        return;
                    }
                    if rx.changed().await.is_err() {
                        return;
                    }
                }
            };
            let end = follow_turn_deltas(&hub_for_task, "T_reb", &tx, &mut acc, rebound).await;
            (end, acc)
        });
        delta(&hub, "T_reb", "live-");
        assert_eq!(next_delta(&mut rx).await, "live-");
        trigger_tx.send_replace(true);
        let (end, acc) = join.await.expect("join");
        assert_eq!(end, FollowEnd::Rebound);
        assert_eq!(acc, "live-");
    }

    #[tokio::test]
    async fn router_turn_streams_only_own_hub_until_done() {
        let hub = Arc::new(LiveReportHub::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let loop_hub = Arc::clone(&hub);
        let join = tokio::spawn(async move {
            let mut acc = String::new();
            let _ = follow_turn_deltas(
                loop_hub.as_ref(),
                "T_router_only",
                &tx,
                &mut acc,
                std::future::pending::<()>(),
            )
            .await;
            send_done(&tx, "task", "task", 99010, &acc);
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        // Specialist hub deltas must NOT appear on router SSE (worker passthrough copies
        // into router hub instead). Author: kejiqing
        delta(hub.as_ref(), "T_spec", "leak-");
        delta(hub.as_ref(), "T_router_only", "router-");
        delta(hub.as_ref(), "T_router_only", "only");
        assert_eq!(next_delta(&mut rx).await, "router-");
        assert_eq!(next_delta(&mut rx).await, "only");
        solve_done(hub.as_ref(), "T_router_only");
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(d))) => {
                assert_eq!(d.report_text.as_deref(), Some("router-only"));
            }
            _ => panic!("expected router done"),
        }
        join.await.expect("loop");
    }

    #[tokio::test]
    async fn worker_passthrough_serial_deltas_appear_once_on_router_hub() {
        let hub = Arc::new(LiveReportHub::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let loop_hub = Arc::clone(&hub);
        tokio::spawn(async move {
            let mut acc = String::new();
            let _ = follow_turn_deltas(
                loop_hub.as_ref(),
                "T_router2",
                &tx,
                &mut acc,
                std::future::pending::<()>(),
            )
            .await;
            send_done(&tx, "task", "task", 99010, &acc);
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        // Passthrough copies specialist body onto the router turn hub once each. Author: kejiqing
        delta(hub.as_ref(), "T_router2", "kb-");
        assert_eq!(next_delta(&mut rx).await, "kb-");
        delta(hub.as_ref(), "T_router2", "ops");
        assert_eq!(next_delta(&mut rx).await, "ops");
        solve_done(hub.as_ref(), "T_router2");
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(d))) => {
                assert_eq!(d.report_text.as_deref(), Some("kb-ops"));
            }
            _ => panic!("expected router done"),
        }
    }

    #[tokio::test]
    async fn specialist_hub_solve_done_does_not_close_router_sse() {
        let hub = Arc::new(LiveReportHub::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let loop_hub = Arc::clone(&hub);
        let join = tokio::spawn(async move {
            let mut acc = String::new();
            let _ = follow_turn_deltas(
                loop_hub.as_ref(),
                "T_router_keep",
                &tx,
                &mut acc,
                std::future::pending::<()>(),
            )
            .await;
            send_done(&tx, "task", "task", 99010, &acc);
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        delta(hub.as_ref(), "T_router_keep", "live-");
        assert_eq!(next_delta(&mut rx).await, "live-");
        // Specialist terminal must not end the user SSE (only router solve.done). Author: kejiqing
        solve_done(hub.as_ref(), "T_spec_other");
        tokio::time::sleep(Duration::from_millis(40)).await;
        match timeout(Duration::from_millis(80), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(_))) => {
                panic!("specialist solve.done must not emit biz.report.done on router SSE")
            }
            _ => {}
        }
        delta(hub.as_ref(), "T_router_keep", "tail");
        assert_eq!(next_delta(&mut rx).await, "tail");
        solve_done(hub.as_ref(), "T_router_keep");
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(d))) => {
                assert_eq!(d.report_text.as_deref(), Some("live-tail"));
            }
            _ => panic!("expected single router done"),
        }
        join.await.expect("join");
    }

    #[tokio::test]
    async fn specialist_hub_text_never_leaks_into_router_follow() {
        let hub = Arc::new(LiveReportHub::default());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let loop_hub = Arc::clone(&hub);
        tokio::spawn(async move {
            let mut acc = String::new();
            let _ = follow_turn_deltas(
                loop_hub.as_ref(),
                "T_router_iso",
                &tx,
                &mut acc,
                std::future::pending::<()>(),
            )
            .await;
            send_done(&tx, "task", "task", 99010, &acc);
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        // Without worker passthrough copy, specialist hub content must stay invisible.
        delta(hub.as_ref(), "T_spec_iso", "SHOULD_NOT_SEE");
        delta(hub.as_ref(), "T_spec_iso", "ALSO_HIDDEN");
        delta(hub.as_ref(), "T_router_iso", "only-router");
        assert_eq!(next_delta(&mut rx).await, "only-router");
        solve_done(hub.as_ref(), "T_router_iso");
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(d))) => {
                assert_eq!(d.report_text.as_deref(), Some("only-router"));
                assert!(!d
                    .report_text
                    .as_deref()
                    .unwrap_or("")
                    .contains("SHOULD_NOT"));
            }
            _ => panic!("expected done"),
        }
    }

    #[test]
    fn active_delegate_record_from_stdout_value() {
        let v = json!({
            "ev": "delegate.active",
            "sessionId": "dgt_sess",
            "turnId": "T_spec_stdout",
            "projId": 99012,
            "delegateProjId": 99012
        });
        let rec = ActiveDelegateRecord::from_stdout_value(&v).expect("parse");
        assert_eq!(rec.session_id, "dgt_sess");
        assert_eq!(rec.turn_id, "T_spec_stdout");
        assert_eq!(rec.proj_id, 99012);
        assert_eq!(rec.delegate_proj_id, 99012);
    }

    #[test]
    fn active_delegate_record_from_stdout_rejects_empty_turn_id() {
        let v = json!({
            "ev": "delegate.active",
            "sessionId": "dgt_sess",
            "turnId": "  ",
            "projId": 99012
        });
        assert!(ActiveDelegateRecord::from_stdout_value(&v).is_none());
    }
}
