//! MCP `tools/call` argument type gate: validate against `inputSchema`, never coerce.
//! Author: kejiqing

use serde_json::Value;

/// Default max nesting depth (root args = depth 0). Author: kejiqing
pub const MCP_ARG_GATE_MAX_DEPTH: u32 = 32;

/// Default max visited value nodes (objects/arrays/leaves). Author: kejiqing
pub const MCP_ARG_GATE_MAX_NODES: u32 = 8_192;

/// Hard limits for pathological payloads. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpArgGateLimits {
    pub max_depth: u32,
    pub max_nodes: u32,
}

impl Default for McpArgGateLimits {
    fn default() -> Self {
        Self {
            max_depth: MCP_ARG_GATE_MAX_DEPTH,
            max_nodes: MCP_ARG_GATE_MAX_NODES,
        }
    }
}

/// Validate MCP tool `arguments` against discovery `inputSchema`.
///
/// - Missing / non-object schema → skip (Ok).
/// - Does **not** coerce strings to numbers/arrays/objects.
/// - `oneOf` / `anyOf` / `allOf` → skip that subtree.
/// - `type` as JSON array (union) → skip type check at that node.
///
/// Author: kejiqing
pub fn validate_mcp_tool_arguments(
    schema: Option<&Value>,
    arguments: &Value,
) -> Result<(), String> {
    validate_mcp_tool_arguments_with_limits(schema, arguments, McpArgGateLimits::default())
}

/// Same as [`validate_mcp_tool_arguments`] with explicit limits. Author: kejiqing
pub fn validate_mcp_tool_arguments_with_limits(
    schema: Option<&Value>,
    arguments: &Value,
    limits: McpArgGateLimits,
) -> Result<(), String> {
    let Some(schema) = schema else {
        return Ok(());
    };
    if !schema.is_object() {
        return Ok(());
    }
    if !arguments.is_object() {
        return Err(type_mismatch_at(
            "$",
            "object",
            json_type_name(arguments),
        ));
    }
    let mut nodes = 0u32;
    validate_node(schema, arguments, "$", 0, limits, &mut nodes)
}

