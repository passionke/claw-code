//! SQLBot solve preflight: MCP calls → session `home/*.md` + short transcript summaries.
//! Gateway `ds_*` is workspace only; SQLBot `datasource_id` comes from MCP token (single row in list).
//! Author: kejiqing

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{DirectToolExecutor, GatewaySolveTurnError};
use runtime::{
    ContentBlock, ConversationMessage, Session, ToolExecutor, GATEWAY_SCHEMA_MD_REL,
    GATEWAY_SQLBOT_MCP_DATASOURCE_EXAMPLES_TOOL, GATEWAY_SQLBOT_MCP_DATASOURCE_LIST_TOOL,
    GATEWAY_SQLBOT_MCP_DATASOURCE_TABLES_TOOL, GATEWAY_SQLBOT_MCP_DATASOURCE_TERMINOLOGIES_TOOL,
    GATEWAY_SQLBOT_MCP_START_TOOL, GATEWAY_SQL_EXAMPLES_MD_REL, GATEWAY_TABLES_AND_RELS_MD_REL,
    GATEWAY_TERMINOLOGIES_MD_REL,
};
use serde_json::{json, Value};
use tracing::warn;

pub(crate) const PREFLIGHT_ENV: &str = "CLAW_GATEWAY_SQLBOT_PREFLIGHT";

const PREFLIGHT_LOG_TARGET: &str = "claw_sqlbot_preflight";

#[derive(Debug, Clone)]
struct SqlbotCredentials {
    token: String,
}

pub(crate) fn sqlbot_preflight_enabled() -> bool {
    match std::env::var(PREFLIGHT_ENV) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "off" | "no")
        }
        Err(_) => true,
    }
}

fn warn_skip(step: &str, reason: &str) {
    warn!(
        target: PREFLIGHT_LOG_TARGET,
        step,
        reason,
        "sqlbot preflight step skipped"
    );
}

fn next_preflight_tool_use_id(step: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("claw_preflight_{step}_{nanos}")
}

fn inject_tool_exchange(
    session: &mut Session,
    tool_use_id: String,
    tool_name: &str,
    input: &str,
    output: String,
    is_error: bool,
) -> Result<(), GatewaySolveTurnError> {
    session
        .push_message(ConversationMessage::assistant(vec![
            ContentBlock::ToolUse {
                id: tool_use_id.clone(),
                name: tool_name.to_string(),
                input: input.to_string(),
            },
        ]))
        .map_err(|e| {
            crate::err(
                crate::HTTP_INTERNAL,
                format!("preflight persist assistant tool_use ({tool_name}): {e}"),
            )
        })?;
    session
        .push_message(ConversationMessage::tool_result(
            tool_use_id,
            tool_name,
            output,
            is_error,
        ))
        .map_err(|e| {
            crate::err(
                crate::HTTP_INTERNAL,
                format!("preflight persist tool_result ({tool_name}): {e}"),
            )
        })?;
    Ok(())
}

fn inject_assistant_text(session: &mut Session, text: &str) -> Result<(), GatewaySolveTurnError> {
    session
        .push_message(ConversationMessage::assistant(vec![ContentBlock::Text {
            text: text.to_string(),
        }]))
        .map_err(|e| {
            crate::err(
                crate::HTTP_INTERNAL,
                format!("preflight persist assistant note: {e}"),
            )
        })
}

fn sqlbot_payload_is_error(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    if lower.contains("\"iserror\":true") || lower.contains("\"error\"") {
        return true;
    }
    parse_sqlbot_inner_json(output)
        .ok()
        .and_then(|v| v.get("code").and_then(Value::as_i64))
        .is_some_and(|code| code != 0)
}

fn parse_sqlbot_inner_json(output: &str) -> Result<Value, String> {
    let outer: Value = serde_json::from_str(output).map_err(|e| format!("outer json: {e}"))?;
    if let Some(text) = outer.pointer("/content/0/text").and_then(Value::as_str) {
        return serde_json::from_str(text).map_err(|e| format!("inner json: {e}"));
    }
    Ok(outer)
}

fn execute_preflight_mcp(
    executor: &mut DirectToolExecutor,
    tool_name: &str,
    input: &str,
) -> Result<String, String> {
    if !executor.allows_tool(tool_name) {
        return Err(format!("tool not allowed: {tool_name}"));
    }
    executor
        .execute(tool_name, input)
        .map_err(|e| format!("mcp execute: {e}"))
}

