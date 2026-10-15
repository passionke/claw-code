//! Localhost reverse proxy that injects claw session/turn headers onto LLM
//! requests. Opencode's `options.headers` are written into config but were
//! observed not to appear on the wire (tap got no `claw-turn-id` → no billing).
//! Author: kejiqing

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::HarnessError;

/// Running proxy: local OpenAI-compatible base URL + background accept loop.
pub struct LlmHeaderProxy {
    pub local_base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl LlmHeaderProxy {
    /// Bind `127.0.0.1:0`, forward to `upstream_base` (`http://host[:port]`), inject
    /// [`gateway_solve_turn::gateway_llm_session_extra_headers`] on each request.
    pub async fn start(upstream_base: &str) -> Result<Self, HarnessError> {
        let upstream = parse_upstream(upstream_base)?;
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .map_err(|e| HarnessError::internal(format!("llm header proxy bind: {e}")))?;
        let local = listener
            .local_addr()
            .map_err(|e| HarnessError::internal(format!("llm header proxy addr: {e}")))?;
        let local_base_url = format!("http://127.0.0.1:{}", local.port());
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let upstream = Arc::new(upstream);
        let join = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    acc = listener.accept() => {
                        let Ok((client, _)) = acc else { continue };
                        let upstream = Arc::clone(&upstream);
                        tokio::spawn(async move {
                            if let Err(e) = proxy_one(client, &upstream).await {
                                eprintln!("[llm-header-proxy] {e}");
                            }
                        });
                    }
                }
            }
        });
        Ok(Self {
            local_base_url,
            shutdown_tx: Some(shutdown_tx),
            join: Some(join),
        })
    }

    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), join).await;
        }
    }
}

struct Upstream {
    host_header: String,
    addr: String,
}

fn parse_upstream(raw: &str) -> Result<Upstream, HarnessError> {
    let trimmed = raw.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .ok_or_else(|| {
            HarnessError::internal(format!(
                "llm header proxy: OPENAI_BASE_URL must be http(s)://…, got {raw:?}"
            ))
        })?;
    let hostport = rest
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HarnessError::internal("llm header proxy: empty host in OPENAI_BASE_URL"))?;
    let addr = if hostport.contains(':') {
        hostport.to_string()
    } else {
        format!("{hostport}:80")
    };
    Ok(Upstream {
        host_header: hostport.to_string(),
        addr,
    })
}

async fn proxy_one(mut client: TcpStream, upstream: &Upstream) -> Result<(), String> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let header_end = read_http_headers(&mut client, &mut buf).await?;
    let extra = gateway_solve_turn::gateway_llm_session_extra_headers();
    let injected = inject_trace_headers(&buf[..header_end], &upstream.host_header, &extra)?;
    // Evidence for e2b billing misses (stderr may be empty on docker exec). Author: kejiqing
    if let Ok(line) = std::str::from_utf8(&buf[..header_end.min(buf.len())]) {
        let req = line.lines().next().unwrap_or("");
        let turn = extra.get("claw-turn-id").map_or("-", String::as_str);
        eprintln!("[llm-header-proxy] {req} claw-turn-id={turn}");
        let log_path = std::path::Path::new(".neuro-harness").join("llm-proxy.ndjson");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(
                f,
                r#"{{"req":{},"clawTurnId":{},"extraKeys":{}}}"#,
                serde_json::to_string(req).unwrap_or_else(|_| "\"\"".into()),
                serde_json::to_string(turn).unwrap_or_else(|_| "\"\"".into()),
                extra.len()
            );
        }
    }
    let mut server = TcpStream::connect(&upstream.addr)
        .await
        .map_err(|e| format!("connect {}: {e}", upstream.addr))?;
    server
        .write_all(&injected)
        .await
        .map_err(|e| format!("write upstream headers: {e}"))?;
    if buf.len() > header_end {
        server
            .write_all(&buf[header_end..])
            .await
            .map_err(|e| format!("write upstream body prefix: {e}"))?;
    }
    // Full-duplex: streaming SSE must not cancel the other direction early. Author: kejiqing
    tokio::io::copy_bidirectional(&mut client, &mut server)
        .await
        .map_err(|e| format!("proxy copy: {e}"))?;
    Ok(())
}

async fn read_http_headers(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<usize, String> {
    let mut tmp = [0_u8; 2048];
    loop {
        if let Some(pos) = find_header_end(buf) {
            return Ok(pos);
        }
        if buf.len() > 64 * 1024 {
            return Err("request headers too large".into());
        }
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read client: {e}"))?;
        if n == 0 {
            return Err("client closed before headers complete".into());
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn inject_trace_headers(
    head: &[u8],
    upstream_host: &str,
    extra: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(head).map_err(|e| format!("headers utf8: {e}"))?;
    let mut lines: Vec<&str> = text.split("\r\n").collect();
    if lines.is_empty() {
        return Err("empty request".into());
    }
    // Drop trailing empty from final \r\n\r\n split.
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let request_line = lines.first().copied().unwrap_or("");
    let mut out = Vec::with_capacity(head.len() + 256);
    out.extend_from_slice(request_line.as_bytes());
    out.extend_from_slice(b"\r\n");
    let mut saw_host = false;
    for line in lines.iter().skip(1) {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("host:") {
            out.extend_from_slice(b"Host: ");
            out.extend_from_slice(upstream_host.as_bytes());
            out.extend_from_slice(b"\r\n");
            saw_host = true;
            continue;
        }
        // Drop any client-supplied claw headers; we re-inject from env. Author: kejiqing
        if lower.starts_with("claw-turn-id:")
            || lower.starts_with("claw-session-id:")
            || lower.starts_with("clawcode-session-id:")
        {
            continue;
        }
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if !saw_host {
        out.extend_from_slice(b"Host: ");
        out.extend_from_slice(upstream_host.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    for (k, v) in extra {
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_http_host_port() {
        let u = parse_upstream("http://tap.example:8080/v1").unwrap();
        assert_eq!(u.host_header, "tap.example:8080");
        assert_eq!(u.addr, "tap.example:8080");
    }

    #[test]
    fn inject_rewrites_host_and_adds_claw_headers() {
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("claw-session-id".into(), "sess-proxy".into());
        extra.insert("claw-turn-id".into(), "T_proxy".into());
        let head =
            b"POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:9\r\nContent-Length: 0\r\n\r\n";
        let out = inject_trace_headers(head, "tap.example:8080", &extra).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("Host: tap.example:8080"));
        assert!(s.contains("claw-turn-id: T_proxy"));
        assert!(s.contains("claw-session-id: sess-proxy"));
        assert!(!s.contains("Host: 127.0.0.1:9"));
    }
}
