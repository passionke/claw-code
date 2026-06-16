//! `extraSession.bizdate` (yyyyMMdd) → solve-time date env + system prompt date. Author: kejiqing

use runtime::RuntimeConfig;
use serde_json::Value;

/// Compact business date in `extraSession` (`yyyyMMdd`).
pub const EXTRA_SESSION_BIZDATE_KEY: &str = "bizdate";

/// ISO calendar date for SQL / prompts (`yyyy-MM-dd`).
pub const ENV_CURRENT_DATE: &str = "CURRENT_DATE";

/// Compact business date echo (`yyyyMMdd`).
pub const ENV_BIZDATE: &str = "BIZDATE";

/// Placeholders supported in project `.claw.json` `env` string values.
pub const PLACEHOLDER_BIZDATE: &str = "{{bizdate}}";
pub const PLACEHOLDER_CURRENT_DATE: &str = "{{current_date}}";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BizDate {
    pub compact: String,
    pub iso: String,
}

/// Parse `extraSession.bizdate` when it is a valid `yyyyMMdd` calendar date.
#[must_use]
pub fn parse_extra_session_bizdate(extra_session: Option<&Value>) -> Option<BizDate> {
    let raw = extra_session_bizdate_raw(extra_session)?;
    parse_yyyy_mm_dd(raw).map(|(year, month, day)| BizDate {
        compact: raw.to_string(),
        iso: format!("{year:04}-{month:02}-{day:02}"),
    })
}

fn extra_session_bizdate_raw(extra_session: Option<&Value>) -> Option<&str> {
    let obj = extra_session?.as_object()?;
    let raw = obj.get(EXTRA_SESSION_BIZDATE_KEY)?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw)
}

#[must_use]
pub fn parse_yyyy_mm_dd(raw: &str) -> Option<(i32, u32, u32)> {
    if raw.len() != 8 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = raw.get(..4)?.parse().ok()?;
    let month: u32 = raw.get(4..6)?.parse().ok()?;
    let day: u32 = raw.get(6..8)?.parse().ok()?;
    if !(1..=12).contains(&month) || !is_valid_civil_day(year, month, day) {
        return None;
    }
    Some((year, month, day))
}