fn inject_preflight_summary(
    session: &mut Session,
    step: &str,
    tool_name: &str,
    input: &str,
    summary: &str,
) -> Result<(), GatewaySolveTurnError> {
    inject_tool_exchange(
        session,
        next_preflight_tool_use_id(step),
        tool_name,
        input,
        summary.to_string(),
        false,
    )
}

fn materialize_summary_json(msg: &str, path_rel: &str, extra: &Value) -> String {
    serde_json::to_string(&json!({
        "code": 0,
        "msg": msg,
        "data": {
            "materialized_path": path_rel,
            "extra": extra,
        }
    }))
    .unwrap_or_else(|_| format!("{{\"materialized_path\":\"{path_rel}\"}}"))
}

fn write_session_home_md(
    session_home: &Path,
    rel: &str,
    markdown: &str,
) -> Result<(), GatewaySolveTurnError> {
    let path = session_home.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::err(
                crate::HTTP_INTERNAL,
                format!("preflight mkdir {}: {e}", parent.display()),
            )
        })?;
    }
    std::fs::write(&path, markdown).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight write {}: {e}", path.display()),
        )
    })
}

fn run_preflight_mcp_required(
    session: &mut Session,
    executor: &mut DirectToolExecutor,
    step: &str,
    tool_name: &str,
    input: &str,
) -> Result<String, GatewaySolveTurnError> {
    let output = execute_preflight_mcp(executor, tool_name, input).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight {tool_name} failed: {e}"),
        )
    })?;
    let is_error = sqlbot_payload_is_error(&output);
    inject_tool_exchange(
        session,
        next_preflight_tool_use_id(step),
        tool_name,
        input,
        output.clone(),
        is_error,
    )?;
    if is_error {
        return Err(crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight {tool_name} returned error payload"),
        ));
    }
    Ok(output)
}

fn credentials_from_start_inner(inner: &Value) -> Option<SqlbotCredentials> {
    let data = inner.get("data")?;
    let token = data.get("access_token")?.as_str()?.to_string();
    data.get("chat_id").and_then(Value::as_i64)?;
    Some(SqlbotCredentials { token })
}

fn sole_datasource_row(inner: &Value) -> Result<&Value, GatewaySolveTurnError> {
    let arr = inner.get("data").and_then(Value::as_array).ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_datasource_list missing data array",
        )
    })?;
    match arr.len() {
        0 => Err(crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_datasource_list empty (check MCP token / SQLBot datasource binding)",
        )),
        1 => Ok(&arr[0]),
        n => Err(crate::err(
            crate::HTTP_INTERNAL,
            format!(
                "preflight: mcp_datasource_list returned {n} datasources; use an MCP token scoped to exactly one"
            ),
        )),
    }
}

fn sole_datasource_id_from_row(row: &Value) -> Result<i64, GatewaySolveTurnError> {
    row.get("id").and_then(Value::as_i64).ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: sole datasource row missing id",
        )
    })
}

fn graph_cell_table_name(cell: &Value) -> Option<&str> {
    cell.pointer("/attrs/text/text")
        .and_then(Value::as_str)
        .or_else(|| cell.pointer("/attrs/label/text").and_then(Value::as_str))
}

