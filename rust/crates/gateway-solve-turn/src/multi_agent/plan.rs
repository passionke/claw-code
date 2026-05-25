//! Analysis plan schema for multi-agent gateway solve. Author: kejiqing

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisPlan {
    pub plan_title: String,
    pub todos: Vec<AnalysisPlanTodo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisPlanTodo {
    pub id: String,
    pub title: String,
    pub question: String,
}

impl<'de> Deserialize<'de> for AnalysisPlanTodo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            id: Value,
            title: String,
            question: String,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            id: value_to_string(raw.id).map_err(serde::de::Error::custom)?,
            title: raw.title,
            question: raw.question,
        })
    }
}

fn value_to_string(v: Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        other => Err(format!("expected string id, got {other}")),
    }
}

impl AnalysisPlan {
  pub fn validate(&self) -> Result<(), String> {
        let title = self.plan_title.trim();
        if title.is_empty() {
            return Err(String::from("planTitle must not be empty"));
        }
        if self.todos.is_empty() {
            return Err(String::from("todos must not be empty"));
        }
        let mut seen = std::collections::HashSet::new();
        for todo in &self.todos {
            let id = todo.id.trim();
            let t = todo.title.trim();
            let q = todo.question.trim();
            if id.is_empty() || t.is_empty() || q.is_empty() {
                return Err(String::from("each todo requires id, title, question"));
            }
            if !seen.insert(id.to_string()) {
                return Err(format!("duplicate todo id: {id}"));
            }
        }
        Ok(())
    }

    /// Progress todos for `report_progress` / task-progress.json.
    #[must_use]
    pub fn progress_todos_pending(&self) -> Vec<crate::task_progress::TaskProgressTodo> {
        self.todos
            .iter()
            .map(|t| crate::task_progress::TaskProgressTodo {
                id: t.id.clone(),
                title: t.title.clone(),
                status: String::from("pending"),
            })
            .collect()
    }
}

/// Extract JSON object from model text (fenced ```json block or raw object).
pub fn parse_plan_from_text(text: &str) -> Result<AnalysisPlan, String> {
    let trimmed = text.trim();
    if let Ok(plan) = serde_json::from_str::<AnalysisPlan>(trimmed) {
        plan.validate()?;
        return Ok(plan);
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let slice = &trimmed[start..=end];
                let plan: AnalysisPlan =
                    serde_json::from_str(slice).map_err(|e| format!("plan JSON parse: {e}"))?;
                plan.validate()?;
                return Ok(plan);
            }
        }
    }
    if let Some(fence) = trimmed.split("```json").nth(1) {
        let body = fence.split("```").next().unwrap_or(fence).trim();
        if let Ok(plan) = serde_json::from_str::<AnalysisPlan>(body) {
            plan.validate()?;
            return Ok(plan);
        }
    }
    if let Some(fence) = trimmed.split("```").nth(1) {
        let body = fence.split("```").next().unwrap_or(fence).trim();
        if let Ok(plan) = serde_json::from_str::<AnalysisPlan>(body) {
            plan.validate()?;
            return Ok(plan);
        }
    }
    Err(String::from("could not parse AnalysisPlan JSON from planner output"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_from_raw_json() {
        let text = r#"{"planTitle":"销售分析","todos":[{"id":"t1","title":"趋势","question":"近7天销售趋势？"}]}"#;
        let plan = parse_plan_from_text(text).unwrap();
        assert_eq!(plan.plan_title, "销售分析");
        assert_eq!(plan.todos.len(), 1);
    }
}
