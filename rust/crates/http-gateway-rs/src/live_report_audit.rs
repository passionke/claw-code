//! Live report debug timestamps (PG NOTIFY / query / SSE emit / worker proxy). Author: kejiqing

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use tracing::info;

#[cfg(test)]
static FORCE_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub fn force_enabled_for_test() {
    FORCE_ENABLED.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// `CLAW_REPORT_SSE_TIMING=1`, `CLAW_LIVE_SSE_EMIT_TRACE=1`, or `CLAW_SSE_DEBUG=1`. Author: kejiqing
#[must_use]
pub fn enabled() -> bool {
    #[cfg(test)]
    if FORCE_ENABLED.load(std::sync::atomic::Ordering::SeqCst) {
        return true;
    }
    fn on(name: &str) -> bool {
        std::env::var(name).ok().is_some_and(|v| {
            let s = v.trim().to_ascii_lowercase();
            matches!(s.as_str(), "1" | "true" | "yes" | "on")
        })
    }
    on("CLAW_REPORT_SSE_TIMING")
        || on("CLAW_LIVE_SSE_EMIT_TRACE")
        || on("CLAW_SSE_DEBUG")
}

/// Wrap worker SSE byte stream with connect → first-byte → per-chunk timing. Author: kejiqing
pub fn trace_worker_proxy_byte_stream(
    turn_id: &str,
    worker_url: &str,
    stream: impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    use futures_util::StreamExt;

    let trace = enabled();
    let connect_ms = if trace { now_ms() } else { 0 };
    let first_byte = Arc::new(AtomicBool::new(true));
    let turn_log = turn_id.to_string();
    let url_log = worker_url.to_string();
    if trace {
        info!(
            target: "claw_report_sse_timing",
            component = "gateway_worker_proxy",
            phase = "gateway_proxy_connect",
            turn_id = %turn_id,
            worker_url = %worker_url,
            connect_at_ms = connect_ms,
        );
    }
    stream.map(move |r| {
        if trace {
            if let Ok(ref chunk) = r {
                let at = now_ms();
                if first_byte.swap(false, Ordering::SeqCst) {
                    info!(
                        target: "claw_report_sse_timing",
                        component = "gateway_worker_proxy",
                        phase = "gateway_proxy_first_byte",
                        turn_id = %turn_log,
                        connect_to_first_byte_ms = (at - connect_ms).max(0),
                        bytes = chunk.len(),
                    );
                }
                let saw_delta = chunk
                    .windows(b"biz.report.delta".len())
                    .any(|w| w == b"biz.report.delta");
                info!(
                    target: "claw_report_sse_timing",
                    component = "gateway_worker_proxy",
                    phase = "gateway_proxy_chunk",
                    turn_id = %turn_log,
                    worker_url = %url_log,
                    bytes = chunk.len(),
                    saw_biz_report_delta = saw_delta,
                    ms_since_connect = (at - connect_ms).max(0),
                );
            }
        }
        r.map_err(std::io::Error::other)
    })
}

#[cfg(test)]
mod tests {
    use axum::routing::get;
    use axum::Router;
    use futures_util::StreamExt;
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn gateway_proxy_timing_smoke() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::new("claw_report_sse_timing=trace,info"),
            )
            .with_test_writer()
            .try_init();

        force_enabled_for_test();

        let turn_id = "T_proxy_timing00000000000000001";
        let path = format!("/v1/turns/{turn_id}/report");
        let app = Router::new().route(
            &path,
            get(|| async {
                "event: biz.report.start\ndata: {\"taskId\":\"t\"}\n\n\
                 event: biz.report.delta\ndata: {\"text\":\"hello\"}\n\n"
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:38888")
            .await
            .expect("bind mock worker");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("http://127.0.0.1:38888{path}");
        let resp = reqwest::Client::new()
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .expect("GET mock worker SSE");
        let traced = trace_worker_proxy_byte_stream(turn_id, &url, resp.bytes_stream());
        let mut n = 0usize;
        let mut stream = traced;
        while let Some(frame) = stream.next().await {
            if frame.is_ok() {
                n += 1;
                if n >= 2 {
                    break;
                }
            }
        }
        assert!(n >= 1, "traced proxy stream should read mock SSE bytes");
    }
}
