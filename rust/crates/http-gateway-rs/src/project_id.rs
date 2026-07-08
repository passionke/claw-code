//! Project identifier type and query-param parsing (projId / legacy dsId). Author: kejiqing

/// Gateway project id (API `projId`, disk `proj_<id>/`). Author: kejiqing
pub type ProjectId = i64;

/// Axum / handler query: accept `projId`, `proj_id`, `dsId`, or `ds_id`. Author: kejiqing
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
pub struct ProjectIdQuery {
    #[serde(default, rename = "projId")]
    pub proj_id: Option<ProjectId>,
    #[serde(default, rename = "proj_id")]
    pub proj_id_snake: Option<ProjectId>,
    #[serde(default, rename = "dsId")]
    pub ds_id_camel: Option<ProjectId>,
    #[serde(default, rename = "ds_id")]
    pub ds_id_snake: Option<ProjectId>,
}

impl ProjectIdQuery {
    #[must_use]
    pub fn resolve(self) -> Option<ProjectId> {
        self.proj_id
            .or(self.proj_id_snake)
            .or(self.ds_id_camel)
            .or(self.ds_id_snake)
    }
}

/// Required project id from query (returns None when missing or invalid). Author: kejiqing
#[must_use]
pub fn parse_project_id_query(q: &ProjectIdQuery) -> Option<ProjectId> {
    q.resolve().filter(|&id| id >= 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_proj_id_aliases() {
        for json in [
            r#"{"projId":42}"#,
            r#"{"proj_id":42}"#,
            r#"{"dsId":42}"#,
            r#"{"ds_id":42}"#,
        ] {
            let q: ProjectIdQuery = serde_json::from_str(json).unwrap();
            assert_eq!(parse_project_id_query(&q), Some(42));
        }
    }

    #[test]
    fn rejects_zero_and_missing() {
        let q: ProjectIdQuery = serde_json::from_str(r#"{"projId":0}"#).unwrap();
        assert_eq!(parse_project_id_query(&q), None);
        let q: ProjectIdQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(parse_project_id_query(&q), None);
    }
}
