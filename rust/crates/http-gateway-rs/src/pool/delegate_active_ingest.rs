//! Ingest `delegate.active` / `delegate.clear` stdout into router turn PG. Author: kejiqing

use std::sync::Arc;

use serde_json::Value;
use tracing::warn;

use crate::pool_consumer_resolve;
use crate::session_db::{ActiveDelegateRecord, GatewaySessionDb};

const PROGRESS_ARCHIVE_LIMIT: usize = 500;

/// Handle one structured stdout event for router delegate fan-in registry.
pub async fn ingest_delegate_stdout_event(
    db: &GatewaySessionDb,
    router_turn_id: &str,
    value: &Value,
) {
    let ev = value.get("ev").and_then(Value::as_str).unwrap_or("");
    match ev {
        "delegate.active" => {
            if let Some(record) = ActiveDelegateRecord::from_stdout_value(value) {
                if let Err(e) = db.set_turn_active_delegate(router_turn_id, &record).await {
                    warn!(
                        target: "claw_delegate_fanin",
                        router_turn_id = %router_turn_id,
                        error = %e,
                        "set_turn_active_delegate failed"
                    );
                }
            }
        }
        "delegate.clear" => {
            let archive = match db.active_delegate_from_turn(router_turn_id).await {
                Ok(Some(active)) => pool_consumer_resolve::resolve_turn_progress(
                    db,
                    &active.turn_id,
                    PROGRESS_ARCHIVE_LIMIT,
                )
                .await
                .ok()
                .map(|snap| snap.events)
                .unwrap_or_default(),
                _ => Vec::new(),
            };
            if let Err(e) = db
                .clear_turn_active_delegate(router_turn_id, &archive)
                .await
            {
                warn!(
                    target: "claw_delegate_fanin",
                    router_turn_id = %router_turn_id,
                    error = %e,
                    "clear_turn_active_delegate failed"
                );
            }
        }
        _ => {}
    }
}

/// Spawn ingest for delegate stdout lines alongside live hub ingest.
pub fn spawn_delegate_active_consumer(
    router_turn_id: String,
    db: Arc<GatewaySessionDb>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Value>,
) {
    tokio::spawn(async move {
        while let Some(value) = rx.recv().await {
            ingest_delegate_stdout_event(db.as_ref(), &router_turn_id, &value).await;
        }
    });
}

/// Route stdout JSON to delegate ingest when ev matches.
#[must_use]
pub fn is_delegate_registry_event(value: &Value) -> bool {
    matches!(
        value.get("ev").and_then(Value::as_str),
        Some("delegate.active" | "delegate.clear")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_delegate_registry_events() {
        assert!(is_delegate_registry_event(&json!({"ev":"delegate.active"})));
        assert!(is_delegate_registry_event(&json!({"ev":"delegate.clear"})));
        assert!(!is_delegate_registry_event(&json!({"ev":"report.delta"})));
    }
}
