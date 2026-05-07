use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::mcp_client::{McpClientTransport, McpRemoteTransport};
use crate::mcp_stdio::{
    JsonRpcId, JsonRpcRequest, JsonRpcResponse, McpInitializeParams, McpInitializeResult,
    McpListResourcesParams, McpListResourcesResult, McpListToolsParams, McpListToolsResult,
    McpReadResourceParams, McpReadResourceResult, McpToolCallParams, McpToolCallResult,
};
use crate::sse::IncrementalSseParser;

#[cfg(test)]
const MCP_LIST_TOOLS_TIMEOUT_MS: u64 = 300;
#[cfg(not(test))]
const MCP_LIST_TOOLS_TIMEOUT_MS: u64 = 30_000;

#[cfg(test)]
const MCP_REMOTE_SSE_CONNECT_TIMEOUT_MS: u64 = 500;
#[cfg(not(test))]
const MCP_REMOTE_SSE_CONNECT_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug)]
pub(crate) struct McpRemoteProcess {
    transport: McpClientTransport,
    client: reqwest::Client,
    headers: HeaderMap,
    session_id: Option<String>,
    sse_message_url: Option<String>,
    sse_pending: Arc<Mutex<BTreeMap<String, oneshot::Sender<JsonValue>>>>,
    sse_reader_task: Option<JoinHandle<()>>,
}

impl McpRemoteProcess {
    pub(crate) fn new(transport: McpClientTransport) -> io::Result<Self> {
        let headers = default_headers_for_transport(&transport)?;
        let client = reqwest::Client::builder()
            .build()
            .map_err(reqwest_error_to_io)?;
        Ok(Self {
            transport,
            client,
            headers,
            session_id: None,
            sse_message_url: None,
            sse_pending: Arc::new(Mutex::new(BTreeMap::new())),
            sse_reader_task: None,
        })
    }

    pub(crate) async fn initialize(
        &mut self,
        id: JsonRpcId,
        params: McpInitializeParams,
    ) -> io::Result<JsonRpcResponse<McpInitializeResult>> {
        self.request(id, "initialize", Some(params)).await
    }

    pub(crate) async fn list_tools(
        &mut self,
        id: JsonRpcId,
        params: Option<McpListToolsParams>,
    ) -> io::Result<JsonRpcResponse<McpListToolsResult>> {
        self.request(id, "tools/list", params).await
    }

    pub(crate) async fn call_tool(
        &mut self,
        id: JsonRpcId,
        params: McpToolCallParams,
    ) -> io::Result<JsonRpcResponse<McpToolCallResult>> {
        self.request(id, "tools/call", Some(params)).await
    }

    pub(crate) async fn list_resources(
        &mut self,
        id: JsonRpcId,
        params: Option<McpListResourcesParams>,
    ) -> io::Result<JsonRpcResponse<McpListResourcesResult>> {
        self.request(id, "resources/list", params).await
    }

    pub(crate) async fn read_resource(
        &mut self,
        id: JsonRpcId,
        params: McpReadResourceParams,
    ) -> io::Result<JsonRpcResponse<McpReadResourceResult>> {
        self.request(id, "resources/read", Some(params)).await
    }