fn validate_node(
    schema: &Value,
    value: &Value,
    path: &str,
    depth: u32,
    limits: McpArgGateLimits,
    nodes: &mut u32,
) -> Result<(), String> {
    if depth > limits.max_depth {
        return Err(format!(
            "MCP arg gate limit exceeded at {path}: max_depth {} (model tool_use). Call not sent.",
            limits.max_depth
        ));
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > limits.max_nodes {
        return Err(format!(
            "MCP arg gate limit exceeded at {path}: max_nodes {} (model tool_use). Call not sent.",
            limits.max_nodes
        ));
    }

    if schema_has_combinator(schema) {
        return Ok(());
    }

    if let Some(expected) = schema_single_type(schema) {
        check_type(expected, value, path)?;
    }

    match value {
        Value::Object(map) => {
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for req in required {
                    let Some(key) = req.as_str() else {
                        continue;
                    };
                    if !map.contains_key(key) {
                        return Err(missing_required_at(&child_path(path, key)));
                    }
                }
            }
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (key, prop_schema) in props {
                    if let Some(child) = map.get(key) {
                        validate_node(
                            prop_schema,
                            child,
                            &child_path(path, key),
                            depth + 1,
                            limits,
                            nodes,
                        )?;
                    }
                }
            }
            Ok(())
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for (i, item) in items.iter().enumerate() {
                    validate_node(
                        item_schema,
                        item,
                        &index_path(path, i),
                        depth + 1,
                        limits,
                        nodes,
                    )?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn schema_has_combinator(schema: &Value) -> bool {
    schema.get("oneOf").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("allOf").is_some()
}

/// Returns a single concrete type name, or `None` to skip type check. Author: kejiqing
fn schema_single_type(schema: &Value) -> Option<&str> {
    match schema.get("type") {
        Some(Value::String(s)) => Some(s.as_str()),
        // Union `type: ["string","null"]` and missing/other → skip.
        _ => None,
    }
}

fn check_type(expected: &str, value: &Value, path: &str) -> Result<(), String> {
    let ok = match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(n) => n.as_i64().is_some() || n.as_u64().is_some(),
            _ => false,
        },
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        // Unknown schema type keywords: do not reject (avoid false positives).
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(type_mismatch_at(path, expected, json_type_name(value)))
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_mismatch_at(path: &str, expected: &str, actual: &str) -> String {
    format!(
        "MCP arg type mismatch at {path}: schema expects {expected}, got {actual} (model tool_use). Call not sent."
    )
}

fn missing_required_at(path: &str) -> String {
    format!("MCP arg missing required at {path} (model tool_use). Call not sent.")
}

fn child_path(parent: &str, key: &str) -> String {
    if parent == "$" {
        format!("$.{key}")
    } else {
        format!("{parent}.{key}")
    }
}

fn index_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_err_contains(result: Result<(), String>, needles: &[&str]) {
        let err = result.expect_err("expected Err");
        for needle in needles {
            assert!(
                err.contains(needle),
                "error `{err}` missing `{needle}`"
            );
        }
    }

    fn object_schema(properties: &Value, required: &Value) -> Value {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }

    // --- A. scalar type matrix ---

    #[test]
    fn matrix_string_ok_and_mismatches() {
        let schema = object_schema(&json!({"v": {"type": "string"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": ""})).is_ok());
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": "hi"})).is_ok());
        for bad in [
            json!({"v": 1}),
            json!({"v": true}),
            json!({"v": null}),
            json!({"v": {}}),
            json!({"v": []}),
        ] {
            assert_err_contains(
                validate_mcp_tool_arguments(Some(&schema), &bad),
                &["$.v", "expects string", "model tool_use", "Call not sent"],
            );
        }
    }

    #[test]
    fn matrix_number_ok_and_mismatches() {
        let schema = object_schema(&json!({"v": {"type": "number"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": 1})).is_ok());
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": 1.5})).is_ok());
        for bad in [
            json!({"v": "1"}),
            json!({"v": true}),
            json!({"v": null}),
            json!({"v": {}}),
            json!({"v": []}),
        ] {
            assert_err_contains(
                validate_mcp_tool_arguments(Some(&schema), &bad),
                &["$.v", "expects number", "Call not sent"],
            );
        }
    }

    #[test]
    fn matrix_integer_ok_rejects_float_and_string() {
        let schema = object_schema(&json!({"v": {"type": "integer"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": 1})).is_ok());
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": 0})).is_ok());
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"v": 1.5})),
            &["$.v", "expects integer", "got number"],
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"v": "123"})),
            &["expects integer", "got string"],
        );
    }

    #[test]
    fn matrix_boolean_null_object_array() {
        let bool_s = object_schema(&json!({"v": {"type": "boolean"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&bool_s), &json!({"v": true})).is_ok());
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&bool_s), &json!({"v": "true"})),
            &["expects boolean", "got string"],
        );

        let null_s = object_schema(&json!({"v": {"type": "null"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&null_s), &json!({"v": null})).is_ok());
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&null_s), &json!({"v": "null"})),
            &["expects null", "got string"],
        );

        let obj_s = object_schema(&json!({"v": {"type": "object"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&obj_s), &json!({"v": {}})).is_ok());
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&obj_s), &json!({"v": []})),
            &["expects object", "got array"],
        );

        let arr_s = object_schema(&json!({"v": {"type": "array"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&arr_s), &json!({"v": []})).is_ok());
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&arr_s), &json!({"v": {}})),
            &["expects array", "got object"],
        );
    }

    // --- B. Mind experiment shapes ---

    #[test]
    fn experiment_s2_root_object_array_as_string_fails() {
        let schema = object_schema(&json!({
                "items": {
                    "type": "array",
                    "items": { "type": "object" }
                }
            }), &json!(["items"]),
        );
        let args = json!({"items": "[{\"a\":1}]"});
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &args),
            &[
                "MCP arg type mismatch at $.items:",
                "schema expects array, got string",
                "(model tool_use)",
                "Call not sent",
            ],
        );
    }

    #[test]
    fn experiment_s1_tags_string_array_ok() {
        let schema = object_schema(&json!({
                "tags": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }), &json!([]),
        );
        assert!(
            validate_mcp_tool_arguments(Some(&schema), &json!({"tags": ["a", "b"]})).is_ok()
        );
    }

    #[test]
    fn experiment_s3_nested_item_list_ok_and_bad() {
        let schema = object_schema(&json!({
                "payload": {
                    "type": "object",
                    "properties": {
                        "itemList": {
                            "type": "array",
                            "items": { "type": "object" }
                        }
                    }
                }
            }), &json!([]),
        );
        assert!(validate_mcp_tool_arguments(
            Some(&schema),
            &json!({"payload": {"itemList": [{"x": 1}]}})
        )
        .is_ok());
        assert_err_contains(
            validate_mcp_tool_arguments(
                Some(&schema),
                &json!({"payload": {"itemList": "[{\"x\":1}]"}}),
            ),
            &["$.payload.itemList", "expects array, got string"],
        );
    }

    // --- C. nested / items paths ---

    #[test]
    fn nested_property_path_and_array_index_path() {
        let schema = object_schema(&json!({
                "a": {
                    "type": "object",
                    "properties": {
                        "b": { "type": "string" }
                    }
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "x": { "type": "number" }
                        }
                    }
                }
            }), &json!([]),
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"a": {"b": 1}})),
            &["$.a.b", "expects string", "got number"],
        );
        assert_err_contains(
            validate_mcp_tool_arguments(
                Some(&schema),
                &json!({"items": [{"x": "nope"}]}),
            ),
            &["$.items[0].x", "expects number", "got string"],
        );
        assert_err_contains(
            validate_mcp_tool_arguments(
                Some(&schema),
                &json!({"items": ["bad"]}),
            ),
            &["$.items[0]", "expects object", "got string"],
        );
    }

    #[test]
    fn empty_array_with_items_constraint_ok() {
        let schema = object_schema(&json!({
                "items": {
                    "type": "array",
                    "items": { "type": "object" }
                }
            }), &json!([]),
        );
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"items": []})).is_ok());
    }

    #[test]
    fn deep_nesting_path_three_levels() {
        let schema = object_schema(&json!({
                "l1": {
                    "type": "object",
                    "properties": {
                        "l2": {
                            "type": "object",
                            "properties": {
                                "l3": { "type": "boolean" }
                            }
                        }
                    }
                }
            }), &json!([]),
        );
        assert_err_contains(
            validate_mcp_tool_arguments(
                Some(&schema),
                &json!({"l1": {"l2": {"l3": 1}}}),
            ),
            &["$.l1.l2.l3", "expects boolean"],
        );
    }

    // --- D. required ---

    #[test]
    fn required_missing_reports_first() {
        let schema = object_schema(&json!({
                "foo": { "type": "string" },
                "bar": { "type": "string" }
            }), &json!(["foo", "bar"]),
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({})),
            &[
                "MCP arg missing required at $.foo",
                "model tool_use",
                "Call not sent",
            ],
        );
        assert!(validate_mcp_tool_arguments(
            Some(&schema),
            &json!({"foo": "a", "bar": "b"})
        )
        .is_ok());
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"foo": "a"})).is_err());
    }

    #[test]
    fn required_present_but_wrong_type_prefers_type_error() {
        let schema = object_schema(&json!({"foo": {"type": "string"}}), &json!(["foo"]),
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"foo": 1})),
            &["MCP arg type mismatch at $.foo:", "expects string", "got number"],
        );
    }

    #[test]
    fn optional_field_absent_ok() {
        let schema = object_schema(&json!({"foo": {"type": "string"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({})).is_ok());
    }

    #[test]
    fn required_key_without_properties_entry_still_checked_for_presence() {
        let schema = json!({
            "type": "object",
            "properties": {},
            "required": ["orphan"]
        });
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({})),
            &["missing required at $.orphan"],
        );
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"orphan": 1})).is_ok());
    }

    // --- E. skip / pass ---

    #[test]
    fn missing_or_non_object_schema_skips() {
        assert!(validate_mcp_tool_arguments(None, &json!({"a": 1})).is_ok());
        assert!(validate_mcp_tool_arguments(Some(&json!("nope")), &json!({"a": 1})).is_ok());
        assert!(validate_mcp_tool_arguments(Some(&json!(null)), &json!({"a": 1})).is_ok());
    }

    #[test]
    fn property_without_type_still_recurses() {
        let schema = object_schema(&json!({
                "wrap": {
                    "properties": {
                        "inner": { "type": "string" }
                    }
                }
            }), &json!([]),
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"wrap": {"inner": 1}})),
            &["$.wrap.inner", "expects string"],
        );
    }

    #[test]
    fn union_type_array_skips_type_check() {
        let schema = object_schema(&json!({"v": {"type": ["string", "null"]}}), &json!([]),
        );
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": 123})).is_ok());
    }

    #[test]
    fn one_of_any_of_all_of_skip_subtree() {
        for key in ["oneOf", "anyOf", "allOf"] {
            let schema = object_schema(&json!({
                    "v": {
                        key: [{"type": "string"}, {"type": "number"}],
                        "type": "string"
                    }
                }), &json!([]),
            );
            // Combinator present → skip entire subtree (including type: string).
            assert!(
                validate_mcp_tool_arguments(Some(&schema), &json!({"v": 1})).is_ok(),
                "combinator {key} should skip"
            );
        }
    }

    #[test]
    fn additional_properties_false_extra_keys_still_ok() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "string" }
            },
            "additionalProperties": false
        });
        assert!(
            validate_mcp_tool_arguments(Some(&schema), &json!({"a": "x", "extra": 1})).is_ok()
        );
    }

    #[test]
    fn args_root_must_be_object() {
        let schema = json!({"type": "object"});
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!([])),
            &["at $:", "expects object", "got array"],
        );
    }

    #[test]
    fn max_depth_fail_closed() {
        let schema = object_schema(&json!({
                "a": {
                    "type": "object",
                    "properties": {
                        "b": {
                            "type": "object",
                            "properties": {
                                "c": { "type": "string" }
                            }
                        }
                    }
                }
            }), &json!([]),
        );
        let limits = McpArgGateLimits {
            max_depth: 1,
            max_nodes: 10_000,
        };
        assert_err_contains(
            validate_mcp_tool_arguments_with_limits(
                Some(&schema),
                &json!({"a": {"b": {"c": "x"}}}),
                limits,
            ),
            &["max_depth", "Call not sent"],
        );
    }

    #[test]
    fn max_nodes_fail_closed() {
        let schema = object_schema(&json!({
                "items": {
                    "type": "array",
                    "items": { "type": "number" }
                }
            }), &json!([]),
        );
        let limits = McpArgGateLimits {
            max_depth: 32,
            max_nodes: 3,
        };
        assert_err_contains(
            validate_mcp_tool_arguments_with_limits(
                Some(&schema),
                &json!({"items": [1, 2, 3, 4]}),
                limits,
            ),
            &["max_nodes", "Call not sent"],
        );
    }

    // --- F. no coerce ---

    #[test]
    fn no_coerce_stringified_array_number_bool() {
        let arr = object_schema(&json!({"items": {"type": "array"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(
            Some(&arr),
            &json!({"items": "[{\"a\":1}]"})
        )
        .is_err());

        let num = object_schema(&json!({"n": {"type": "number"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&num), &json!({"n": "123"})).is_err());

        let integer = object_schema(&json!({"n": {"type": "integer"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&integer), &json!({"n": "123"})).is_err());

        let b = object_schema(&json!({"v": {"type": "boolean"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&b), &json!({"v": "true"})).is_err());
    }

    // --- G. error contract ---

    #[test]
    fn error_message_contract_type_and_missing() {
        let schema = object_schema(&json!({"items": {"type": "array"}, "foo": {"type": "string"}}), &json!(["foo"]),
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"items": "x", "foo": "y"})),
            &[
                "MCP arg type mismatch at $.items:",
                "schema expects array, got string",
                "(model tool_use)",
                "Call not sent",
            ],
        );
        assert_err_contains(
            validate_mcp_tool_arguments(Some(&schema), &json!({"items": []})),
            &[
                "MCP arg missing required at $.foo",
                "(model tool_use)",
                "Call not sent.",
            ],
        );
    }

    // --- H. no rewrite (pure function returns unit; args unchanged by caller) ---

    #[test]
    fn validate_does_not_mutate_arguments() {
        let schema = object_schema(&json!({"n": {"type": "number"}}), &json!([]));
        let mut args = json!({"n": 1, "extra": "keep"});
        let before = args.clone();
        validate_mcp_tool_arguments(Some(&schema), &args).unwrap();
        assert_eq!(args, before);
        // Force borrow to prove we only need &Value
        let _ = &mut args;
    }

    #[test]
    fn unknown_schema_type_keyword_does_not_reject() {
        let schema = object_schema(&json!({"v": {"type": "date"}}), &json!([]));
        assert!(validate_mcp_tool_arguments(Some(&schema), &json!({"v": "x"})).is_ok());
    }
}
