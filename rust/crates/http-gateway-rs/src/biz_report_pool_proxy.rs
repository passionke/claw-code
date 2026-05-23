//! Gateway → pool HTTP reverse proxy for live report SSE. Author: kejiqing

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use reqwest::Client;
use tracing::info;

pub async fn proxy_pool_live_report_sse(
    pool_http_base: &str,
    turn_id: &str,
    task_id: &str,
    ds_id: i64,
) -> Result<Response, (StatusCode, String)> {
    let base = pool_http_base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "CLAW_POOL_HTTP_BASE is not configured".into(),
        ));
    }
    let url = format!(
        "{base}/v1/biz_advice_report/live?turnId={turn_id}&taskId={task_id}&dsId={ds_id}&stream=true"
    );
    info!(
        target: "claw_live_report",
        component = "biz_report_pool_proxy",
        phase = "upstream_request",
        turn_id = %turn_id,
        task_id = %task_id,
        ds_id,
        pool_http_base = %base,
        upstream_url = %url,
        "pool live SSE proxy — dialing pool HTTP"
    );
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let upstream = client
        .get(&url)
        .header(header::ACCEPT, "text/event-stream")
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("pool sse request failed: {e}"),
            )
        })?;
    let status = upstream.status();
    if !status.is_success() {
        let body = upstream.text().await.unwrap_or_default();
        return Ok((status, body).into_response());
    }
    let headers = upstream.headers().clone();
    let byte_stream = upstream.bytes_stream().map(|r| {
        r.map(|b| b)
            .map_err(|e| std::io::Error::other(e.to_string()))
    });
    let mut resp = Response::new(Body::from_stream(byte_stream));
    *resp.status_mut() = status;
    copy_sse_headers(&headers, resp.headers_mut());
    Ok(resp)
}

fn copy_sse_headers(from: &HeaderMap, to: &mut HeaderMap) {
    if let Some(v) = from.get(header::CONTENT_TYPE) {
        to.insert(header::CONTENT_TYPE, v.clone());
    } else {
        to.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
    }
    if let Some(v) = from.get(header::CACHE_CONTROL) {
        to.insert(header::CACHE_CONTROL, v.clone());
    }
    to.insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
}
