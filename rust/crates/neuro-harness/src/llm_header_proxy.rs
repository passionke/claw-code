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
    if let Ok(raw) = std::str::from_utf8(&buf[..header_end]) {
        let req = raw.lines().next().unwrap_or("");
        let turn = extra.get("claw-turn-id").map_or("-", String::as_str);
        eprintln!("[llm-header-proxy] {req} claw-turn-id={turn}");
        let mut header_names = Vec::new();
        for line in raw.lines().skip(1) {
            if line.is_empty() {
                break;
            }
            let name = line.split_once(':').map_or(line, |(n, _)| n.trim());
            header_names.push(name.to_ascii_lowercase());
        }
        let injected_preview = std::str::from_utf8(&injected)
            .unwrap_or("")
            .lines()
            .take(20)
            .collect::<Vec<_>>()
            .join("\\n");
        let log_path = std::path::Path::new(".neuro-harness").join("llm-proxy.ndjson");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(
                f,
                "{}",
                serde_json::json!({
                    "req": req,
                    "clawTurnId": turn,
                    "extraKeys": extra.len(),
                    "clientHeaderNames": header_names,
                    "injectedHead": injected_preview,
                })
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
    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some(p + 4);
    }
    // Bun may use LF-only. Author: kejiqing
    buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2)
}

fn inject_trace_headers(
    head: &[u8],
    upstream_host: &str,
    extra: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    // Always emit CRLF upstream — Bun often speaks LF-only; tap needs CRLF to parse
    // claw-turn-id (otherwise billing is skipped while the body still proxies). Author: kejiqing
    const OUT_SEP: &str = "\r\n";
    let text = std::str::from_utf8(head).map_err(|e| format!("headers utf8: {e}"))?;
    // Prefer CRLF (HTTP); fall back to LF-only (some Bun builds). Author: kejiqing
    let (in_sep, end) = if text.contains("\r\n\r\n") {
        ("\r\n", "\r\n\r\n")
    } else if text.contains("\n\n") {
        ("\n", "\n\n")
    } else {
        return Err("no header terminator".into());
    };
    let end_at = text
        .find(end)
        .ok_or_else(|| "header terminator missing after detect".to_string())?;
    let header_block = &text[..end_at];
    let mut out = String::with_capacity(head.len() + 256);
    let mut saw_host = false;
    for (i, line) in header_block.split(in_sep).enumerate() {
        if i == 0 {
            // Normalize openai-compatible path so tap billing matches claw. Author: kejiqing
            let req = if let Some(rest) = line.strip_prefix("POST /chat/completions") {
                format!("POST /v1/chat/completions{rest}")
            } else if let Some(rest) = line.strip_prefix("POST /responses") {
                format!("POST /v1/responses{rest}")
            } else {
                line.to_string()
            };
            out.push_str(&req);
            out.push_str(OUT_SEP);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("host:") {
            out.push_str("Host: ");
            out.push_str(upstream_host);
            out.push_str(OUT_SEP);
            saw_host = true;
            continue;
        }
        // Drop keep-alive / claw / content-encoding negotiation that confuses billing. Author: kejiqing
        if lower.starts_with("claw-turn-id:")
            || lower.starts_with("claw-session-id:")
            || lower.starts_with("clawcode-session-id:")
            || lower.starts_with("connection:")
            || lower.starts_with("keep-alive:")
            || lower.starts_with("expect:")
            || lower.starts_with("accept-encoding:")
        {
            continue;
        }
        out.push_str(line);
        out.push_str(OUT_SEP);
    }
    if !saw_host {
        out.push_str("Host: ");
        out.push_str(upstream_host);
        out.push_str(OUT_SEP);
    }
    out.push_str("Connection: close");
    out.push_str(OUT_SEP);
    for (k, v) in extra {
        out.push_str(k);
        out.push_str(": ");
        out.push_str(v);
        out.push_str(OUT_SEP);
    }
    out.push_str(OUT_SEP);
    Ok(out.into_bytes())
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

    #[test]
    fn inject_accepts_lf_only_headers() {
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("claw-turn-id".into(), "T_lf".into());
        let head = b"POST /chat/completions HTTP/1.1\nHost: 127.0.0.1:9\nContent-Length: 0\n\n";
        assert_eq!(find_header_end(head), Some(head.len()));
        let out = inject_trace_headers(head, "tap.example:8080", &extra).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("claw-turn-id: T_lf"));
        assert!(s.contains("Host: tap.example:8080"));
        assert!(s.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.contains("claw-turn-id: T_lf\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }
}
