//! Gateway solve turn ids (`T_<32 hex lowercase>`). Author: kejiqing

use uuid::Uuid;

pub const TURN_ID_PREFIX: &str = "T_";

/// Mint a new turn id: `T_` + UUID v4 simple (32 lowercase hex, no hyphens).
#[must_use]
pub fn mint_turn_id() -> String {
    format!("{TURN_ID_PREFIX}{}", Uuid::new_v4().simple())
}

/// Returns true when `s` matches `^T_[0-9a-f]{32}$`.
#[must_use]
pub fn validate_turn_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(TURN_ID_PREFIX) else {
        return false;
    };
    rest.len() == 32
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_and_validate_round_trip() {
        let id = mint_turn_id();
        assert!(validate_turn_id(&id));
        assert!(id.starts_with(TURN_ID_PREFIX));
    }

    #[test]
    fn rejects_invalid_shapes() {
        assert!(!validate_turn_id("turn-1"));
        assert!(!validate_turn_id("T_ABCD"));
        assert!(!validate_turn_id("T_550e8400-e29b-41d4-a716-446655440000"));
    }
}