fn format_tables_and_rels_md(row: &Value) -> Option<String> {
    let mut out = String::from(
        "# Tables and relations (SQLBot `mcp_datasource_list`)\n\n\
         Scoped by MCP token. Regenerated on each session first turn.\n\n",
    );
    out.push_str("## Datasource\n\n");
    for key in ["id", "name", "type_name", "status", "description"] {
        if let Some(v) = row.get(key) {
            if let Some(s) = v.as_str() {
                out.push_str(&format!("- **{key}**: {s}\n"));
            } else if let Some(n) = v.as_i64() {
                out.push_str(&format!("- **{key}**: {n}\n"));
            }
        }
    }
    out.push('\n');
    let tr = row.get("table_relation")?.as_array()?;
    let mut id_to_name: HashMap<i64, String> = HashMap::new();
    let mut edges: Vec<(i64, i64)> = Vec::new();
    for cell in tr {
        let shape = cell.get("shape").and_then(Value::as_str).unwrap_or("");
        if shape == "er-rect" {
            let id = cell.get("id").and_then(Value::as_i64)?;
            let name = graph_cell_table_name(cell)?.to_string();
            id_to_name.insert(id, name);
        } else if shape == "edge" {
            let src = cell.get("source")?.get("cell")?.as_i64()?;
            let tgt = cell.get("target")?.get("cell")?.as_i64()?;
            edges.push((src, tgt));
        }
    }
    if !id_to_name.is_empty() {
        out.push_str("## Tables in relation graph\n\n");
        let mut names: Vec<_> = id_to_name.values().cloned().collect();
        names.sort();
        for name in names {
            out.push_str(&format!("- `{name}`\n"));
        }
        out.push('\n');
    }
    if !edges.is_empty() {
        out.push_str("## Relations (from graph)\n\n");
        out.push_str("| from | to |\n| --- | --- |\n");
        for (src, tgt) in edges {
            let from = id_to_name.get(&src).map(String::as_str).unwrap_or("?");
            let to = id_to_name.get(&tgt).map(String::as_str).unwrap_or("?");
            out.push_str(&format!("| `{from}` | `{to}` |\n"));
        }
        out.push('\n');
    }
    Some(out)
}

