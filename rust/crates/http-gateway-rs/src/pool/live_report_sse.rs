//! Live SSE from pool-local stdout hub (`GET /v1/biz_advice_report/live`). Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderValue};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, watch};

use crate::biz_advice_report::{
    biz_report_sse_event_stream, sanitize_external_report_text, BizAdviceReportPayload,
    BizReportStreamMsg,
};
use crate::pool::live_report_hub::{HubMsg, LiveReportHub};
use crate::session_db::GatewaySessionDb;

const ACTIVE_DELEGATE_POLL: Duration = Duration::from_millis(250);

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

/// Router running SSE: same HTTP connection; rebind in-process hub when `activeDelegate` appears.
/// Specialist `SolveDone` does not emit `biz.report.done`. Author: kejiqing
pub fn router_fanin_live_sse_response(
    hub: Arc<LiveReportHub>,
    db: &Arc<GatewaySessionDb>,
    router_proj_id: i64,
    router_turn_id: &str,
    task_id: String,
    source_request_id: String,
    source_proj_id: i64,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<BizReportStreamMsg>();
    let router_turn = router_turn_id.to_string();
    let (active_tx, active_rx) = watch::channel(None);
    let poll_db = Arc::clone(db);
    let poll_turn = router_turn.clone();
    let poller = tokio::spawn(async move {
        loop {
            let next = crate::delegate_fanin::active_delegate_for_router_live(
                poll_db.as_ref(),
                router_proj_id,
                &poll_turn,
            )
            .await
            .ok()
            .flatten()
            .map(|a| a.turn_id);
            active_tx.send_replace(next);
            tokio::time::sleep(ACTIVE_DELEGATE_POLL).await;
        }
    });
    tokio::spawn(async move {
        run_router_fanin_loop(
            hub,
            router_turn,
            active_rx,
            tx,
            task_id,
            source_request_id,
            source_proj_id,
        )
        .await;
        poller.abort();
    });
    sse_response(router_turn_id, rx)
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

async fn wait_until_active_ne(rx: &mut watch::Receiver<Option<String>>, current: Option<&str>) {
    if rx.borrow().as_deref() != current {
        return;
    }
    while rx.changed().await.is_ok() {
        if rx.borrow().as_deref() != current {
            return;
        }
    }
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
        if chunk.is_empty() {
            continue;
        }
        acc.push_str(&chunk);
        if tx.send(BizReportStreamMsg::Delta(chunk)).is_err() {
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
                    Ok(HubMsg::Delta(chunk)) => {
                        if chunk.is_empty() {
                            continue;
                        }
                        acc.push_str(&chunk);
                        if tx.send(BizReportStreamMsg::Delta(chunk)).is_err() {
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

async fn run_router_fanin_loop(
    hub: Arc<LiveReportHub>,
    router_turn_id: String,
    mut active_rx: watch::Receiver<Option<String>>,
    tx: mpsc::UnboundedSender<BizReportStreamMsg>,
    task_id: String,
    source_request_id: String,
    source_proj_id: i64,
) {
    let mut acc = String::new();
    loop {
        let spec = active_rx.borrow().clone();
        let follow_id = spec.clone().unwrap_or_else(|| router_turn_id.clone());
        let following_spec = spec.is_some();
        tracing::info!(
            target: "claw_live_report",
            component = "router_fanin_sse",
            router_turn_id = %router_turn_id,
            follow_turn_id = %follow_id,
            following_specialist = following_spec,
            "biz_advice_report stream — in-process hub rebind"
        );
        let mut wait_rx = active_rx.clone();
        let spec_wait = spec.clone();
        let rebound = async move {
            wait_until_active_ne(&mut wait_rx, spec_wait.as_deref()).await;
        };
        match follow_turn_deltas(hub.as_ref(), &follow_id, &tx, &mut acc, rebound).await {
            FollowEnd::HubDone if following_spec => {
                wait_until_active_ne(&mut active_rx, spec.as_deref()).await;
            }
            FollowEnd::HubDone => {
                let text = if acc.is_empty() {
                    hub.snapshot_text(&router_turn_id)
                } else {
                    acc
                };
                send_done(&tx, &task_id, &source_request_id, source_proj_id, &text);
                hub.try_remove_turn(&router_turn_id);
                return;
            }
            FollowEnd::Rebound => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{run_router_fanin_loop, BizReportStreamMsg};
    use crate::pool::live_report_hub::LiveReportHub;
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
            Ok(Some(BizReportStreamMsg::Delta(t))) => t,
            _ => panic!("expected delta, got non-delta or timeout"),
        }
    }

    #[tokio::test]
    async fn rebind_forwards_specialist_deltas_before_router_done() {
        let hub = Arc::new(LiveReportHub::default());
        let (active_tx, active_rx) = watch::channel(None);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let loop_hub = Arc::clone(&hub);
        let join = tokio::spawn(async move {
            run_router_fanin_loop(
                loop_hub,
                "T_router".into(),
                active_rx,
                tx,
                "task".into(),
                "task".into(),
                99010,
            )
            .await;
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        active_tx.send_replace(Some("T_spec".into()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        delta(hub.as_ref(), "T_spec", "hello-");
        delta(hub.as_ref(), "T_spec", "world");
        assert_eq!(next_delta(&mut rx).await, "hello-");
        assert_eq!(next_delta(&mut rx).await, "world");

        solve_done(hub.as_ref(), "T_spec");
        tokio::time::sleep(Duration::from_millis(30)).await;
        match timeout(Duration::from_millis(80), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(_))) => panic!("specialist done must not close SSE"),
            _ => {}
        }

        active_tx.send_replace(None);
        tokio::time::sleep(Duration::from_millis(20)).await;
        solve_done(hub.as_ref(), "T_router");
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(d))) => {
                assert_eq!(d.report_text.as_deref(), Some("hello-world"));
            }
            _ => panic!("expected router done"),
        }
        join.await.expect("loop");
    }

    #[tokio::test]
    async fn mixed_serial_second_specialist_still_streams() {
        let hub = Arc::new(LiveReportHub::default());
        let (active_tx, active_rx) = watch::channel(None);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let loop_hub = Arc::clone(&hub);
        tokio::spawn(async move {
            run_router_fanin_loop(
                loop_hub,
                "T_router2".into(),
                active_rx,
                tx,
                "task".into(),
                "task".into(),
                99010,
            )
            .await;
        });

        active_tx.send_replace(Some("T_kb".into()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        delta(hub.as_ref(), "T_kb", "kb-");
        assert_eq!(next_delta(&mut rx).await, "kb-");
        solve_done(hub.as_ref(), "T_kb");
        active_tx.send_replace(None);
        tokio::time::sleep(Duration::from_millis(20)).await;

        active_tx.send_replace(Some("T_ops".into()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        delta(hub.as_ref(), "T_ops", "ops");
        assert_eq!(next_delta(&mut rx).await, "ops");
        solve_done(hub.as_ref(), "T_ops");
        active_tx.send_replace(None);
        tokio::time::sleep(Duration::from_millis(20)).await;
        solve_done(hub.as_ref(), "T_router2");
        match timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(BizReportStreamMsg::Done(d))) => {
                assert_eq!(d.report_text.as_deref(), Some("kb-ops"));
            }
            _ => panic!("expected router done"),
        }
    }
}
