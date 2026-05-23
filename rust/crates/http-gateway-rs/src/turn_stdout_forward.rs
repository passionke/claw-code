//! Bridge pool exec stdout lines to hub (in-process) or HTTP ingest. Author: kejiqing

use std::sync::Arc;

use crate::turn_stdout_hub::{forward_claw_stdout_line, TurnStdoutHub};

/// Called for each stdout line from `docker exec` while `gateway-solve-once` runs.
pub async fn handle_claw_stdout_line(
    turn_id: &str,
    line: &str,
    hub: Option<&Arc<TurnStdoutHub>>,
) {
    let Some(value) = gateway_solve_turn::gateway_stdout::parse_stdout_line(line) else {
        return;
    };
    if let Some(h) = hub {
        h.ingest_json(turn_id, &value);
        return;
    }
    forward_claw_stdout_line(turn_id, line).await;
}
