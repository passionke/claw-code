//! Parent turn output acc: child specialist reports adopt as parent output. Author: kejiqing

use std::sync::Mutex;

#[derive(Debug, Default)]
struct Segment {
    child: String,
    bridge_after: String,
}

#[derive(Debug, Default)]
struct ParentOutputAcc {
    segments: Vec<Segment>,
    active: bool,
}

static PARENT_OUTPUT: Mutex<ParentOutputAcc> = Mutex::new(ParentOutputAcc {
    segments: Vec::new(),
    active: false,
});

fn with_acc<R>(f: impl FnOnce(&mut ParentOutputAcc) -> R) -> R {
    let mut guard = PARENT_OUTPUT.lock().expect("parent output acc lock");
    f(&mut guard)
}

/// Reset at turn start (`gateway-solve-once` one process = one turn).
pub fn reset_parent_output_acc() {
    with_acc(|acc| {
        acc.segments.clear();
        acc.active = false;
    });
}

/// Adopt specialist terminal report as a parent output segment (identity, not copy-for-LLM).
pub fn adopt_child_output(text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    with_acc(|acc| {
        acc.active = true;
        acc.segments.push(Segment {
            child: trimmed.to_string(),
            bridge_after: String::new(),
        });
    });
}

/// Mixed-intent bridge text between serial delegates (additive, streamed separately).
pub fn append_parent_bridge(text: &str) {
    if text.is_empty() {
        return;
    }
    with_acc(|acc| {
        if !acc.active {
            return;
        }
        if let Some(last) = acc.segments.last_mut() {
            last.bridge_after.push_str(text);
        }
    });
}

#[must_use]
pub fn had_adopted_child_output() -> bool {
    with_acc(|acc| acc.active)
}

#[must_use]
pub fn take_parent_output() -> Option<String> {
    with_acc(|acc| {
        if !acc.active || acc.segments.is_empty() {
            return None;
        }
        let mut out = String::new();
        for seg in &acc.segments {
            if !seg.child.is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&seg.child);
            }
            if !seg.bridge_after.is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(seg.bridge_after.trim());
            }
        }
        acc.segments.clear();
        acc.active = false;
        let trimmed = out.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_child_adopt() {
        reset_parent_output_acc();
        adopt_child_output("kb answer");
        assert_eq!(take_parent_output().as_deref(), Some("kb answer"));
    }

    #[test]
    fn serial_with_bridge() {
        reset_parent_output_acc();
        adopt_child_output("kb");
        append_parent_bridge("关于问数：");
        adopt_child_output("ops");
        assert_eq!(
            take_parent_output().as_deref(),
            Some("kb\n\n关于问数：\n\nops")
        );
    }
}
