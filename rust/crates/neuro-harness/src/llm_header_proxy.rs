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
    /// [`gateway_solve_turn::gateway_llm_session_extra_headers`] on each request, and merge
    /// [`gateway_solve_turn::chat_thinking_fields`] into JSON chat/responses bodies.
    pub async fn start(
        upstream_base: &str,
        model: &str,
        thinking_enabled: bool,
    ) -> Result<Self, HarnessError> {
        let upstream = Arc::new(ProxyTarget {
            upstream: parse_upstream(upstream_base)?,
            model: model.to_string(),
            thinking_enabled,
        });
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .map_err(|e| HarnessError::internal(format!("llm header proxy bind: {e}")))?;
        let local = listener
            .local_addr()
            .map_err(|e| HarnessError::internal(format!("llm header proxy addr: {e}")))?;
        let local_base_url = format!("http://127.0.0.1:{}", local.port());
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
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

struct ProxyTarget {
    upstream: Upstream,
    model: String,
    thinking_enabled: bool,
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

async fn proxy_one(mut client: TcpStream, target: &ProxyTarget) -> Result<(), String> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let header_end = read_http_headers(&mut client, &mut buf).await?;
    let head = buf[..header_end].to_vec();
    let mut body = buf[header_end..].to_vec();
    let content_length = content_length_of(&head);
    if let Some(len) = content_length {
        while body.len() < len {
            let mut tmp = [0_u8; 8192];
            let n = client
                .read(&mut tmp)
                .await
                .map_err(|e| format!("read client body: {e}"))?;
            if n == 0 {
                return Err("client closed before body complete".into());
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(len);
    }
    let rewritten = rewrite_llm_body(&head, &body, &target.model, target.thinking_enabled)?;
    let (body, length_override) = match rewritten {
        Some(next) => {
            let len = next.len();
            (next, Some(len))
        }
        None => (body, None),
    };
    let extra = gateway_solve_turn::gateway_llm_session_extra_headers();
    let injected =
        inject_trace_headers(&head, &target.upstream.host_header, &extra, length_override)?;
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
    let mut server = TcpStream::connect(&target.upstream.addr)
        .await
        .map_err(|e| format!("connect {}: {e}", target.upstream.addr))?;
    server
        .write_all(&injected)
        .await
        .map_err(|e| format!("write upstream headers: {e}"))?;
    if !body.is_empty() {
        server
            .write_all(&body)
            .await
            .map_err(|e| format!("write upstream body: {e}"))?;
    }
    // Responses upstream still emits reasoning_text after effort=none. The project
    // switch off drops those events before the engine can surface them. Author: kejiqing
    if !target.thinking_enabled && is_responses_post(&head) {
        forward_response_without_reasoning(&mut server, &mut client).await?;
    } else {
        tokio::io::copy_bidirectional(&mut client, &mut server)
            .await
            .map_err(|e| format!("proxy copy: {e}"))?;
    }
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

fn content_length_of(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn is_llm_post(head: &[u8]) -> bool {
    request_path(head)
        .is_some_and(|path| path.ends_with("/chat/completions") || path.ends_with("/responses"))
}

fn is_responses_post(head: &[u8]) -> bool {
    request_path(head).is_some_and(|path| path.ends_with("/responses"))
}

fn request_path(head: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(head).ok()?;
    let line = text.lines().next()?;
    if !line.starts_with("POST ") {
        return None;
    }
    let path = line.split_whitespace().nth(1)?;
    Some(path.split('?').next().unwrap_or(path))
}

fn is_deepseek_model(model: &str) -> bool {
    let lowered = model.to_ascii_lowercase();
    let canonical = lowered.rsplit('/').next().unwrap_or(lowered.as_str());
    canonical.starts_with("deepseek")
}

/// Drop Responses reasoning events. Incomplete trailing bytes stay in `pending`.
/// Author: kejiqing
struct ReasoningSseFilter {
    pending: Vec<u8>,
}

impl ReasoningSseFilter {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some(end) = sse_event_end(&self.pending) else {
                break;
            };
            let event: Vec<u8> = self.pending.drain(..end).collect();
            if let Some(kept) = rewrite_sse_event(&event) {
                out.extend_from_slice(&kept);
            }
        }
        out
    }

    fn finish(&mut self) -> Vec<u8> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let event: Vec<u8> = self.pending.drain(..).collect();
        rewrite_sse_event(&event).unwrap_or_default()
    }
}

fn sse_event_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .or_else(|| buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2))
}

