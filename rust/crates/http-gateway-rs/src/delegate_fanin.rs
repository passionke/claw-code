//! Router delegate live SSE / progress fan-in helpers. Author: kejiqing

use crate::master_observer::PROJECT_ROLE_ROUTER;
use crate::session_db::{ActiveDelegateRecord, GatewaySessionDb};

/// Specialist turn to stream when router has an active delegate. Author: kejiqing
pub async fn active_delegate_for_router_live(
    db: &GatewaySessionDb,
    router_proj_id: i64,
    router_turn_id: &str,
) -> Result<Option<ActiveDelegateRecord>, String> {
    let role = db
        .get_project_role(router_proj_id)
        .await
        .map_err(|e| e.to_string())?;
    if role != PROJECT_ROLE_ROUTER {
        return Ok(None);
    }
    let Some(active) = db
        .active_delegate_from_turn(router_turn_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let status = db
        .get_turn_status(&active.turn_id, &active.session_id, active.proj_id)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if matches!(status.as_str(), "running" | "queued") {
        Ok(Some(active))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_delegate_record_parses_timing_json() {
        let v = serde_json::json!({
            "activeDelegate": {
                "sessionId": "dgt_abc",
                "turnId": "T_spec",
                "projId": 99012,
                "delegateProjId": 99012
            }
        });
        let rec = ActiveDelegateRecord::from_timing_store(&v).expect("parse");
        assert_eq!(rec.session_id, "dgt_abc");
        assert_eq!(rec.turn_id, "T_spec");
    }

    #[test]
    fn active_delegate_record_from_stdout_defaults_delegate_proj_id() {
        let v = serde_json::json!({
            "ev": "delegate.active",
            "sessionId": "dgt_def",
            "turnId": "T_def",
            "projId": 99011
        });
        let rec = ActiveDelegateRecord::from_stdout_value(&v).expect("parse");
        assert_eq!(rec.delegate_proj_id, 99011);
    }

    #[test]
    fn merged_delegate_progress_dedupes_events() {
        use gateway_solve_turn::ProgressEvent;
        let store = serde_json::json!({
            "delegateProgressArchive": [
                {"kind":"report_progress","message":"kb","tsMs":1000}
            ]
        });
        let router = vec![ProgressEvent {
            kind: "report_progress".into(),
            message: "router".into(),
            ts_ms: 2000,
        }];
        let spec = vec![
            ProgressEvent {
                kind: "report_progress".into(),
                message: "kb".into(),
                ts_ms: 1000,
            },
            ProgressEvent {
                kind: "mcp_tool_started".into(),
                message: "sql".into(),
                ts_ms: 3000,
            },
        ];
        let merged = GatewaySessionDb::merged_delegate_progress_events(&store, &router, &spec, 50);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].message, "kb");
        assert_eq!(merged[1].message, "router");
        assert_eq!(merged[2].message, "sql");
    }
}