    async fn request<TParams: Serialize, TResult: DeserializeOwned>(
        &mut self,
        id: JsonRpcId,
        method: impl Into<String>,
        params: Option<TParams>,
    ) -> io::Result<JsonRpcResponse<TResult>> {
        let method = method.into();
        let request = JsonRpcRequest::new(id.clone(), method.clone(), params);
        let mut response = self.send_jsonrpc_request(&request).await?;
        if matches!(self.transport, McpClientTransport::Sse(_))
            && response
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("Could not find session"))
        {
            self.sse_message_url = None;
            response = self.send_jsonrpc_request(&request).await?;
        }
        if response.jsonrpc != "2.0" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "MCP response for {method} used unsupported jsonrpc version `{}`",
                    response.jsonrpc
                ),
            ));
        }
        if response.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "MCP response for {method} used mismatched id: expected {id:?}, got {:?}",
                    response.id
                ),
            ));
        }
        Ok(response)
    }

    async fn send_jsonrpc_request<TParams: Serialize, TResult: DeserializeOwned>(
        &mut self,
        request: &JsonRpcRequest<TParams>,
    ) -> io::Result<JsonRpcResponse<TResult>> {
        let transport = self.transport.clone();
        match transport {
            McpClientTransport::Http(remote) => self.send_http_jsonrpc(&remote, request).await,
            McpClientTransport::Sse(remote) => self.send_sse_jsonrpc(&remote, request).await,
            other => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported remote transport: {other:?}"),
            )),
        }
    }

    async fn send_http_jsonrpc<TParams: Serialize, TResult: DeserializeOwned>(
        &mut self,
        remote: &McpRemoteTransport,
        request: &JsonRpcRequest<TParams>,
    ) -> io::Result<JsonRpcResponse<TResult>> {
        let mut call = self.client.post(&remote.url);
        let mut has_accept = false;
        let mut has_protocol_version = false;
        for (name, value) in &self.headers {
            if name.as_str().eq_ignore_ascii_case("accept") {
                has_accept = true;
            }
            if name.as_str().eq_ignore_ascii_case("mcp-protocol-version") {
                has_protocol_version = true;
            }
            call = call.header(name, value);
        }
        if !has_accept {
            call = call.header("Accept", "application/json, text/event-stream");
        }
        if !has_protocol_version {
            call = call.header("MCP-Protocol-Version", "2025-06-18");
        }
        if let Some(session_id) = &self.session_id {
            call = call.header("Mcp-Session-Id", session_id);
        }
        let response = call
            .json(request)
            .send()
            .await
            .map_err(reqwest_error_to_io)?;
        if let Some(value) = response.headers().get("Mcp-Session-Id") {
            if let Ok(session_id) = value.to_str() {
                self.session_id = Some(session_id.to_string());
            }
        }
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(io::Error::other(format!(
                "MCP remote server returned HTTP {status}: {body}"
            )));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if content_type.contains("text/event-stream") {
            let body = response.text().await.map_err(reqwest_error_to_io)?;
            let mut parser = IncrementalSseParser::new();
            for event in parser.push_chunk(&body) {
                if let Ok(parsed) = serde_json::from_str::<JsonRpcResponse<TResult>>(&event.data) {
                    return Ok(parsed);
                }
            }
            for event in parser.finish() {
                if let Ok(parsed) = serde_json::from_str::<JsonRpcResponse<TResult>>(&event.data) {
                    return Ok(parsed);
                }
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP streamable HTTP returned SSE without JSON-RPC response event",
            ));
        }
        response.json().await.map_err(reqwest_error_to_io)
    }

    async fn send_sse_jsonrpc<TParams: Serialize, TResult: DeserializeOwned>(
        &mut self,
        remote: &McpRemoteTransport,
        request: &JsonRpcRequest<TParams>,
    ) -> io::Result<JsonRpcResponse<TResult>> {
        self.ensure_sse_session(remote).await?;
        let request_url = self
            .sse_message_url
            .clone()
            .ok_or_else(|| io::Error::other("MCP SSE endpoint is not ready"))?;
        let request_id = json_rpc_id_key(&request.id);
        let (tx, rx) = oneshot::channel::<JsonValue>();
        self.sse_pending.lock().await.insert(request_id, tx);

        let payload = serde_json::to_vec(request)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let post_result = self.post_sse_message(&request_url, payload).await;
        if let Err(error) = post_result {
            self.sse_pending
                .lock()
                .await
                .remove(&json_rpc_id_key(&request.id));
            return Err(error);
        }

        let message = timeout(Duration::from_millis(MCP_LIST_TOOLS_TIMEOUT_MS), rx)
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for SSE JSON-RPC response",
                )
            })?
            .map_err(|_| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "SSE response channel closed")
            })?;

        serde_json::from_value(message)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    async fn post_sse_message(&self, request_url: &str, payload: Vec<u8>) -> io::Result<()> {
        let mut with_headers = self
            .client
            .post(request_url)
            .header("content-type", "application/json")
            .body(payload.clone());
        for (name, value) in &self.headers {
            with_headers = with_headers.header(name, value);
        }
        let primary = with_headers.send().await.map_err(reqwest_error_to_io)?;
        if primary.status().is_success() {
            return Ok(());
        }

        let secondary = self
            .client
            .post(request_url)
            .header("content-type", "application/json")
            .body(payload)
            .send()
            .await
            .map_err(reqwest_error_to_io)?;
        if secondary.status().is_success() {
            return Ok(());
        }

        let first_status = primary.status();
        let first_body = primary.text().await.unwrap_or_default();
        let second_status = secondary.status();
        let second_body = secondary.text().await.unwrap_or_default();
        Err(io::Error::other(format!(
            "MCP SSE POST failed: first HTTP {first_status}: {first_body}; second HTTP {second_status}: {second_body}"
        )))
    }

    async fn ensure_sse_session(&mut self, remote: &McpRemoteTransport) -> io::Result<()> {
        if self.sse_message_url.is_some() && self.sse_reader_task.is_some() {
            return Ok(());
        }

        self.shutdown();
        let mut connect = self
            .client
            .get(&remote.url)
            .header("Accept", "text/event-stream");
        for (name, value) in &self.headers {
            connect = connect.header(name, value);
        }
        let response = connect.send().await.map_err(reqwest_error_to_io)?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(io::Error::other(format!(
                "MCP SSE handshake failed with HTTP {status}: {body}"
            )));
        }

        let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();
        let pending = Arc::clone(&self.sse_pending);
        let base_url = remote.url.clone();
        let task = tokio::spawn(async move {
            let mut parser = IncrementalSseParser::new();
            let mut endpoint_tx = Some(endpoint_tx);
            let mut stream = response;
            #[allow(clippy::match_same_arms)]
            loop {
                match stream.chunk().await {
                    Ok(Some(chunk)) => {
                        let text = String::from_utf8_lossy(&chunk);
                        for event in parser.push_chunk(&text) {
                            if event.event.as_deref() == Some("endpoint") && endpoint_tx.is_some() {
                                if let Ok(Some(endpoint)) =
                                    extract_sse_message_url(&base_url, &event.data)
                                {
                                    if let Some(tx) = endpoint_tx.take() {
                                        let _ = tx.send(endpoint);
                                    }
                                }
                                continue;
                            }
                            if let Ok(message) = serde_json::from_str::<JsonValue>(&event.data) {
                                if let Some(id) = message.get("id") {
                                    let key = json_value_id_key(id);
                                    if let Some(tx) = pending.lock().await.remove(&key) {
                                        let _ = tx.send(message);
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });
        self.sse_reader_task = Some(task);
        let endpoint = timeout(
            Duration::from_millis(MCP_REMOTE_SSE_CONNECT_TIMEOUT_MS),
            endpoint_rx,
        )
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("MCP SSE handshake timed out after {MCP_REMOTE_SSE_CONNECT_TIMEOUT_MS}ms"),
            )
        })?
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SSE stream closed before endpoint event",
            )
        })?;
        self.sse_message_url = Some(endpoint);
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        self.session_id = None;
        self.sse_message_url = None;
        if let Some(task) = self.sse_reader_task.take() {
            task.abort();
        }
    }
}

