//! Resolve agent-loop `maxIterations`: request (turn) > project > cluster.
//! Author: kejiqing

use serde::{Deserialize, Serialize};

/// Where the effective `maxIterations` came from (persisted on `solve_task_json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaxIterationsSource {
    Request,
    Project,
    Cluster,
}

impl MaxIterationsSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Project => "project",
            Self::Cluster => "cluster",
        }
    }
}

/// Validate and resolve turn/project/cluster max iterations.
///
/// - Request `Some(0)` → error (API 400).
/// - Project `Some(0)` or dirty data → treated as unset (fall through to cluster).
pub fn resolve_max_iterations(
    request: Option<usize>,
    project: Option<usize>,
    cluster_default: usize,
) -> Result<(usize, MaxIterationsSource), String> {
    if let Some(n) = request {
        if n == 0 {
            return Err("maxIterations must be >= 1".into());
        }
        return Ok((n, MaxIterationsSource::Request));
    }
    if let Some(n) = project.filter(|&n| n > 0) {
        return Ok((n, MaxIterationsSource::Project));
    }
    let cluster = if cluster_default > 0 {
        cluster_default
    } else {
        64
    };
    Ok((cluster, MaxIterationsSource::Cluster))
}

/// Parse PUT `maxIterations`: omit keep; null clear; positive set; 0 reject.
pub fn parse_project_max_iterations_put(
    patch: Option<Option<usize>>,
    existing: Option<usize>,
) -> Result<Option<usize>, String> {
    match patch {
        None => Ok(existing.filter(|&n| n > 0)),
        Some(None) => Ok(None),
        Some(Some(0)) => Err("maxIterations must be >= 1".into()),
        Some(Some(n)) => Ok(Some(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_matrix_p1_request_wins_over_project() {
        let (n, src) = resolve_max_iterations(Some(3), Some(10), 64).unwrap();
        assert_eq!(n, 3);
        assert_eq!(src, MaxIterationsSource::Request);
    }

    #[test]
    fn priority_matrix_p2_project_when_no_request() {
        let (n, src) = resolve_max_iterations(None, Some(10), 64).unwrap();
        assert_eq!(n, 10);
        assert_eq!(src, MaxIterationsSource::Project);
    }

    #[test]
    fn priority_matrix_p3_cluster_when_both_unset() {
        let (n, src) = resolve_max_iterations(None, None, 64).unwrap();
        assert_eq!(n, 64);
        assert_eq!(src, MaxIterationsSource::Cluster);
    }

    #[test]
    fn priority_matrix_p4_request_without_project() {
        let (n, src) = resolve_max_iterations(Some(3), None, 64).unwrap();
        assert_eq!(n, 3);
        assert_eq!(src, MaxIterationsSource::Request);
    }

    #[test]
    fn priority_matrix_p5_request_zero_errors() {
        let err = resolve_max_iterations(Some(0), Some(10), 64).unwrap_err();
        assert!(err.contains("maxIterations must be >= 1"));
    }

    #[test]
    fn priority_matrix_p6_project_zero_falls_to_cluster() {
        let (n, src) = resolve_max_iterations(None, Some(0), 64).unwrap();
        assert_eq!(n, 64);
        assert_eq!(src, MaxIterationsSource::Cluster);
    }

    #[test]
    fn put_omit_keeps_existing() {
        assert_eq!(
            parse_project_max_iterations_put(None, Some(12)).unwrap(),
            Some(12)
        );
    }

    #[test]
    fn put_null_clears() {
        assert_eq!(
            parse_project_max_iterations_put(Some(None), Some(12)).unwrap(),
            None
        );
    }

    #[test]
    fn put_positive_sets() {
        assert_eq!(
            parse_project_max_iterations_put(Some(Some(8)), None).unwrap(),
            Some(8)
        );
    }

    #[test]
    fn put_zero_rejects() {
        assert!(parse_project_max_iterations_put(Some(Some(0)), None).is_err());
    }

    #[test]
    fn source_serde_camel_case() {
        let v = serde_json::to_value(MaxIterationsSource::Request).unwrap();
        assert_eq!(v, serde_json::json!("request"));
        let back: MaxIterationsSource = serde_json::from_value(v).unwrap();
        assert_eq!(back, MaxIterationsSource::Request);
    }

    #[test]
    fn turn_isolation_resolved_tasks_differ_by_request() {
        // Same project=10, cluster=64: turn A overrides, turn B uses project.
        let (a, src_a) = resolve_max_iterations(Some(2), Some(10), 64).unwrap();
        let (b, src_b) = resolve_max_iterations(None, Some(10), 64).unwrap();
        let (c, src_c) = resolve_max_iterations(None, None, 64).unwrap();
        assert_eq!((a, src_a.as_str()), (2, "request"));
        assert_eq!((b, src_b.as_str()), (10, "project"));
        assert_eq!((c, src_c.as_str()), (64, "cluster"));

        let task_a = serde_json::json!({
            "maxIterations": a,
            "maxIterationsSource": src_a.as_str(),
        });
        let task_b = serde_json::json!({
            "maxIterations": b,
            "maxIterationsSource": src_b.as_str(),
        });
        assert_ne!(task_a, task_b);
        assert_eq!(task_a["maxIterations"], 2);
        assert_eq!(task_b["maxIterationsSource"], "project");
    }

    #[test]
    fn entry_params_keeps_request_only_not_resolved_default() {
        // entry_params should mirror request field (null when unset), not cluster default.
        let req_set = Some(4_usize);
        let req_unset: Option<usize> = None;
        let entry_a = serde_json::json!({ "maxIterations": req_set });
        let entry_b = serde_json::json!({ "maxIterations": req_unset });
        assert_eq!(entry_a["maxIterations"], 4);
        assert!(entry_b["maxIterations"].is_null());
    }
}