fn rewrite_sse_event(event: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(event).ok()?;
    let sep = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut others = Vec::new();
    let mut data_parts = Vec::new();
    for line in text.split(sep) {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_parts.push(rest.trim_start());
        } else {
            others.push(line);
        }
    }
    if data_parts.is_empty() {
        return Some(event.to_vec());
    }
    let data = data_parts.join("\n");
    if data == "[DONE]" {
        return Some(event.to_vec());
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&data) else {
        return Some(event.to_vec());
    };
    if sse_event_is_reasoning(&value) {
        return None;
    }
    scrub_reasoning_output(&mut value);
    let rendered = serde_json::to_string(&value).ok()?;
    let mut out = String::new();
    for line in others {
        out.push_str(line);
        out.push_str(sep);
    }
    out.push_str("data: ");
    out.push_str(&rendered);
    out.push_str(sep);
    out.push_str(sep);
    Some(out.into_bytes())
}

fn sse_event_is_reasoning(value: &serde_json::Value) -> bool {
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if kind.starts_with("response.reasoning_text.")
        || kind.starts_with("response.reasoning_summary")
    {
        return true;
    }
    if kind == "response.output_item.added" || kind == "response.output_item.done" {
        return value
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(|v| v.as_str())
            == Some("reasoning");
    }
    if kind == "response.content_part.added" || kind == "response.content_part.done" {
        return value
            .get("part")
            .and_then(|part| part.get("type"))
            .and_then(|v| v.as_str())
            == Some("reasoning_text");
    }
    false
}

fn scrub_reasoning_output(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(response) = obj.get_mut("response") {
        retain_non_reasoning_output(response);
    }
    retain_non_reasoning_output(value);
}

fn retain_non_reasoning_output(value: &mut serde_json::Value) {
    let Some(output) = value.get_mut("output").and_then(|v| v.as_array_mut()) else {
        return;
    };
    output.retain(|item| item.get("type").and_then(|v| v.as_str()) != Some("reasoning"));
}

