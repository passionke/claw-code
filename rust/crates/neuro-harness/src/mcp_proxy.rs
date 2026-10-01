//! `mcp-proxy`: stdio MCP server the engine talks to; forwards to the real server through
//! `runtime::McpServerManager` and injects the gateway `_meta.extra_session` on every `tools/call`.
//! Contract: `docs/neuro-harness-contract.md` §7. Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use runtime::{build_mcp_call_meta, ConfigLoader, McpServerManager};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::turn_context::read_turn_context;

const PROTOCOL_VERSION: &str = "2025-06-18";

struct Proxy {
    server: String,
    session_root: PathBuf,
    manager: McpServerManager,
    /// raw tool name → manager-qualified name; filled by discovery.
    tools: Option<HashMap<String, String>>,
    listed: Vec<Value>,
}

/// Serve MCP over stdin/stdout until EOF.
pub async fn run(server: &str, session_root: &Path) -> Result<(), String> {
    let loader = ConfigLoader::default_for(session_root);
    let config = loader.load().map_err(|e| format!("load mcp config: {e}"))?;
    let scoped = config
        .mcp()
        .servers()
        .get(server)
        .cloned()
        .ok_or_else(|| format!("mcp server {server} not found in session settings"))?;
    let mut servers = BTreeMap::new();
    servers.insert(server.to_string(), scoped);
    let mut proxy = Proxy {
        server: server.to_string(),
        session_root: session_root.to_path_buf(),
        manager: McpServerManager::from_servers(&servers),
        tools: None,
        listed: Vec::new(),
    };

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            eprintln!("[mcp-proxy {server}] ignored non-JSON line");
            continue;
        };
        let Some(reply) = proxy.handle(&msg).await else {
            continue;
        };
        let mut out = reply.to_string();
        out.push('\n');
        stdout
            .write_all(out.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdout.flush().await.map_err(|e| e.to_string())?;
    }
    let _ = proxy.manager.shutdown().await;
    Ok(())
}

impl Proxy {
    /// Reply for requests; `None` for notifications.
    async fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": format!("neuro-mcp-proxy/{}", self.server), "version": env!("CARGO_PKG_VERSION")},
            })),
            "ping" => Ok(json!({})),
            "tools/list" => self.list().await.map(|tools| json!({"tools": tools})),
            "tools/call" => self.call(&params).await,
            _ => Err((-32601, format!("method not supported by proxy: {method}"))),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        })
    }

    async fn discover(&mut self) -> Result<&HashMap<String, String>, (i64, String)> {
        if self.tools.is_none() {
            let found = self
                .manager
                .discover_tools()
                .await
                .map_err(|e| (-32603, format!("discover tools on {}: {e}", self.server)))?;
            let mut map = HashMap::new();
            let mut listed = Vec::new();
            for t in found {
                map.insert(t.raw_name.clone(), t.qualified_name.clone());
                let mut tool = t.tool.clone();
                tool.name = t.raw_name;
                listed.push(serde_json::to_value(tool).unwrap_or(Value::Null));
            }
            self.listed = listed;
            self.tools = Some(map);
        }
        Ok(self.tools.get_or_insert_with(HashMap::new))
    }

    async fn list(&mut self) -> Result<Vec<Value>, (i64, String)> {
        self.discover().await?;
        Ok(self.listed.clone())
    }

    async fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "tools/call without name".to_string()))?
            .to_string();
        let qualified = self
            .discover()
            .await?
            .get(&name)
            .cloned()
            .ok_or((-32602, format!("unknown tool {name}")))?;
        let meta = merge_meta(params.get("_meta"), &self.gateway_meta()?);
        let args = params.get("arguments").cloned();
        match self.manager.call_tool(&qualified, args, Some(meta)).await {
            Ok(resp) => {
                if let Some(err) = resp.error {
                    return Err((err.code, err.message));
                }
                let result = resp.result.map_or(Value::Null, |r| {
                    serde_json::to_value(r).unwrap_or(Value::Null)
                });
                Ok(strip_nulls(result))
            }
            Err(e) => Ok(json!({
                "content": [{"type": "text", "text": format!("mcp call failed: {e}")}],
                "isError": true,
            })),
        }
    }

    fn gateway_meta(&self) -> Result<Value, (i64, String)> {
        let ctx = read_turn_context(&self.session_root).map_err(|e| (-32603, e))?;
        Ok(build_mcp_call_meta(&ctx.to_mcp_call_context()))
    }
}

/// Engine `_meta` + gateway keys; gateway wins on collisions.
fn merge_meta(engine: Option<&Value>, gateway: &Value) -> Value {
    let mut out: Map<String, Value> = engine
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(g) = gateway.as_object() {
        for (k, v) in g {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_meta_wins() {
        let engine = json!({"progressToken": 2, "extra_session": {"x": 1}});
        let gateway = json!({"extra_session": {"_claw_session_id": "s"}});
        let merged = merge_meta(Some(&engine), &gateway);
        assert_eq!(merged["progressToken"], 2);
        assert_eq!(merged["extra_session"], json!({"_claw_session_id": "s"}));
    }

    #[test]
    fn strip_nulls_top_level() {
        let v = strip_nulls(json!({"content": [], "structuredContent": null, "isError": false}));
        assert_eq!(v, json!({"content": [], "isError": false}));
    }
}
