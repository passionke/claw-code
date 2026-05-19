//! Store/org id → display name cache for user-visible progress (session-scoped). Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gateway_solve_session_persistence_path;

fn is_query_mcp_tool_name(tool_name: &str) -> bool {
    tool_name.contains("mcp_question")
}

const ENTITY_LABELS_FILE: &str = "entity-labels.json";
const MIN_ENTITY_ID_LEN: usize = 12;

/// Non-empty `store_id` / `org_id` only (whitespace-only counts as empty).
#[must_use]
pub fn is_usable_entity_id(id: &str) -> bool {
    !id.trim().is_empty()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityLabelMap {
    #[serde(default)]
    pub stores: BTreeMap<String, String>,
    #[serde(default)]
    pub orgs: BTreeMap<String, String>,
}

#[must_use]
pub fn entity_labels_path(session_home: &Path) -> std::path::PathBuf {
    session_home.join(".claw").join(ENTITY_LABELS_FILE)
}

/// Fast path for progress display: disk cache + `extraSession` (no jsonl rescan).
#[must_use]
pub fn entity_labels_for_progress(
    session_home: &Path,
    extra_session: Option<&Value>,
) -> EntityLabelMap {
    let mut map = read_entity_labels_file(session_home).unwrap_or_default();
    drop_empty_entity_id_entries(&mut map);
    merge_extra_session_labels(&mut map, extra_session);
    map
}

fn drop_empty_entity_id_entries(map: &mut EntityLabelMap) {
    map.stores.retain(|id, _| is_usable_entity_id(id.as_str()));
    map.orgs.retain(|id, _| is_usable_entity_id(id.as_str()));
}

pub fn load_entity_labels(
    session_home: &Path,
    extra_session: Option<&Value>,
) -> Result<EntityLabelMap, String> {
    let mut map = read_entity_labels_file(session_home)?;
    drop_empty_entity_id_entries(&mut map);
    merge_extra_session_labels(&mut map, extra_session);
    refresh_entity_labels_from_session_jsonl(session_home, &mut map)?;
    write_entity_labels_file(session_home, &map)?;
    Ok(map)
}

pub fn ingest_entity_labels_from_mcp_response(
    session_home: &Path,
    extra_session: Option<&Value>,
    question_args: &Value,
    output: &str,
    is_error: bool,
) -> Result<(), String> {
    if is_error {
        return Ok(());
    }
    let question = question_text_from_args(question_args);
    if question.is_empty() || !is_entity_name_lookup_question(&question) {
        return Ok(());
    }
    let mut map = load_entity_labels(session_home, extra_session)?;
    let ids = scan_entity_ids(&question);
    if ids.is_empty() {
        return Ok(());
    }
    let mut changed = false;
    for (kind, id) in ids {
        if !is_usable_entity_id(&id) {
            continue;
        }
        if map.get_label(kind, &id).is_some() {
            continue;
        }
        if let Some(name) = extract_display_name_for_id(output, kind, &id) {
            map.insert_label(kind, id, &name);
            changed = true;
        }
    }
    if changed {
        write_entity_labels_file(session_home, &map)?;
    }
    Ok(())
}

/// Replace known `store_id` / `org_id` tokens in user-visible progress text.
#[must_use]
pub fn substitute_entity_ids_in_text(text: &str, labels: &EntityLabelMap) -> String {
    if labels.stores.is_empty() && labels.orgs.is_empty() {
        return text.to_string();
    }
    let mut pairs: Vec<(&str, &str)> = labels
        .stores
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .chain(
            labels
                .orgs
                .iter()
                .map(|(id, name)| (id.as_str(), name.as_str())),
        )
        .collect();
    pairs.sort_by_key(|(id, _)| std::cmp::Reverse(id.len()));
    let mut out = text.to_string();
    for (id, name) in pairs {
        if !is_usable_entity_id(id) || name.is_empty() || name == id {
            continue;
        }
        out = out.replace(id, name);
    }
    out
}

impl EntityLabelMap {
    fn get_label(&self, kind: char, id: &str) -> Option<&str> {
        if !is_usable_entity_id(id) {
            return None;
        }
        match kind {
            'S' => self.stores.get(id).map(String::as_str),
            'O' => self.orgs.get(id).map(String::as_str),
            _ => None,
        }
    }

    fn insert_label(&mut self, kind: char, id: String, name: &str) {
        if !is_usable_entity_id(&id) {
            return;
        }
        let name = name.trim().to_string();
        if name.is_empty() || name == id {
            return;
        }
        match kind {
            'S' => {
                self.stores.insert(id, name);
            }
            'O' => {
                self.orgs.insert(id, name);
            }
            _ => {}
        }
    }
}

fn read_entity_labels_file(session_home: &Path) -> Result<EntityLabelMap, String> {
    let path = entity_labels_path(session_home);
    if !path.is_file() {
        return Ok(EntityLabelMap::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read entity labels: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse entity labels: {e}"))
}

fn write_entity_labels_file(session_home: &Path, map: &EntityLabelMap) -> Result<(), String> {
    if let Some(parent) = entity_labels_path(session_home).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create entity labels dir: {e}"))?;
    }
    let bytes =
        serde_json::to_vec_pretty(map).map_err(|e| format!("serialize entity labels: {e}"))?;
    let path = entity_labels_path(session_home);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("write entity labels temp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename entity labels: {e}"))?;
    Ok(())
}

fn merge_extra_session_labels(map: &mut EntityLabelMap, extra_session: Option<&Value>) {
    let Some(extra) = extra_session else {
        return;
    };
    pair_extra_field(map, extra, "store_id", "store_name", 'S');
    pair_extra_field(map, extra, "org_id", "org_name", 'O');
}

fn pair_extra_field(
    map: &mut EntityLabelMap,
    extra: &Value,
    id_key: &str,
    name_key: &str,
    kind: char,
) {
    let Some(id) = extra.get(id_key).and_then(Value::as_str) else {
        return;
    };
    let id = id.trim();
    if id.is_empty() {
        return;
    }
    if let Some(name) = extra.get(name_key).and_then(Value::as_str) {
        let name = name.trim();
        if !name.is_empty() {
            map.insert_label(kind, id.to_string(), name);
        }
    }
}

fn refresh_entity_labels_from_session_jsonl(
    session_home: &Path,
    map: &mut EntityLabelMap,
) -> Result<(), String> {
    let path = gateway_solve_session_persistence_path(session_home);
    if !path.is_file() {
        return Ok(());
    }
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("read session jsonl for labels: {e}"))?;
    let mut pending_uses: HashMap<String, (String, String)> = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(msg) = record.get("message") else {
            continue;
        };
        let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let Some(id) = block.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(name) = block.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    if !is_query_mcp_tool_name(name) {
                        continue;
                    }
                    let input = block
                        .get("input")
                        .and_then(Value::as_str)
                        .unwrap_or("{}")
                        .to_string();
                    pending_uses.insert(id.to_string(), (name.to_string(), input));
                }
                Some("tool_result") => {
                    let Some(use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let is_error = block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    if is_error {
                        pending_uses.remove(use_id);
                        continue;
                    }
                    let Some((tool_name, input)) = pending_uses.remove(use_id) else {
                        continue;
                    };
                    if !is_query_mcp_tool_name(&tool_name) {
                        continue;
                    }
                    let output = block.get("output").and_then(Value::as_str).unwrap_or("");
                    let args = serde_json::from_str::<Value>(&input).unwrap_or_else(|_| json!({}));
                    let question = question_text_from_args(&args);
                    if question.is_empty() || !is_entity_name_lookup_question(&question) {
                        continue;
                    }
                    for (kind, id) in scan_entity_ids(&question) {
                        if !is_usable_entity_id(&id) {
                            continue;
                        }
                        if map.get_label(kind, &id).is_some() {
                            continue;
                        }
                        if let Some(name) = extract_display_name_for_id(output, kind, &id) {
                            map.insert_label(kind, id, &name);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[must_use]
pub fn question_text_from_args(args: &Value) -> String {
    for key in ["question", "query", "prompt", "message", "text"] {
        if let Some(s) = args.get(key).and_then(Value::as_str) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

#[must_use]
pub fn is_entity_name_lookup_question(question: &str) -> bool {
    question.contains("注册名称")
        || question.contains("门店名称")
        || question.contains("机构名称")
        || ((question.contains("查询门店") || question.contains("查询机构"))
            && question.contains("名称"))
}

#[must_use]
pub fn scan_entity_ids(text: &str) -> Vec<(char, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == 'S' || c == 'O' {
            let kind = c;
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let id: String = chars[start..i].iter().collect();
            if id.len() >= MIN_ENTITY_ID_LEN {
                out.push((kind, id));
            }
        } else {
            i += 1;
        }
    }
    out
}

fn extract_display_name_for_id(output: &str, kind: char, id: &str) -> Option<String> {
    if let Ok(outer) = serde_json::from_str::<Value>(output) {
        if let Some(text) = outer.pointer("/content/0/text").and_then(Value::as_str) {
            if let Ok(inner) = serde_json::from_str::<Value>(text) {
                if let Some(name) = find_label_in_json(&inner, kind, id) {
                    return Some(name);
                }
            }
            if let Some(name) = find_label_in_plaintext(text, kind, id) {
                return Some(name);
            }
        }
        if let Some(name) = find_label_in_json(&outer, kind, id) {
            return Some(name);
        }
    }
    find_label_in_plaintext(output, kind, id)
}

fn find_label_in_json(value: &Value, kind: char, id: &str) -> Option<String> {
    let mut found = None;
    walk_json_for_labels(value, kind, id, &mut found);
    found
}

fn walk_json_for_labels(value: &Value, kind: char, id: &str, found: &mut Option<String>) {
    if found.is_some() {
        return;
    }
    match value {
        Value::Object(map) => {
            let id_key = if kind == 'S' { "store_id" } else { "org_id" };
            if map.get(id_key).and_then(Value::as_str) == Some(id) {
                for key in name_field_keys(kind) {
                    if let Some(name) = map.get(*key).and_then(Value::as_str) {
                        if is_plausible_label(name, id) {
                            *found = Some(name.trim().to_string());
                            return;
                        }
                    }
                }
            }
            for key in name_field_keys(kind) {
                if let Some(name) = map.get(*key).and_then(Value::as_str) {
                    if is_plausible_label(name, id) && text_mentions_id(map, id) {
                        *found = Some(name.trim().to_string());
                        return;
                    }
                }
            }
            for v in map.values() {
                walk_json_for_labels(v, kind, id, found);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                walk_json_for_labels(v, kind, id, found);
            }
        }
        _ => {}
    }
}

fn name_field_keys(kind: char) -> &'static [&'static str] {
    if kind == 'S' {
        &[
            "store_name",
            "shop_name",
            "name",
            "storeName",
            "shopName",
            "门店名称",
            "注册名称",
        ]
    } else {
        &["org_name", "name", "orgName", "机构名称", "注册名称"]
    }
}

fn text_mentions_id(map: &serde_json::Map<String, Value>, id: &str) -> bool {
    map.values().any(|v| {
        v.as_str()
            .is_some_and(|s| s.contains(id) || scan_entity_ids(s).iter().any(|(_, x)| x == id))
    })
}

fn is_plausible_label(name: &str, id: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name != id
        && !name.starts_with('S')
        && !name.starts_with('O')
        && name.chars().count() <= 80
}

fn find_label_in_plaintext(text: &str, kind: char, id: &str) -> Option<String> {
    let patterns: &[&str] = if kind == 'S' {
        &[
            "注册名称",
            "门店名称",
            "门店名",
            "店铺名称",
            "store_name",
            "shop_name",
        ]
    } else {
        &["注册名称", "机构名称", "机构名", "org_name"]
    };
    for pat in patterns {
        if let Some(name) = extract_after_label_pattern(text, pat, id) {
            return Some(name);
        }
    }
    None
}

fn extract_after_label_pattern(text: &str, label: &str, id: &str) -> Option<String> {
    let pos = text.find(label)?;
    let tail = &text[pos + label.len()..];
    let tail = tail.trim_start_matches(['：', ':', ' ', '\t', '"', '\'']);
    let end = tail
        .find(['\n', '\r', '，', ',', '。', ';', '；'])
        .unwrap_or(tail.len().min(60));
    let candidate = tail[..end].trim();
    if is_plausible_label(candidate, id) {
        return Some(candidate.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scan_finds_store_and_org_ids() {
        let ids =
            scan_entity_ids("查询门店 S20241007172800004204 与机构 O20241007172800004204 的名称");
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].0, 'S');
        assert_eq!(ids[1].0, 'O');
    }

    #[test]
    fn substitute_replaces_known_ids() {
        let mut map = EntityLabelMap::default();
        map.insert_label('S', "S20241007172800004204".to_string(), "测试门店");
        let out = substitute_entity_ids_in_text("统计门店 S20241007172800004204 营业额", &map);
        assert!(out.contains("测试门店"));
        assert!(!out.contains("S20241007172800004204"));
    }

    #[test]
    fn extra_session_seeds_labels() {
        let mut map = EntityLabelMap::default();
        let extra = json!({
            "store_id": "S20241007172800004204",
            "store_name": "外滩店"
        });
        merge_extra_session_labels(&mut map, Some(&extra));
        assert_eq!(
            map.stores.get("S20241007172800004204").map(String::as_str),
            Some("外滩店")
        );
    }

    #[test]
    fn detects_name_lookup_question() {
        assert!(is_entity_name_lookup_question(
            "查询门店 S20241007172800004204 的注册名称"
        ));
        assert!(!is_entity_name_lookup_question(
            "统计门店 S20241007172800004204 在 2026-05-07 的销售总额"
        ));
    }

    #[test]
    fn empty_store_or_org_id_is_not_substituted() {
        let mut map = EntityLabelMap::default();
        map.stores.insert(String::new(), "不应替换".to_string());
        map.orgs.insert("   ".to_string(), "也不应替换".to_string());
        map.insert_label('S', "S20241007172800004204".to_string(), "有效门店");
        let out = substitute_entity_ids_in_text("统计门店 S20241007172800004204", &map);
        assert!(out.contains("有效门店"));
        assert!(!out.contains("不应替换"));
    }

    #[test]
    fn extra_session_ignores_empty_store_and_org_id() {
        let mut map = EntityLabelMap::default();
        let extra = json!({
            "store_id": "",
            "store_name": "忽略店名",
            "org_id": "  ",
            "org_name": "忽略机构"
        });
        merge_extra_session_labels(&mut map, Some(&extra));
        assert!(map.stores.is_empty());
        assert!(map.orgs.is_empty());
    }

    #[test]
    fn insert_label_rejects_empty_id() {
        let mut map = EntityLabelMap::default();
        map.insert_label('S', String::new(), "店");
        map.insert_label('O', "  ".to_string(), "机构");
        assert!(map.stores.is_empty());
        assert!(map.orgs.is_empty());
    }
}