fn default_headers_for_transport(transport: &McpClientTransport) -> io::Result<HeaderMap> {
    let headers = match transport {
        McpClientTransport::Http(remote) | McpClientTransport::Sse(remote) => &remote.headers,
        _ => return Ok(HeaderMap::new()),
    };
    let mut map = HeaderMap::new();
    for (key, value) in headers {
        let name = HeaderName::from_bytes(key.as_bytes()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid MCP header name `{key}`: {error}"),
            )
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid MCP header value for `{key}`: {error}"),
            )
        })?;
        map.insert(name, header_value);
    }
    Ok(map)
}

pub(crate) fn extract_sse_message_url(
    base_url: &str,
    event_data: &str,
) -> io::Result<Option<String>> {
    let data = event_data.trim();
    if data.is_empty() {
        return Ok(None);
    }
    if let Ok(value) = serde_json::from_str::<JsonValue>(data) {
        if let Some(endpoint) = value.get("endpoint").and_then(JsonValue::as_str) {
            return resolve_relative_url(base_url, endpoint).map(Some);
        }
    }
    resolve_relative_url(base_url, data).map(Some)
}

fn resolve_relative_url(base: &str, maybe_relative: &str) -> io::Result<String> {
    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return Ok(maybe_relative.to_string());
    }
    let base_url = Url::parse(base).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid MCP base URL `{base}`: {error}"),
        )
    })?;
    base_url
        .join(maybe_relative)
        .map(|url| url.to_string())
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid MCP endpoint URL `{maybe_relative}`: {error}"),
            )
        })
}

fn reqwest_error_to_io(error: reqwest::Error) -> io::Error {
    if error.is_timeout() {
        io::Error::new(io::ErrorKind::TimedOut, error)
    } else {
        io::Error::other(error)
    }
}

fn json_rpc_id_key(id: &JsonRpcId) -> String {
    match id {
        JsonRpcId::Number(value) => value.to_string(),
        JsonRpcId::String(value) => value.clone(),
        JsonRpcId::Null => "null".to_string(),
    }
}

fn json_value_id_key(value: &JsonValue) -> String {
    if let Some(number) = value.as_i64() {
        return number.to_string();
    }
    if let Some(number) = value.as_u64() {
        return number.to_string();
    }
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    "null".to_string()
}
