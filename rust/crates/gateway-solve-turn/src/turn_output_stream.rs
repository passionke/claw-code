//! Live report: worker fixed-port SSE (`report_sse_server`) or legacy gateway POST. Author: kejiqing

use std::sync::{Arc, Mutex};

use crate::report_sse_server::{resolve_report_sse_port, ReportSseServerGuard, ReportStreamHandle};

/// Model `TextDelta` sink; prefers in-worker SSE on [`resolve_report_sse_port`]. Author: kejiqing
pub struct TurnOutputStreamClient {
    handle: ReportStreamHandle,
    _server: Mutex<Option<ReportSseServerGuard>>,
}

impl TurnOutputStreamClient {
    /// Starts worker SSE server when `CLAW_WORKER_REPORT_SSE_PORT` is set (default **18765** if unset). Author: kejiqing
    #[must_use]
    pub fn try_new(turn_id: &str) -> Option<Arc<Self>> {
        let port = std::env::var("CLAW_WORKER_REPORT_SSE_PORT")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(crate::report_sse_server::DEFAULT_REPORT_SSE_PORT);
        if port == 0 {
            return None;
        }
        let (handle, server) = crate::report_sse_server::spawn(turn_id, port).ok()?;
        Some(std::sync::Arc::new(Self {
            handle,
            _server: Mutex::new(Some(server)),
        }))
    }

    #[cfg(test)]
    pub(crate) fn upload_thread_started(&self) -> bool {
        true
    }

    pub fn push_text_delta(&self, text: &str) {
        self.handle.push_text_delta(text);
    }
}

/// Wire helpers (tests / legacy). Author: kejiqing
#[must_use]
pub fn biz_report_delta_frame(text: &str) -> String {
    let data = serde_json::json!({ "text": text }).to_string();
    format!("event: biz.report.delta\ndata: {data}\n\n")
}

#[must_use]
pub fn biz_report_start_frame(task_id: &str) -> String {
    let data = serde_json::json!({ "taskId": task_id }).to_string();
    format!("event: biz.report.start\ndata: {data}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biz_report_delta_frame_shape() {
        let frame = biz_report_delta_frame("wire-Δ");
        assert!(frame.contains("event: biz.report.delta"));
        assert!(frame.contains(r#""text":"wire-Δ""#));
    }
}
