//! Gateway session inbox: receive / drain / query (PG SoT). Author: kejiqing

use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::inbox_capacity::InboxCapacity;
use crate::master_observer::PROJECT_ROLE_STEERABLE;
use crate::session_db::{now_ms_for_registry, GatewaySessionDb};

pub const INBOX_STATUS_QUEUED: &str = "queued";
pub const INBOX_STATUS_CONSUMED: &str = "consumed";
pub const INBOX_STATUS_DROPPED: &str = "dropped";
pub const INBOX_SOURCE_USER: &str = "user";
pub const INBOX_SOURCE_MAILBOX: &str = "mailbox";
pub const INBOX_DROP_OVERFLOW: &str = "overflow";

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InboxMessageRow {
    pub message_id: String,
    pub session_id: String,
    pub proj_id: i64,
    pub source: String,
    pub body: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InboxLimitsView {
    pub queue_limit: u32,
    pub body_max_bytes: usize,
    pub drain_batch: u32,
    pub drain_max_bytes: usize,
}

impl From<InboxCapacity> for InboxLimitsView {
    fn from(c: InboxCapacity) -> Self {
        Self {
            queue_limit: c.queue_limit,
            body_max_bytes: c.body_max_bytes,
            drain_batch: c.drain_batch,
            drain_max_bytes: c.drain_max_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InboxSummary {
    pub queued: i64,
    pub consumed: i64,
    pub dropped: i64,
    pub queued_by_source: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_received_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_consumed_at_ms: Option<i64>,
    pub limits: InboxLimitsView,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InboxEnqueueResult {
    pub message_id: String,
    pub status: String,
    pub idempotent_hit: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InboxDrainResult {
    pub messages: Vec<InboxMessageRow>,
    pub remaining_queued: i64,
    pub limits: InboxLimitsView,
}

#[derive(Debug)]
pub enum InboxError {
    BadRequest(String),
    Forbidden(String),
    NotFound(String),
    Db(SqlxError),
}

impl From<SqlxError> for InboxError {
    fn from(e: SqlxError) -> Self {
        Self::Db(e)
    }
}

impl std::fmt::Display for InboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(m) | Self::Forbidden(m) | Self::NotFound(m) => write!(f, "{m}"),
            Self::Db(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InboxError {}

pub fn validate_inbox_source(source: &str) -> Result<&str, InboxError> {
    match source.trim() {
        INBOX_SOURCE_USER | INBOX_SOURCE_MAILBOX => Ok(source.trim()),
        other => Err(InboxError::BadRequest(format!(
            "invalid source={other:?}; expected user|mailbox"
        ))),
    }
}

fn new_message_id() -> String {
    format!("sim_{}", Uuid::new_v4().simple())
}

fn row_from_sql(r: &sqlx::postgres::PgRow) -> InboxMessageRow {
    let references = r
        .try_get::<Option<sqlx::types::Json<Vec<String>>>, _>("references_json")
        .ok()
        .flatten()
        .map(|j| j.0)
        .unwrap_or_default();
    InboxMessageRow {
        message_id: r.get("message_id"),
        session_id: r.get("session_id"),
        proj_id: r.get("proj_id"),
        source: r.get("source"),
        body: r.get("body"),
        status: r.get("status"),
        from_address: r.try_get("from_address").ok().flatten(),
        in_reply_to: r.try_get("in_reply_to").ok().flatten(),
        references,
        idempotency_key: r.get("idempotency_key"),
        created_at_ms: r.get("created_at_ms"),
        consumed_at_ms: r.get("consumed_at_ms"),
        consumed_turn_id: r.get("consumed_turn_id"),
        iteration: r.get("iteration"),
        dropped_at_ms: r.get("dropped_at_ms"),
        dropped_reason: r.get("dropped_reason"),
    }
}

impl GatewaySessionDb {
    /// Ensure project is steerable before inbox ops. Author: kejiqing
    pub async fn assert_project_steerable(&self, proj_id: i64) -> Result<(), InboxError> {
        let role = self
            .get_project_role(proj_id)
            .await
            .map_err(InboxError::Db)?;
        if role != PROJECT_ROLE_STEERABLE {
            return Err(InboxError::Forbidden(format!(
                "project_role={role} cannot use inbox; need steerable"
            )));
        }
        Ok(())
    }

    /// Receive one message into the session inbox (bounded). Author: kejiqing
    pub async fn inbox_enqueue(
        &self,
        session_id: &str,
        proj_id: i64,
        source: &str,
        body: &str,
        idempotency_key: Option<&str>,
        capacity: InboxCapacity,
        from_address: Option<&str>,
        in_reply_to: Option<&str>,
        references: Option<&[String]>,
    ) -> Result<InboxEnqueueResult, InboxError> {
        self.assert_project_steerable(proj_id).await?;
        let source = validate_inbox_source(source)?;
        if body.len() > capacity.body_max_bytes {
            return Err(InboxError::BadRequest(format!(
                "body exceeds CLAW_INBOX_BODY_MAX_BYTES={}",
                capacity.body_max_bytes
            )));
        }
        let from_address = from_address
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                gateway_solve_turn::parse_mailbox_address(s)
                    .map(|a| a.format())
                    .map_err(InboxError::BadRequest)
            })
            .transpose()?;
        if source == INBOX_SOURCE_MAILBOX && from_address.is_none() {
            return Err(InboxError::BadRequest(
                "fromAddress is required when source=mailbox".into(),
            ));
        }
        let in_reply_to = in_reply_to
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let references_json = references
            .map(|r| {
                r.iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|r| !r.is_empty())
            .map(sqlx::types::Json);

        let session_ok = self
            .get_session_home_rel(session_id, proj_id)
            .await
            .map_err(InboxError::Db)?;
        if session_ok.is_none() {
            return Err(InboxError::NotFound(format!(
                "session not found: {session_id} proj_id={proj_id}"
            )));
        }

        let idem = idempotency_key
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let mut tx = self.pg_pool().begin().await?;
        if let Some(ref key) = idem {
            if let Some(existing) =
                fetch_by_idempotency(&mut tx, self.cluster_id(), session_id, key).await?
            {
                return Ok(InboxEnqueueResult {
                    message_id: existing.message_id,
                    status: existing.status,
                    idempotent_hit: true,
                });
            }
        }

        loop {
            let queued: i64 = sqlx::query_scalar(
                r"SELECT COUNT(*) FROM session_inbox_messages
                  WHERE cluster_id = $1 AND session_id = $2 AND status = $3",
            )
            .bind(self.cluster_id())
            .bind(session_id)
            .bind(INBOX_STATUS_QUEUED)
            .fetch_one(&mut *tx)
            .await?;
            if queued < i64::from(capacity.queue_limit) {
                break;
            }
            let oldest = sqlx::query(
                r"SELECT message_id FROM session_inbox_messages
                  WHERE cluster_id = $1 AND session_id = $2 AND status = $3
                  ORDER BY created_at_ms ASC, message_id ASC
                  LIMIT 1
                  FOR UPDATE",
            )
            .bind(self.cluster_id())
            .bind(session_id)
            .bind(INBOX_STATUS_QUEUED)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = oldest else { break };
            let mid: String = row.get("message_id");
            let now = now_ms_for_registry();
            sqlx::query(
                r"UPDATE session_inbox_messages
                  SET status = $4, dropped_at_ms = $5, dropped_reason = $6
                  WHERE cluster_id = $1 AND message_id = $2 AND status = $3",
            )
            .bind(self.cluster_id())
            .bind(&mid)
            .bind(INBOX_STATUS_QUEUED)
            .bind(INBOX_STATUS_DROPPED)
            .bind(now)
            .bind(INBOX_DROP_OVERFLOW)
            .execute(&mut *tx)
            .await?;
        }

        let message_id = new_message_id();
        let now = now_ms_for_registry();
        sqlx::query(
            r"INSERT INTO session_inbox_messages (
                cluster_id, message_id, session_id, proj_id, source, body, status,
                idempotency_key, created_at_ms, from_address, in_reply_to, references_json
              ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(self.cluster_id())
        .bind(&message_id)
        .bind(session_id)
        .bind(proj_id)
        .bind(source)
        .bind(body)
        .bind(INBOX_STATUS_QUEUED)
        .bind(idem.as_deref())
        .bind(now)
        .bind(from_address.as_deref())
        .bind(in_reply_to.as_deref())
        .bind(references_json)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(InboxEnqueueResult {
            message_id,
            status: INBOX_STATUS_QUEUED.to_string(),
            idempotent_hit: false,
        })
    }

    /// Atomic FIFO drain (bounded by capacity). Author: kejiqing
    pub async fn inbox_drain(
        &self,
        session_id: &str,
        proj_id: i64,
        turn_id: &str,
        iteration: i32,
        capacity: InboxCapacity,
    ) -> Result<InboxDrainResult, InboxError> {
        self.assert_project_steerable(proj_id).await?;
        let turn_id = turn_id.trim();
        if turn_id.is_empty() {
            return Err(InboxError::BadRequest("turnId required".into()));
        }
        let turn_status: Option<String> = sqlx::query_scalar(
            r"SELECT status FROM gateway_turns
              WHERE cluster_id = $1 AND turn_id = $2 AND session_id = $3 AND proj_id = $4
              LIMIT 1",
        )
        .bind(self.cluster_id())
        .bind(turn_id)
        .bind(session_id)
        .bind(proj_id)
        .fetch_optional(self.pg_pool())
        .await?;
        let Some(status) = turn_status else {
            return Err(InboxError::Forbidden(
                "turn not found for session; drain requires active turn".into(),
            ));
        };
        if !matches!(status.as_str(), "queued" | "running") {
            return Err(InboxError::Forbidden(format!(
                "turn status={status} cannot drain; need queued|running"
            )));
        }

        let mut tx = self.pg_pool().begin().await?;
        let candidates = sqlx::query(
            r"SELECT message_id, session_id, proj_id, source, body, status, idempotency_key,
                     created_at_ms, consumed_at_ms, consumed_turn_id, iteration,
                     dropped_at_ms, dropped_reason,
                     from_address, in_reply_to, references_json
              FROM session_inbox_messages
              WHERE cluster_id = $1 AND session_id = $2 AND status = $3
              ORDER BY created_at_ms ASC, message_id ASC
              FOR UPDATE",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(INBOX_STATUS_QUEUED)
        .fetch_all(&mut *tx)
        .await?;

        let body_lens: Vec<usize> = candidates
            .iter()
            .map(|r| {
                let body: String = r.get("body");
                body.len()
            })
            .collect();
        let take = capacity.plan_drain(&body_lens);
        let now = now_ms_for_registry();
        let mut messages = Vec::with_capacity(take);
        for row in candidates.into_iter().take(take) {
            let mut msg = row_from_sql(&row);
            sqlx::query(
                r"UPDATE session_inbox_messages
                  SET status = $4, consumed_at_ms = $5, consumed_turn_id = $6, iteration = $7
                  WHERE cluster_id = $1 AND message_id = $2 AND status = $3",
            )
            .bind(self.cluster_id())
            .bind(&msg.message_id)
            .bind(INBOX_STATUS_QUEUED)
            .bind(INBOX_STATUS_CONSUMED)
            .bind(now)
            .bind(turn_id)
            .bind(iteration)
            .execute(&mut *tx)
            .await?;
            msg.status = INBOX_STATUS_CONSUMED.to_string();
            msg.consumed_at_ms = Some(now);
            msg.consumed_turn_id = Some(turn_id.to_string());
            msg.iteration = Some(iteration);
            messages.push(msg);
        }
        let remaining: i64 = sqlx::query_scalar(
            r"SELECT COUNT(*) FROM session_inbox_messages
              WHERE cluster_id = $1 AND session_id = $2 AND status = $3",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(INBOX_STATUS_QUEUED)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(InboxDrainResult {
            messages,
            remaining_queued: remaining,
            limits: capacity.into(),
        })
    }

    /// List + summary for observability. Author: kejiqing
    pub async fn inbox_list(
        &self,
        session_id: &str,
        proj_id: i64,
        source_filter: Option<&str>,
        status_filter: Option<&str>,
        capacity: InboxCapacity,
        limit: i64,
    ) -> Result<(Vec<InboxMessageRow>, InboxSummary), InboxError> {
        self.assert_project_steerable(proj_id).await?;
        let limit = limit.clamp(1, 200);
        let source_filter = match source_filter {
            Some(s) if !s.trim().is_empty() => Some(validate_inbox_source(s)?),
            _ => None,
        };
        let status_filter = status_filter
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let rows = sqlx::query(
            r"SELECT message_id, session_id, proj_id, source, body, status, idempotency_key,
                     created_at_ms, consumed_at_ms, consumed_turn_id, iteration,
                     dropped_at_ms, dropped_reason,
                     from_address, in_reply_to, references_json
              FROM session_inbox_messages
              WHERE cluster_id = $1 AND session_id = $2 AND proj_id = $3
                AND ($4::text IS NULL OR source = $4)
                AND ($5::text IS NULL OR status = $5)
              ORDER BY created_at_ms DESC, message_id DESC
              LIMIT $6",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .bind(proj_id)
        .bind(source_filter)
        .bind(status_filter.as_deref())
        .bind(limit)
        .fetch_all(self.pg_pool())
        .await?;
        let messages: Vec<_> = rows.iter().map(row_from_sql).collect();
        let summary = self.inbox_summary(session_id, proj_id, capacity).await?;
        Ok((messages, summary))
    }

    pub async fn inbox_summary(
        &self,
        session_id: &str,
        _proj_id: i64,
        capacity: InboxCapacity,
    ) -> Result<InboxSummary, InboxError> {
        let counts = sqlx::query(
            r"SELECT status, source, COUNT(*)::bigint AS n,
                     MAX(created_at_ms) AS max_created,
                     MAX(consumed_at_ms) AS max_consumed
              FROM session_inbox_messages
              WHERE cluster_id = $1 AND session_id = $2
              GROUP BY status, source",
        )
        .bind(self.cluster_id())
        .bind(session_id)
        .fetch_all(self.pg_pool())
        .await?;

        let mut queued = 0i64;
        let mut consumed = 0i64;
        let mut dropped = 0i64;
        let mut queued_by_source = serde_json::Map::new();
        let mut last_received_at_ms: Option<i64> = None;
        let mut last_consumed_at_ms: Option<i64> = None;
        for r in counts {
            let status: String = r.get("status");
            let source: String = r.get("source");
            let n: i64 = r.get("n");
            let max_created: Option<i64> = r.get("max_created");
            let max_consumed: Option<i64> = r.get("max_consumed");
            match status.as_str() {
                INBOX_STATUS_QUEUED => {
                    queued += n;
                    let entry = queued_by_source
                        .entry(source)
                        .or_insert(serde_json::json!(0));
                    if let Some(v) = entry.as_i64() {
                        *entry = serde_json::json!(v + n);
                    }
                }
                INBOX_STATUS_CONSUMED => consumed += n,
                INBOX_STATUS_DROPPED => dropped += n,
                _ => {}
            }
            if let Some(t) = max_created {
                last_received_at_ms = Some(last_received_at_ms.map_or(t, |x| x.max(t)));
            }
            if let Some(t) = max_consumed {
                last_consumed_at_ms = Some(last_consumed_at_ms.map_or(t, |x| x.max(t)));
            }
        }
        Ok(InboxSummary {
            queued,
            consumed,
            dropped,
            queued_by_source,
            last_received_at_ms,
            last_consumed_at_ms,
            limits: capacity.into(),
        })
    }
}

async fn fetch_by_idempotency(
    tx: &mut Transaction<'_, Postgres>,
    cluster_id: &str,
    session_id: &str,
    key: &str,
) -> Result<Option<InboxMessageRow>, SqlxError> {
    let row = sqlx::query(
        r"SELECT message_id, session_id, proj_id, source, body, status, idempotency_key,
                 created_at_ms, consumed_at_ms, consumed_turn_id, iteration,
                 dropped_at_ms, dropped_reason,
                 from_address, in_reply_to, references_json
          FROM session_inbox_messages
          WHERE cluster_id = $1 AND session_id = $2 AND idempotency_key = $3
          LIMIT 1",
    )
    .bind(cluster_id)
    .bind(session_id)
    .bind(key)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.as_ref().map(row_from_sql))
}

/// Exposed for migration wiring. Author: kejiqing
#[allow(dead_code)]
pub fn inbox_migration_sql() -> &'static str {
    include_str!("../migrations/028_session_inbox_messages.sql")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox_capacity::InboxCapacity;
    use crate::session_db::{connect_gateway_test_db, ProjectConfigUpsert};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    fn ephemeral_proj() -> i64 {
        i64::try_from(uuid::Uuid::new_v4().as_u128() % 900_000_000).unwrap_or(42) + 1
    }

    async fn seed_steerable(db: &GatewaySessionDb, proj_id: i64) {
        let t = now_ms();
        db.upsert_project_config(ProjectConfigUpsert {
            proj_id,
            content_rev: "inbox-test",
            stable_content_rev: Some("inbox-test"),
            draft_open: false,
            updated_at_ms: t,
            rules_json: &json!([]),
            mcp_servers_json: &json!({}),
            skills_sources_json: &json!([]),
            skills_json: &json!([]),
            allowed_tools_json: &json!([]),
            claude_md: None,
            git_sync_json: &json!({}),
            solve_preflight_json: &json!({}),
            solve_orchestration_json: &json!({}),
            language_pipeline_json: &json!({}),
            extra_session_fields_json: &json!([]),
            prompt_limits_json: &json!({}),
            worker_profile_json: &json!({"mode": "strict"}),
            worker_env_json: &json!({}),
            kb_sources_json: &json!([]),
            project_code: "",
            project_description: "",
            max_iterations: None,
        })
        .await
        .unwrap();
        db.set_project_role(proj_id, PROJECT_ROLE_STEERABLE)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn inbox_enqueue_drain_fifo_and_overflow() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!(
                "skip inbox_enqueue_drain_fifo_and_overflow: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let proj_id = ephemeral_proj();
        seed_steerable(&db, proj_id).await;
        let sid = format!("inbox_{}", uuid::Uuid::new_v4().simple());
        let t = now_ms();
        db.insert_session(&sid, proj_id, "proj/sessions/x", t, None)
            .await
            .unwrap();

        let cap = InboxCapacity::from_parts(3, 1024, Some(2), Some(10_000)).unwrap();

        let other = ephemeral_proj();
        seed_steerable(&db, other).await;
        db.set_project_role(other, "normal").await.unwrap();
        let r = db
            .inbox_enqueue(&sid, other, "user", "x", None, cap, None, None, None)
            .await;
        assert!(matches!(r, Err(InboxError::Forbidden(_))));

        for i in 0..3 {
            db.inbox_enqueue(
                &sid,
                proj_id,
                "user",
                &format!("m{i}"),
                None,
                cap,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        }
        let from = format!("peer@1.{}", db.cluster_id());
        db.inbox_enqueue(
            &sid,
            proj_id,
            "mailbox",
            "m3",
            None,
            cap,
            Some(&from),
            None,
            None,
        )
        .await
        .unwrap();
        let summary = db.inbox_summary(&sid, proj_id, cap).await.unwrap();
        assert_eq!(summary.queued, 3);
        assert!(summary.dropped >= 1);

        let tid = format!("T_{}", uuid::Uuid::new_v4().simple());
        db.insert_turn(&tid, &sid, proj_id, "running", t, Some("p"), None, None)
            .await
            .unwrap();

        let d1 = db.inbox_drain(&sid, proj_id, &tid, 1, cap).await.unwrap();
        assert_eq!(d1.messages.len(), 2);
        assert_eq!(d1.remaining_queued, 1);
        assert_eq!(d1.messages[0].status, INBOX_STATUS_CONSUMED);

        let d2 = db.inbox_drain(&sid, proj_id, &tid, 2, cap).await.unwrap();
        assert_eq!(d2.messages.len(), 1);
        assert_eq!(d2.remaining_queued, 0);

        let a = db
            .inbox_enqueue(
                &sid,
                proj_id,
                "user",
                "same",
                Some("k1"),
                cap,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let b = db
            .inbox_enqueue(
                &sid,
                proj_id,
                "user",
                "same",
                Some("k1"),
                cap,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert!(b.idempotent_hit);
        assert_eq!(a.message_id, b.message_id);

        let big = "x".repeat(cap.body_max_bytes + 1);
        let err = db
            .inbox_enqueue(&sid, proj_id, "user", &big, None, cap, None, None, None)
            .await;
        assert!(matches!(err, Err(InboxError::BadRequest(_))));

        let _ = db.delete_project_config(proj_id).await;
        let _ = db.delete_project_config(other).await;
    }

    #[tokio::test]
    async fn inbox_drain_byte_cap_stops_batch() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!("skip inbox_drain_byte_cap_stops_batch: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let proj_id = ephemeral_proj();
        seed_steerable(&db, proj_id).await;
        let sid = format!("inbox_b_{}", uuid::Uuid::new_v4().simple());
        let t = now_ms();
        db.insert_session(&sid, proj_id, "proj/sessions/y", t, None)
            .await
            .unwrap();
        let cap = InboxCapacity::from_parts(32, 100, Some(8), Some(50)).unwrap();
        db.inbox_enqueue(
            &sid,
            proj_id,
            "user",
            &"a".repeat(30),
            None,
            cap,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        db.inbox_enqueue(
            &sid,
            proj_id,
            "user",
            &"b".repeat(30),
            None,
            cap,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let tid = format!("T_{}", uuid::Uuid::new_v4().simple());
        db.insert_turn(&tid, &sid, proj_id, "running", t, None, None, None)
            .await
            .unwrap();
        let d = db.inbox_drain(&sid, proj_id, &tid, 1, cap).await.unwrap();
        assert_eq!(d.messages.len(), 1);
        assert_eq!(d.remaining_queued, 1);
        let _ = db.delete_project_config(proj_id).await;
    }

    #[tokio::test]
    async fn inbox_mailbox_threading_fields_roundtrip() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!(
                "skip inbox_mailbox_threading_fields_roundtrip: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let proj_id = ephemeral_proj();
        seed_steerable(&db, proj_id).await;
        let sid = format!("inbox_t_{}", uuid::Uuid::new_v4().simple());
        let t = now_ms();
        db.insert_session(&sid, proj_id, "proj/sessions/t", t, None)
            .await
            .unwrap();
        let cap = InboxCapacity::from_parts(8, 1024, Some(4), Some(4096)).unwrap();
        let missing = db
            .inbox_enqueue(&sid, proj_id, "mailbox", "hi", None, cap, None, None, None)
            .await;
        assert!(matches!(missing, Err(InboxError::BadRequest(_))));

        let from = format!("{sid}@{proj_id}.{}", db.cluster_id());
        let refs = vec!["msg_a".into(), "msg_b".into()];
        db.inbox_enqueue(
            &sid,
            proj_id,
            "mailbox",
            "reply body",
            None,
            cap,
            Some(&from),
            Some("msg_a"),
            Some(refs.as_slice()),
        )
        .await
        .unwrap();
        let tid = format!("T_{}", uuid::Uuid::new_v4().simple());
        db.insert_turn(&tid, &sid, proj_id, "running", t, None, None, None)
            .await
            .unwrap();
        let d = db.inbox_drain(&sid, proj_id, &tid, 0, cap).await.unwrap();
        assert_eq!(d.messages.len(), 1);
        let m = &d.messages[0];
        assert_eq!(m.from_address.as_deref(), Some(from.as_str()));
        assert_eq!(m.in_reply_to.as_deref(), Some("msg_a"));
        assert_eq!(m.references, refs);
        let _ = db.delete_project_config(proj_id).await;
    }
}
