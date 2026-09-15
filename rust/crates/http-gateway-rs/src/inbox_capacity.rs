//! Session inbox capacity knobs (gateway parameter space). Author: kejiqing

use serde::Serialize;

/// Default max queued messages per session.
pub const DEFAULT_INBOX_QUEUE_LIMIT: u32 = 32;
/// Default max UTF-8 bytes per message body.
pub const DEFAULT_INBOX_BODY_MAX_BYTES: usize = 32 * 1024;
/// Env: queue depth.
pub const ENV_INBOX_QUEUE_LIMIT: &str = "CLAW_INBOX_QUEUE_LIMIT";
/// Env: single body max bytes.
pub const ENV_INBOX_BODY_MAX_BYTES: &str = "CLAW_INBOX_BODY_MAX_BYTES";
/// Env: explicit drain batch (optional; else `queue_limit / 4`).
pub const ENV_INBOX_DRAIN_BATCH: &str = "CLAW_INBOX_DRAIN_BATCH";
/// Env: explicit drain byte cap (optional; else `2 * body_max`).
pub const ENV_INBOX_DRAIN_MAX_BYTES: &str = "CLAW_INBOX_DRAIN_MAX_BYTES";

/// Effective inbox capacity for one gateway process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxCapacity {
    pub queue_limit: u32,
    pub body_max_bytes: usize,
    pub drain_batch: u32,
    pub drain_max_bytes: usize,
}

impl Default for InboxCapacity {
    fn default() -> Self {
        Self::from_parts(
            DEFAULT_INBOX_QUEUE_LIMIT,
            DEFAULT_INBOX_BODY_MAX_BYTES,
            None,
            None,
        )
        .expect("default inbox capacity")
    }
}

impl InboxCapacity {
    /// Build from explicit parts; `drain_*` None → derive from queue/body.
    /// Author: kejiqing
    pub fn from_parts(
        queue_limit: u32,
        body_max_bytes: usize,
        drain_batch: Option<u32>,
        drain_max_bytes: Option<usize>,
    ) -> Result<Self, String> {
        if queue_limit == 0 {
            return Err(format!("{ENV_INBOX_QUEUE_LIMIT} must be >= 1"));
        }
        if body_max_bytes == 0 {
            return Err(format!("{ENV_INBOX_BODY_MAX_BYTES} must be >= 1"));
        }
        let derived_batch = (queue_limit / 4).max(1);
        let mut batch = drain_batch.unwrap_or(derived_batch);
        if batch == 0 {
            return Err(format!("{ENV_INBOX_DRAIN_BATCH} must be >= 1"));
        }
        if batch > queue_limit {
            batch = queue_limit;
        }
        let derived_bytes = body_max_bytes.saturating_mul(2).max(1);
        let max_bytes = match drain_max_bytes {
            Some(0) => return Err(format!("{ENV_INBOX_DRAIN_MAX_BYTES} must be >= 1")),
            Some(n) => n,
            None => derived_bytes,
        };
        Ok(Self {
            queue_limit,
            body_max_bytes,
            drain_batch: batch,
            drain_max_bytes: max_bytes,
        })
    }

    /// Parse gateway env; missing keys use defaults. Invalid → Err (caller should refuse start).
    /// Author: kejiqing
    pub fn from_env() -> Result<Self, String> {
        let queue_limit = parse_u32_env(ENV_INBOX_QUEUE_LIMIT, DEFAULT_INBOX_QUEUE_LIMIT)?;
        let body_max_bytes =
            parse_usize_env(ENV_INBOX_BODY_MAX_BYTES, DEFAULT_INBOX_BODY_MAX_BYTES)?;
        let drain_batch = parse_optional_u32_env(ENV_INBOX_DRAIN_BATCH)?;
        let drain_max_bytes = parse_optional_usize_env(ENV_INBOX_DRAIN_MAX_BYTES)?;
        Self::from_parts(queue_limit, body_max_bytes, drain_batch, drain_max_bytes)
    }

    /// How many leading messages (FIFO) fit under drain batch + byte caps.
    /// Single message at/under body_max always included even if alone exceeds drain_max_bytes.
    /// Author: kejiqing
    #[must_use]
    pub fn plan_drain(&self, body_lens: &[usize]) -> usize {
        if body_lens.is_empty() {
            return 0;
        }
        let mut count = 0u32;
        let mut bytes = 0usize;
        for (i, &len) in body_lens.iter().enumerate() {
            if count >= self.drain_batch {
                break;
            }
            if count > 0 && bytes.saturating_add(len) > self.drain_max_bytes {
                break;
            }
            // First message: always take (body already gated at enqueue).
            bytes = bytes.saturating_add(len);
            count += 1;
            let _ = i;
        }
        count as usize
    }
}