fn is_valid_civil_day(year: i32, month: u32, day: u32) -> bool {
    if day == 0 {
        return false;
    }
    let max_day = days_in_month(year, month);
    day <= max_day
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Substitute bizdate placeholders in one env template string.
#[must_use]
pub fn substitute_bizdate_placeholders(template: &str, bizdate: &BizDate) -> String {
    template
        .replace(PLACEHOLDER_BIZDATE, &bizdate.compact)
        .replace(PLACEHOLDER_CURRENT_DATE, &bizdate.iso)
}

/// Apply project `env` defaults, then API `bizdate` wins for date injection.
pub fn apply_solve_env_from_config_and_extra(
    config: &RuntimeConfig,
    extra_session: Option<&Value>,
) {
    let bizdate = parse_extra_session_bizdate(extra_session);
    apply_config_env_with_bizdate_templates(config, bizdate.as_ref());
    if let Some(bd) = bizdate {
        std::env::set_var(ENV_CURRENT_DATE, &bd.iso);
        std::env::set_var(ENV_BIZDATE, &bd.compact);
    }
}

fn apply_config_env_with_bizdate_templates(config: &RuntimeConfig, bizdate: Option<&BizDate>) {
    let Some(value) = config.get("env") else {
        return;
    };
    let Some(map) = value.as_object() else {
        return;
    };
    for (key, entry) in map {
        if std::env::var_os(key).is_some() {
            continue;
        }
        let Some(string) = entry.as_str() else {
            continue;
        };
        let resolved = bizdate
            .map(|bd| substitute_bizdate_placeholders(string, bd))
            .unwrap_or_else(|| string.to_string());
        std::env::set_var(key, resolved);
    }
}

/// System prompt / environment context date for one solve turn.
#[must_use]
pub fn resolve_system_date_for_solve(
    extra_session: Option<&Value>,
    fallback: impl FnOnce() -> String,
) -> String {
    parse_extra_session_bizdate(extra_session)
        .map(|bd| bd.iso)
        .unwrap_or_else(fallback)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use runtime::ConfigLoader;

    use super::*;
    use serde_json::json;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().expect("env test lock")
    }

    fn test_config_from_json(json: &str) -> RuntimeConfig {
        let root = std::env::temp_dir().join(format!(
            "claw-bizdate-cfg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let cwd = root.join("project");
        let home = root.join("home").join(".claw");
        fs::create_dir_all(&home).expect("home config dir");
        fs::create_dir_all(&cwd).expect("project dir");
        fs::write(cwd.join(".claw.json"), json).expect("write project config");
        let config = ConfigLoader::new(&cwd, &home).load().expect("load config");
        let _ = fs::remove_dir_all(&root);
        config
    }

    #[test]
    fn parse_accepts_valid_yyyy_mm_dd() {
        let bd = parse_yyyy_mm_dd("20250615").expect("valid");
        assert_eq!(bd, (2025, 6, 15));
        let parsed = parse_extra_session_bizdate(Some(&json!({"bizdate": "20250615"})))
            .expect("extra session");
        assert_eq!(parsed.compact, "20250615");
        assert_eq!(parsed.iso, "2025-06-15");
    }

    #[test]
    fn parse_rejects_bad_length_and_invalid_calendar() {
        assert!(parse_yyyy_mm_dd("2025061").is_none());
        assert!(parse_yyyy_mm_dd("20250230").is_none());
        assert!(parse_yyyy_mm_dd("20251301").is_none());
        assert!(parse_extra_session_bizdate(Some(&json!({"bizdate": "20250230"}))).is_none());
        assert!(parse_extra_session_bizdate(Some(&json!({"bizdate": ""}))).is_none());
        assert!(parse_extra_session_bizdate(None).is_none());
    }

    #[test]
    fn substitute_placeholders_in_env_template() {
        let bd = BizDate {
            compact: "20240102".into(),
            iso: "2024-01-02".into(),
        };
        let out = substitute_bizdate_placeholders("day={{current_date}} compact={{bizdate}}", &bd);
        assert_eq!(out, "day=2024-01-02 compact=20240102");
    }

    #[test]
    fn apply_solve_env_sets_current_date_and_bizdate_from_api() {
        let _guard = env_lock();
        std::env::remove_var(ENV_CURRENT_DATE);
        std::env::remove_var(ENV_BIZDATE);

        let config = test_config_from_json(
            r#"{"env":{"REPORT_DAY":"{{current_date}}","BIZ":"{{bizdate}}"}}"#,
        );
        apply_solve_env_from_config_and_extra(&config, Some(&json!({"bizdate": "20250301"})));

        assert_eq!(std::env::var(ENV_CURRENT_DATE).as_deref(), Ok("2025-03-01"));
        assert_eq!(std::env::var(ENV_BIZDATE).as_deref(), Ok("20250301"));
        assert_eq!(std::env::var("REPORT_DAY").as_deref(), Ok("2025-03-01"));
        assert_eq!(std::env::var("BIZ").as_deref(), Ok("20250301"));

        std::env::remove_var(ENV_CURRENT_DATE);
        std::env::remove_var(ENV_BIZDATE);
        std::env::remove_var("REPORT_DAY");
        std::env::remove_var("BIZ");
    }

    #[test]
    fn apply_solve_env_without_bizdate_uses_static_env_values() {
        let _guard = env_lock();
        std::env::remove_var("STATIC_ONLY");
        let config = test_config_from_json(r#"{"env":{"STATIC_ONLY":"fixed"}}"#);
        apply_solve_env_from_config_and_extra(&config, None);
        assert_eq!(std::env::var("STATIC_ONLY").as_deref(), Ok("fixed"));
        std::env::remove_var("STATIC_ONLY");
    }

    #[test]
    fn resolve_system_date_prefers_bizdate() {
        let iso = resolve_system_date_for_solve(Some(&json!({"bizdate": "20241231"})), || {
            "2099-01-01".to_string()
        });
        assert_eq!(iso, "2024-12-31");
        let fallback = resolve_system_date_for_solve(None, || "2099-01-01".to_string());
        assert_eq!(fallback, "2099-01-01");
    }

    #[test]
    fn invalid_bizdate_does_not_override_existing_env() {
        let _guard = env_lock();
        std::env::set_var(ENV_CURRENT_DATE, "keep-me");
        let config = RuntimeConfig::empty();
        apply_solve_env_from_config_and_extra(&config, Some(&json!({"bizdate": "not-a-date"})));
        assert_eq!(std::env::var(ENV_CURRENT_DATE).as_deref(), Ok("keep-me"));
        std::env::remove_var(ENV_CURRENT_DATE);
    }
}
