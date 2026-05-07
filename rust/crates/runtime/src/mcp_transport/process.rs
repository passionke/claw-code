use std::io;

use crate::mcp_client::{McpClientBootstrap, McpClientTransport};
use crate::mcp_stdio::{
    JsonRpcId, JsonRpcResponse, McpInitializeParams, McpInitializeResult, McpListResourcesParams,
    McpListResourcesResult, McpListToolsParams, McpListToolsResult, McpReadResourceParams,
    McpReadResourceResult, McpToolCallParams, McpToolCallResult,
};
use crate::mcp_transport::http::McpRemoteProcess;
use crate::mcp_transport::stdio::{JsonRpcStdioFraming, McpStdioProcess};

#[derive(Debug)]
pub(crate) enum McpProcess {
    Stdio(McpStdioProcess),
    Remote(McpRemoteProcess),
}

impl McpProcess {
    pub(crate) fn set_framing_mode(&mut self, framing_mode: JsonRpcStdioFraming) {
        if let Self::Stdio(process) = self {
            process.set_framing_mode(framing_mode);
        }
    }

    pub(crate) async fn initialize(
        &mut self,
        id: JsonRpcId,
        params: McpInitializeParams,
    ) -> io::Result<JsonRpcResponse<McpInitializeResult>> {
        match self {
            Self::Stdio(process) => process.initialize(id, params).await,
            Self::Remote(process) => process.initialize(id, params).await,
        }
    }

    pub(crate) async fn list_tools(
        &mut self,
        id: JsonRpcId,
        params: Option<McpListToolsParams>,
    ) -> io::Result<JsonRpcResponse<McpListToolsResult>> {
        match self {
            Self::Stdio(process) => process.list_tools(id, params).await,
            Self::Remote(process) => process.list_tools(id, params).await,
        }
    }

    pub(crate) async fn call_tool(
        &mut self,
        id: JsonRpcId,
        params: McpToolCallParams,
    ) -> io::Result<JsonRpcResponse<McpToolCallResult>> {
        match self {
            Self::Stdio(process) => process.call_tool(id, params).await,
            Self::Remote(process) => process.call_tool(id, params).await,
        }
    }

    pub(crate) async fn list_resources(
        &mut self,
        id: JsonRpcId,
        params: Option<McpListResourcesParams>,
    ) -> io::Result<JsonRpcResponse<McpListResourcesResult>> {
        match self {
            Self::Stdio(process) => process.list_resources(id, params).await,
            Self::Remote(process) => process.list_resources(id, params).await,
        }
    }

    pub(crate) async fn read_resource(
        &mut self,
        id: JsonRpcId,
        params: McpReadResourceParams,
    ) -> io::Result<JsonRpcResponse<McpReadResourceResult>> {
        match self {
            Self::Stdio(process) => process.read_resource(id, params).await,
            Self::Remote(process) => process.read_resource(id, params).await,
        }
    }

    pub(crate) fn has_exited(&mut self) -> io::Result<bool> {
        match self {
            Self::Stdio(process) => process.has_exited(),
            Self::Remote(_) => Ok(false),
        }
    }

    pub(crate) async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            Self::Stdio(process) => process.shutdown().await,
            Self::Remote(process) => {
                process.shutdown();
                Ok(())
            }
        }
    }
}

pub(crate) fn spawn_mcp_process(bootstrap: &McpClientBootstrap) -> io::Result<McpProcess> {
    match &bootstrap.transport {
        McpClientTransport::Stdio(transport) => {
            Ok(McpProcess::Stdio(McpStdioProcess::spawn(transport)?))
        }
        McpClientTransport::Http(_) | McpClientTransport::Sse(_) => Ok(McpProcess::Remote(
            McpRemoteProcess::new(bootstrap.transport.clone())?,
        )),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "MCP bootstrap transport for {} is not supported: {other:?}",
                bootstrap.server_name
            ),
        )),
    }
}