async fn forward_response_without_reasoning(
    server: &mut TcpStream,
    client: &mut TcpStream,
) -> Result<(), String> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let header_end = read_http_headers(server, &mut buf).await?;
    let head = std::str::from_utf8(&buf[..header_end])
        .map_err(|e| format!("response headers utf8: {e}"))?
        .to_string();
    let mut body = buf[header_end..].to_vec();
    let chunked = header_is_chunked(&head);
    let content_length = content_length_of(head.as_bytes());
    if chunked {
        client
            .write_all(chunked_response_head(&head).as_bytes())
            .await
            .map_err(|e| format!("write filtered response head: {e}"))?;
        let mut decoder = ChunkDecoder::default();
        let mut filter = ReasoningSseFilter::new();
        decoder.push(&body)?;
        write_filtered_chunks(client, &filter.push(&decoder.take_data())).await?;
        let mut tmp = [0_u8; 8192];
        while !decoder.done {
            let n = server
                .read(&mut tmp)
                .await
                .map_err(|e| format!("read upstream response: {e}"))?;
            if n == 0 {
                return Err("upstream closed before chunked body ended".into());
            }
            decoder.push(&tmp[..n])?;
            write_filtered_chunks(client, &filter.push(&decoder.take_data())).await?;
        }
        write_filtered_chunks(client, &filter.finish()).await?;
        client
            .write_all(b"0\r\n\r\n")
            .await
            .map_err(|e| format!("write filtered terminator: {e}"))?;
        return Ok(());
    }
    if let Some(len) = content_length {
        while body.len() < len {
            let mut tmp = [0_u8; 8192];
            let n = server
                .read(&mut tmp)
                .await
                .map_err(|e| format!("read upstream response: {e}"))?;
            if n == 0 {
                return Err("upstream closed before response body ended".into());
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(len);
    } else {
        let mut tmp = [0_u8; 8192];
        loop {
            let n = server
                .read(&mut tmp)
                .await
                .map_err(|e| format!("read upstream response: {e}"))?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
    }
    let mut filter = ReasoningSseFilter::new();
    let mut filtered = filter.push(&body);
    filtered.extend(filter.finish());
    let head = content_length_response_head(&head, filtered.len());
    client
        .write_all(head.as_bytes())
        .await
        .map_err(|e| format!("write filtered response head: {e}"))?;
    client
        .write_all(&filtered)
        .await
        .map_err(|e| format!("write filtered response body: {e}"))?;
    Ok(())
}

fn header_is_chunked(head: &str) -> bool {
    header_value(head, "transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
}

fn header_value(head: &str, name: &str) -> Option<String> {
    for line in head.lines().skip(1) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn content_length_response_head(head: &str, len: usize) -> String {
    rewrite_response_head(head, &format!("Content-Length: {len}"))
}

fn chunked_response_head(head: &str) -> String {
    rewrite_response_head(head, "Transfer-Encoding: chunked")
}

fn rewrite_response_head(head: &str, replacement: &str) -> String {
    let sep = if head.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::new();
    for (i, line) in head.split(sep).enumerate() {
        if line.is_empty() {
            continue;
        }
        if i > 0 {
            let name = line.split_once(':').map_or(line, |(name, _)| name);
            if name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding")
            {
                continue;
            }
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(replacement);
    out.push_str("\r\n\r\n");
    out
}

async fn write_filtered_chunks(client: &mut TcpStream, data: &[u8]) -> Result<(), String> {
    if data.is_empty() {
        return Ok(());
    }
    let header = format!("{:X}\r\n", data.len());
    client
        .write_all(header.as_bytes())
        .await
        .map_err(|e| format!("write filtered chunk: {e}"))?;
    client
        .write_all(data)
        .await
        .map_err(|e| format!("write filtered chunk: {e}"))?;
    client
        .write_all(b"\r\n")
        .await
        .map_err(|e| format!("write filtered chunk: {e}"))?;
    Ok(())
}

#[derive(Default)]
struct ChunkDecoder {
    buf: Vec<u8>,
    data: Vec<u8>,
    done: bool,
}

impl ChunkDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.buf.extend_from_slice(bytes);
        while !self.done {
            let Some(line_end) = self.buf.windows(2).position(|w| w == b"\r\n") else {
                break;
            };
            let line = std::str::from_utf8(&self.buf[..line_end])
                .map_err(|e| format!("chunk size utf8: {e}"))?;
            let size_hex = line.split(';').next().unwrap_or("").trim();
            let size =
                usize::from_str_radix(size_hex, 16).map_err(|e| format!("chunk size: {e}"))?;
            let start = line_end + 2;
            if size == 0 {
                self.done = true;
                break;
            }
            if self.buf.len() < start + size + 2 {
                break;
            }
            self.data.extend_from_slice(&self.buf[start..start + size]);
            self.buf.drain(..start + size + 2);
        }
        Ok(())
    }

    fn take_data(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.data)
    }
}

/// Merge the project thinking switch into a chat/responses JSON body.
/// `None` means this request is not an LLM post and must be forwarded as-is.
/// Author: kejiqing
fn rewrite_llm_body(
    head: &[u8],
    body: &[u8],
    model: &str,
    thinking_enabled: bool,
) -> Result<Option<Vec<u8>>, String> {
    if !is_llm_post(head) {
        return Ok(None);
    }
    if content_length_of(head).is_none() {
        return Err("llm request has no Content-Length; cannot apply thinkingEnabled".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("llm body is not json: {e}"))?;
    let Some(obj) = value.as_object_mut() else {
        return Err("llm body is not a json object; cannot apply thinkingEnabled".into());
    };
    let wire_model = obj
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(model)
        .to_string();
    for (key, field) in
        gateway_solve_turn::chat_thinking_fields(&wire_model, Some(thinking_enabled))
    {
        obj.insert(key, field);
    }
    // Codex catalog asks for `reasoning.summary=auto`. DeepSeek then returns
    // reasoning text even when `thinking.type` is disabled. The project switch
    // off drops that summary; on leaves the body Codex already sent. Author: kejiqing
    if !thinking_enabled && is_responses_post(head) && is_deepseek_model(&wire_model) {
        let reasoning = obj
            .entry("reasoning")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(reasoning) = reasoning.as_object_mut() {
            reasoning.insert("effort".to_string(), serde_json::json!("none"));
            reasoning.remove("summary");
        }
        if let Some(include) = obj.get_mut("include").and_then(|v| v.as_array_mut()) {
            include.retain(|v| v.as_str() != Some("reasoning.encrypted_content"));
        }
        if obj
            .get("include")
            .and_then(|v| v.as_array())
            .is_some_and(|items| items.is_empty())
        {
            obj.remove("include");
        }
    } else if !thinking_enabled {
        if let Some(reasoning) = obj.get_mut("reasoning").and_then(|v| v.as_object_mut()) {
            reasoning.remove("summary");
        }
    }
    serde_json::to_vec(&value)
        .map(Some)
        .map_err(|e| format!("rewrite llm body: {e}"))
}

fn inject_trace_headers(
    head: &[u8],
    upstream_host: &str,
    extra: &std::collections::BTreeMap<String, String>,
    content_length: Option<usize>,
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
            || (content_length.is_some() && lower.starts_with("content-length:"))
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
    if let Some(len) = content_length {
        out.push_str("Content-Length: ");
        out.push_str(&len.to_string());
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
        let out = inject_trace_headers(head, "tap.example:8080", &extra, None).unwrap();
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
        let out = inject_trace_headers(head, "tap.example:8080", &extra, None).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("claw-turn-id: T_lf"));
        assert!(s.contains("Host: tap.example:8080"));
        assert!(s.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.contains("claw-turn-id: T_lf\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }

    #[test]
    fn rewrite_puts_qwen_disable_on_the_chat_body() {
        let body = br#"{"model":"qwen3.7-max","messages":[]}"#;
        let head = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let out = rewrite_llm_body(head.as_bytes(), body, "qwen3.7-max", false)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["enable_thinking"], false);
    }

    #[test]
    fn rewrite_drops_reasoning_summary_when_thinking_is_off() {
        let body = br#"{"model":"deepseek-flash","reasoning":{"effort":"none","summary":"auto"}}"#;
        let head = format!(
            "POST /v1/responses HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let out = rewrite_llm_body(head.as_bytes(), body, "deepseek-flash", false)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["thinking"]["type"], "disabled");
        assert_eq!(v["reasoning"]["effort"], "none");
        assert!(v["reasoning"].get("summary").is_none());
    }

    #[test]
    fn rewrite_keeps_reasoning_summary_when_thinking_is_on() {
        let body = br#"{"model":"deepseek-flash","reasoning":{"effort":"none","summary":"auto"}}"#;
        let head = format!(
            "POST /v1/responses HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let out = rewrite_llm_body(head.as_bytes(), body, "deepseek-flash", true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["thinking"]["type"], "enabled");
        assert_eq!(v["reasoning"]["summary"], "auto");
    }

    #[test]
    fn rewrite_forces_responses_effort_none_and_drops_reasoning_include() {
        let body = br#"{"model":"deepseek-flash","include":["reasoning.encrypted_content"],"reasoning":{"effort":"high","summary":"auto"}}"#;
        let head = format!(
            "POST /v1/responses HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let out = rewrite_llm_body(head.as_bytes(), body, "deepseek-flash", false)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["thinking"]["type"], "disabled");
        assert_eq!(v["reasoning"]["effort"], "none");
        assert!(v.get("include").is_none());
        assert!(v["reasoning"].get("summary").is_none());
    }

    #[test]
    fn filter_drops_reasoning_events_and_keeps_the_answer() {
        let raw = "\
data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"SECRET\"}\n\n\
data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"r1\"}}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"好\"}\n\n\
data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"reasoning\",\"content\":[{\"type\":\"reasoning_text\",\"text\":\"SECRET\"}]},{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"好\"}]}]}}\n\n";
        let mut filter = ReasoningSseFilter::new();
        let mut out = filter.push(raw.as_bytes());
        out.extend(filter.finish());
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("SECRET"));
        assert!(text.contains("好"));
        assert!(text.contains("response.output_text.delta"));
        assert!(text.contains("\"type\":\"message\""));
    }

    #[tokio::test]
    async fn proxy_strips_chunked_reasoning_when_thinking_is_off() {
        let sse = "\
data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"SECRET\"}\n\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"好\"}\n\n";
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let sse_bytes = sse.as_bytes().to_vec();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 8192];
            let _ = sock.read(&mut buf).await;
            let chunk = format!("{:X}\r\n", sse_bytes.len());
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(chunk.as_bytes()).await.unwrap();
            sock.write_all(&sse_bytes).await.unwrap();
            sock.write_all(b"\r\n0\r\n\r\n").await.unwrap();
        });
        let proxy = LlmHeaderProxy::start(&format!("http://{addr}"), "deepseek-flash", false)
            .await
            .unwrap();
        let mut client = TcpStream::connect(proxy.local_base_url.trim_start_matches("http://"))
            .await
            .unwrap();
        let req_body = b"{\"model\":\"deepseek-flash\",\"input\":\"ga\"}";
        let req = format!(
            "POST /v1/responses HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            req_body.len()
        );
        client.write_all(req.as_bytes()).await.unwrap();
        client.write_all(req_body).await.unwrap();
        let mut got = Vec::new();
        client.read_to_end(&mut got).await.unwrap();
        proxy.stop().await;
        let text = String::from_utf8_lossy(&got);
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("好"), "{text}");
    }

    #[test]
    fn rewrite_leaves_non_llm_posts_alone() {
        let head = b"GET /v1/models HTTP/1.1\r\nContent-Length: 2\r\n\r\n";
        assert!(rewrite_llm_body(head, b"{}", "qwen3.7-max", false)
            .unwrap()
            .is_none());
    }
}
