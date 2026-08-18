//! Ordered stdout fan-out for solve live reports + delegate registry. Author: kejiqing

use std::sync::Arc;

use gateway_solve_turn::gateway_stdout::parse_stdout_line;
use serde_json::Value;
use tokio::sync::mpsc;

use super::delegate_active_ingest::{is_delegate_registry_event, spawn_delegate_active_consumer};
use crate::session_db::GatewaySessionDb;

/// One mpsc channel + one consumer per turn keeps SSE token order stable.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn merge_stdout_hooks(
    turn_id: &str,
    hub: Option<Arc<super::live_report_hub::LiveReportHub>>,
    session_db: Option<Arc<GatewaySessionDb>>,
    outer: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Option<Arc<dyn Fn(String) + Send + Sync>> {
    let tid = turn_id.to_string();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let delegate_tx: Option<mpsc::UnboundedSender<Value>> = session_db.as_ref().map(|db| {
        let (dtx, drx) = mpsc::unbounded_channel::<Value>();
        spawn_delegate_active_consumer(tid.clone(), Arc::clone(db), drx);
        dtx
    });
    let tid_for_worker = tid.clone();
    let hub_for_worker = hub.clone();
    let outer_for_worker = outer.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if let Some(ref o) = outer_for_worker {
                o(line.clone());
            }
            if let Some(ref h) = hub_for_worker {
                if let Some(value) = parse_stdout_line(&line) {
                    if is_delegate_registry_event(&value) {
                        if let Some(ref dtx) = delegate_tx {
                            let _ = dtx.send(value);
                        }
                        continue;
                    }
                }
                h.ingest_stdout_line(&tid_for_worker, &line);
            }
        }
    });
    let hook: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line: String| {
        let _ = tx.send(line);
    });
    Some(hook)
}