fn parse_u32_env(key: &str, default: u32) -> Result<u32, String> {
    match std::env::var(key) {
        Ok(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                return Ok(default);
            }
            t.parse::<u32>()
                .map_err(|_| format!("{key}={raw:?} is not a valid u32 >= 1"))
                .and_then(|n| {
                    if n == 0 {
                        Err(format!("{key} must be >= 1"))
                    } else {
                        Ok(n)
                    }
                })
        }
        Err(_) => Ok(default),
    }
}

fn parse_usize_env(key: &str, default: usize) -> Result<usize, String> {
    match std::env::var(key) {
        Ok(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                return Ok(default);
            }
            t.parse::<usize>()
                .map_err(|_| format!("{key}={raw:?} is not a valid usize >= 1"))
                .and_then(|n| {
                    if n == 0 {
                        Err(format!("{key} must be >= 1"))
                    } else {
                        Ok(n)
                    }
                })
        }
        Err(_) => Ok(default),
    }
}

fn parse_optional_u32_env(key: &str) -> Result<Option<u32>, String> {
    match std::env::var(key) {
        Ok(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                return Ok(None);
            }
            let n = t
                .parse::<u32>()
                .map_err(|_| format!("{key}={raw:?} is not a valid u32 >= 1"))?;
            if n == 0 {
                return Err(format!("{key} must be >= 1"));
            }
            Ok(Some(n))
        }
        Err(_) => Ok(None),
    }
}

fn parse_optional_usize_env(key: &str) -> Result<Option<usize>, String> {
    match std::env::var(key) {
        Ok(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                return Ok(None);
            }
            let n = t
                .parse::<usize>()
                .map_err(|_| format!("{key}={raw:?} is not a valid usize >= 1"))?;
            if n == 0 {
                return Err(format!("{key} must be >= 1"));
            }
            Ok(Some(n))
        }
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_inbox_env() {
        for k in [
            ENV_INBOX_QUEUE_LIMIT,
            ENV_INBOX_BODY_MAX_BYTES,
            ENV_INBOX_DRAIN_BATCH,
            ENV_INBOX_DRAIN_MAX_BYTES,
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn defaults_match_plan() {
        let c = InboxCapacity::default();
        assert_eq!(c.queue_limit, 32);
        assert_eq!(c.body_max_bytes, 32 * 1024);
        assert_eq!(c.drain_batch, 8);
        assert_eq!(c.drain_max_bytes, 64 * 1024);
    }

    #[test]
    fn queue_limit_16_derives_drain_batch_4() {
        let c = InboxCapacity::from_parts(16, 32 * 1024, None, None).unwrap();
        assert_eq!(c.drain_batch, 4);
        assert_eq!(c.drain_max_bytes, 64 * 1024);
    }

    #[test]
    fn explicit_drain_batch_overrides_and_clamps() {
        let c = InboxCapacity::from_parts(32, 1024, Some(2), None).unwrap();
        assert_eq!(c.drain_batch, 2);
        let clamped = InboxCapacity::from_parts(8, 1024, Some(100), None).unwrap();
        assert_eq!(clamped.drain_batch, 8);
    }

    #[test]
    fn rejects_zero_parts() {
        assert!(InboxCapacity::from_parts(0, 100, None, None).is_err());
        assert!(InboxCapacity::from_parts(8, 0, None, None).is_err());
        assert!(InboxCapacity::from_parts(8, 100, Some(0), None).is_err());
        assert!(InboxCapacity::from_parts(8, 100, None, Some(0)).is_err());
    }

    #[test]
    fn from_env_defaults_and_override() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_inbox_env();
        assert_eq!(InboxCapacity::from_env().unwrap(), InboxCapacity::default());
        std::env::set_var(ENV_INBOX_QUEUE_LIMIT, "16");
        let c = InboxCapacity::from_env().unwrap();
        assert_eq!(c.queue_limit, 16);
        assert_eq!(c.drain_batch, 4);
        std::env::set_var(ENV_INBOX_DRAIN_BATCH, "2");
        let c2 = InboxCapacity::from_env().unwrap();
        assert_eq!(c2.drain_batch, 2);
        std::env::set_var(ENV_INBOX_QUEUE_LIMIT, "0");
        assert!(InboxCapacity::from_env().is_err());
        clear_inbox_env();
    }

    #[test]
    fn plan_drain_fifo_batch_and_bytes() {
        let c = InboxCapacity::from_parts(32, 1024, Some(3), Some(100)).unwrap();
        assert_eq!(c.plan_drain(&[]), 0);
        assert_eq!(c.plan_drain(&[10, 10, 10, 10]), 3);
        // first alone ok even if large
        assert_eq!(c.plan_drain(&[200]), 1);
        // after first, stop when next would exceed
        assert_eq!(c.plan_drain(&[60, 50, 10]), 1);
        assert_eq!(c.plan_drain(&[40, 40, 40]), 2);
    }
}