fn format_tables_inner_to_schema_md(inner: &Value) -> Option<String> {
    let mut out = format!(
        "# Table schema (SQLBot `mcp_datasource_tables`)\n\n\
         Scoped by MCP token. Regenerated on each session first turn.\n\n"
    );
    let data = inner.get("data")?;
    let tables = data.as_array()?;
    if tables.is_empty() {
        out.push_str("_No tables returned._\n");
        return Some(out);
    }
    for table in tables {
        let name = table
            .get("table_name")
            .or_else(|| table.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown_table");
        out.push_str(&format!("## {name}\n\n"));
        if let Some(ddl) = table
            .get("ddl")
            .or_else(|| table.get("create_sql"))
            .or_else(|| table.get("createTableSql"))
            .and_then(Value::as_str)
        {
            out.push_str("```sql\n");
            out.push_str(ddl.trim());
            out.push_str("\n```\n\n");
            continue;
        }
        if let Some(cols) = table
            .get("fields")
            .or_else(|| table.get("columns"))
            .and_then(Value::as_array)
        {
            out.push_str("| column | type | comment |\n| --- | --- | --- |\n");
            for col in cols {
                let cname = col
                    .get("field_name")
                    .or_else(|| col.get("name"))
                    .or_else(|| col.get("column_name"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                let ctype = col
                    .get("field_type")
                    .or_else(|| col.get("type"))
                    .or_else(|| col.get("data_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                let comment = col
                    .get("comment")
                    .or_else(|| col.get("remarks"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                out.push_str(&format!("| {cname} | {ctype} | {comment} |\n"));
            }
            out.push('\n');
            continue;
        }
        out.push_str("```json\n");
        out.push_str(&serde_json::to_string_pretty(table).unwrap_or_default());
        out.push_str("\n```\n\n");
    }
    Some(out)
}

fn format_terminologies_md(inner: &Value) -> Option<String> {
    let items = inner.get("data")?.as_array()?;
    let mut out = String::from(
        "# Terminologies (SQLBot `mcp_datasource_terminologies`)\n\n\
         Business terms and field meanings for this MCP token scope.\n\n",
    );
    if items.is_empty() {
        out.push_str("_No terminologies returned._\n");
        return Some(out);
    }
    for item in items {
        let word = item.get("word").and_then(Value::as_str).unwrap_or("-");
        out.push_str(&format!("## {word}\n\n"));
        if let Some(desc) = item.get("description").and_then(Value::as_str) {
            out.push_str(desc);
            out.push_str("\n\n");
        }
        if let Some(other) = item.get("other_words").and_then(Value::as_array) {
            if !other.is_empty() {
                out.push_str("**Aliases:** ");
                let words: Vec<_> = other.iter().filter_map(Value::as_str).collect();
                out.push_str(&words.join(", "));
                out.push_str("\n\n");
            }
        }
    }
    Some(out)
}

fn format_sql_examples_md(inner: &Value) -> Option<String> {
    let items = inner.get("data")?.as_array()?;
    let mut out = String::from(
        "# SQL examples (SQLBot `mcp_datasource_examples`)\n\n\
         Few-shot questions and SQL for this MCP token scope.\n\n",
    );
    if items.is_empty() {
        out.push_str("_No examples returned._\n");
        return Some(out);
    }
    for (i, item) in items.iter().enumerate() {
        let title = item
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("Example");
        out.push_str(&format!("## Example {}\n\n", i + 1));
        out.push_str(&format!("**Question:** {title}\n\n"));
        if let Some(sql) = item
            .get("description")
            .or_else(|| item.get("sql"))
            .and_then(Value::as_str)
        {
            out.push_str("```sql\n");
            out.push_str(sql.trim());
            out.push_str("\n```\n\n");
        }
    }
    Some(out)
}

fn try_materialize_from_mcp(
    session_home: &Path,
    session: &mut Session,
    executor: &mut DirectToolExecutor,
    step: &str,
    tool_name: &str,
    input: &str,
    path_rel: &str,
    format_md: fn(&Value) -> Option<String>,
) {
    let output = match execute_preflight_mcp(executor, tool_name, input) {
        Ok(o) => o,
        Err(e) => {
            warn_skip(step, &e);
            return;
        }
    };
    if sqlbot_payload_is_error(&output) {
        warn_skip(step, "MCP returned error payload");
        return;
    }
    let inner = match parse_sqlbot_inner_json(&output) {
        Ok(v) => v,
        Err(e) => {
            warn_skip(step, &e);
            return;
        }
    };
    let markdown = match format_md(&inner) {
        Some(m) if !m.trim().is_empty() => m,
        _ => {
            warn_skip(step, "empty or unparseable markdown");
            return;
        }
    };
    if let Err(e) = write_session_home_md(session_home, path_rel, &markdown) {
        warn_skip(step, &e.message);
        return;
    }
    let extra = json!({});
    let summary =
        materialize_summary_json(&format!("preflight: wrote {path_rel}"), path_rel, &extra);
    if let Err(e) = inject_preflight_summary(session, step, tool_name, input, &summary) {
        warn_skip(step, &e.message);
    }
}

fn try_materialize_list_tables_and_rels(
    session_home: &Path,
    session: &mut Session,
    list_inner: &Value,
    list_input: &str,
) {
    let row = match sole_datasource_row(list_inner) {
        Ok(r) => r,
        Err(e) => {
            warn_skip("tables_and_rels", &e.message);
            return;
        }
    };
    let markdown = match format_tables_and_rels_md(row) {
        Some(m) => m,
        None => {
            warn_skip("tables_and_rels", "could not format list row");
            return;
        }
    };
    if let Err(e) = write_session_home_md(session_home, GATEWAY_TABLES_AND_RELS_MD_REL, &markdown) {
        warn_skip("tables_and_rels", &e.message);
        return;
    }
    let table_count = row
        .get("table_relation")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|c| c.get("shape").and_then(Value::as_str) == Some("er-rect"))
                .count()
        })
        .unwrap_or(0);
    let extra = json!({ "table_count": table_count });
    let summary = materialize_summary_json(
        "preflight: tables and relations written from datasource list",
        GATEWAY_TABLES_AND_RELS_MD_REL,
        &extra,
    );
    if let Err(e) = inject_preflight_summary(
        session,
        "datasource_list",
        GATEWAY_SQLBOT_MCP_DATASOURCE_LIST_TOOL,
        list_input,
        &summary,
    ) {
        warn_skip("tables_and_rels", &e.message);
    }
}

fn datasource_tool_input(token: &str, datasource_id: i64) -> Result<String, GatewaySolveTurnError> {
    serde_json::to_string(&json!({
        "token": token,
        "datasource_id": datasource_id,
    }))
    .map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: encode datasource tool input: {e}"),
        )
    })
}

/// First-turn SQLBot preflight: materialize `home/*.md` + transcript summaries.
pub(crate) fn run_sqlbot_preflight(
    session_home: &Path,
    session: &mut Session,
    executor: &mut DirectToolExecutor,
) -> Result<(), GatewaySolveTurnError> {
    if !sqlbot_preflight_enabled() {
        return Ok(());
    }
    let start_output = run_preflight_mcp_required(
        session,
        executor,
        "mcp_start",
        GATEWAY_SQLBOT_MCP_START_TOOL,
        "{}",
    )?;
    let start_inner = parse_sqlbot_inner_json(&start_output).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: parse mcp_start output: {e}"),
        )
    })?;
    let creds = credentials_from_start_inner(&start_inner).ok_or_else(|| {
        crate::err(
            crate::HTTP_INTERNAL,
            "preflight: mcp_start missing access_token or chat_id",
        )
    })?;

    let list_input = serde_json::to_string(&json!({ "token": creds.token })).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: encode mcp_datasource_list input: {e}"),
        )
    })?;
    let list_output = execute_preflight_mcp(
        executor,
        GATEWAY_SQLBOT_MCP_DATASOURCE_LIST_TOOL,
        &list_input,
    )
    .map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight mcp_datasource_list failed: {e}"),
        )
    })?;
    if sqlbot_payload_is_error(&list_output) {
        return Err(crate::err(
            crate::HTTP_INTERNAL,
            "preflight mcp_datasource_list returned error payload",
        ));
    }
    let list_inner = parse_sqlbot_inner_json(&list_output).map_err(|e| {
        crate::err(
            crate::HTTP_INTERNAL,
            format!("preflight: parse mcp_datasource_list output: {e}"),
        )
    })?;
    let datasource_id = sole_datasource_id_from_row(sole_datasource_row(&list_inner)?)?;

    try_materialize_list_tables_and_rels(session_home, session, &list_inner, &list_input);

    let ds_input = datasource_tool_input(&creds.token, datasource_id)?;
    try_materialize_from_mcp(
        session_home,
        session,
        executor,
        "datasource_tables",
        GATEWAY_SQLBOT_MCP_DATASOURCE_TABLES_TOOL,
        &ds_input,
        GATEWAY_SCHEMA_MD_REL,
        format_tables_inner_to_schema_md,
    );
    try_materialize_from_mcp(
        session_home,
        session,
        executor,
        "datasource_terminologies",
        GATEWAY_SQLBOT_MCP_DATASOURCE_TERMINOLOGIES_TOOL,
        &ds_input,
        GATEWAY_TERMINOLOGIES_MD_REL,
        format_terminologies_md,
    );
    try_materialize_from_mcp(
        session_home,
        session,
        executor,
        "datasource_examples",
        GATEWAY_SQLBOT_MCP_DATASOURCE_EXAMPLES_TOOL,
        &ds_input,
        GATEWAY_SQL_EXAMPLES_MD_REL,
        format_sql_examples_md,
    );

    inject_assistant_text(
        session,
        "[Gateway SQLBot preflight] Session context files (read with Read/bash):\n\
         - `home/schema.md` — table DDL / columns\n\
         - `home/tables_and_rels.md` — tables and relation graph from list\n\
         - `home/terminologies.md` — business terminology\n\
         - `home/sql_examples.md` — few-shot SQL examples\n\
         Missing files were skipped (see gateway logs). \
         `access_token` and `chat_id` are in the latest `mcp_start` tool_result above.",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn sole_datasource_requires_exactly_one_row() {
        let one = json!({"data": [{"id": 34, "name": "boss"}]});
        assert_eq!(
            sole_datasource_id_from_row(sole_datasource_row(&one).unwrap()).unwrap(),
            34
        );
        let many = json!({"data": [{"id": 1}, {"id": 2}]});
        assert!(sole_datasource_row(&many).is_err());
    }

    #[test]
    fn format_tables_and_rels_from_graph() {
        let row = json!({
            "id": 27,
            "name": "boss",
            "table_relation": [
                {"id": 1, "shape": "er-rect", "attrs": {"text": {"text": "t_a"}}},
                {"id": 2, "shape": "er-rect", "attrs": {"text": {"text": "t_b"}}},
                {"shape": "edge", "source": {"cell": 1}, "target": {"cell": 2}}
            ]
        });
        let md = format_tables_and_rels_md(&row).expect("md");
        assert!(md.contains("t_a"));
        assert!(md.contains("t_b"));
        assert!(md.contains("| `t_a` | `t_b` |"));
    }

    #[test]
    fn format_terminologies_and_examples() {
        let term = json!({"data": [{"word": "AOV", "description": "avg order value"}]});
        assert!(format_terminologies_md(&term).unwrap().contains("AOV"));
        let ex = json!({"data": [{"question": "q1", "description": "SELECT 1"}]});
        assert!(format_sql_examples_md(&ex).unwrap().contains("SELECT 1"));
    }
}
