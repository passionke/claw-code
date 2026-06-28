//! Ordered stdout fan-out for solve live reports. Author: kejiqing

use std::sync::Arc;

/// One mpsc channel + one consumer per turn keeps SSE token order stable.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn merge_stdout_hooks(
    turn_id: &str,
    hub: Option<Arc<super::live_report_hub::LiveReportHub>>,
    outer: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Option<Arc<dyn Fn(String) + Send + Sync>> {
    let tid = turn_id.to_string();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let tid_for_worker = tid.clone();
    let hub_for_worker = hub.clone();
    let outer_for_worker = outer.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if let Some(ref o) = outer_for_worker {
                o(line.clone());
            }
            if let Some(ref h) = hub_for_worker {
                h.ingest_stdout_line(&tid_for_worker, &line);
            }
        }
    });
    let hook: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line: String| {
        let _ = tx.send(line);
    });
    Some(hook)
}
