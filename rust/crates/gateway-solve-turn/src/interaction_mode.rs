//! Solve interaction mode: agent (default) vs plan (read-only alignment). Author: kejiqing

use serde::{Deserialize, Serialize};

/// Chat / solve interaction mode. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionMode {
    #[default]
    Agent,
    Plan,
}

impl InteractionMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Plan => "plan",
        }
    }

    #[must_use]
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).unwrap_or("") {
            "plan" => Self::Plan,
            _ => Self::Agent,
        }
    }

    #[must_use]
    pub fn is_plan(self) -> bool {
        matches!(self, Self::Plan)
    }
}

/// Optional turn options carried on the gateway task file. Author: kejiqing
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveTurnOptions {
    #[serde(default)]
    pub interaction_mode: InteractionMode,
    /// Force single_turn path (skip multi_agent_analysis). Author: kejiqing
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force_single_turn: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_plan_markdown: Option<String>,
    /// Whether AskUserQuestion is registered for this turn. Author: kejiqing
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ask_user_question_enabled: bool,
}

/// Extract a short title from plan markdown (`#` heading or first non-empty line). Author: kejiqing
#[must_use]
pub fn plan_title_from_markdown(body: &str) -> String {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let title = rest.trim().trim_start_matches('#').trim();
            if !title.is_empty() {
                return title.chars().take(200).collect();
            }
        }
        return t.chars().take(200).collect();
    }
    String::from("未命名方案")
}

/// Derive todo titles from `## 实施步骤` numbered list (fallback: whole steps section lines). Author: kejiqing
#[must_use]
pub fn todos_from_plan_markdown(body: &str) -> Vec<String> {
    let mut in_steps = false;
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            let heading = t.trim_start_matches('#').trim();
            in_steps = heading.contains("实施步骤") || heading.eq_ignore_ascii_case("steps");
            continue;
        }
        if !in_steps {
            continue;
        }
        if t.is_empty() {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        let item = t
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['.', ')', '、', ' '])
            .trim();
        if !item.is_empty() {
            out.push(item.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_heading() {
        assert_eq!(
            plan_title_from_markdown("# 方案：导出 Excel\n\n## 目标\n"),
            "方案：导出 Excel"
        );
    }

    #[test]
    fn todos_from_steps() {
        let md = r#"# 方案

## 实施步骤
1. 加按钮
2. 接导出 API
3. 单测

## 验收
- ok
"#;
        assert_eq!(
            todos_from_plan_markdown(md),
            vec![
                "加按钮".to_string(),
                "接导出 API".to_string(),
                "单测".to_string()
            ]
        );
    }
}
