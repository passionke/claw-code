//! Convert runtime session blocks to API input blocks (loads image bytes). Author: kejiqing

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use runtime::{ContentBlock, ConversationMessage, MessageRole};
use serde_json::json;

use crate::types::{InputContentBlock, InputMessage, ToolResultContentBlock};

/// Load an image from `cwd`/`path` into an API image content block.
#[must_use]
pub fn image_input_from_path(path: &str, mime: &str, cwd: &Path) -> InputContentBlock {
    let full = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    match std::fs::read(&full) {
        Ok(bytes) => InputContentBlock::Image {
            media_type: mime.to_string(),
            data_base64: STANDARD.encode(bytes),
        },
        Err(e) => InputContentBlock::Text {
            text: format!("[image load failed path={path}: {e}]"),
        },
    }
}

/// Convert one runtime content block; images are read relative to `cwd`.
#[must_use]
pub fn runtime_block_to_input(block: &ContentBlock, cwd: &Path) -> InputContentBlock {
    match block {
        ContentBlock::Text { text } => InputContentBlock::Text { text: text.clone() },
        ContentBlock::ReasoningContent { text } => {
            InputContentBlock::ReasoningContent { text: text.clone() }
        }
        ContentBlock::Image { path, mime, .. } => image_input_from_path(path, mime, cwd),
        ContentBlock::ToolUse { id, name, input } => InputContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: serde_json::from_str(input).unwrap_or_else(|_| json!({ "raw": input })),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            output,
            is_error,
            ..
        } => InputContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: vec![ToolResultContentBlock::Text {
                text: output.clone(),
            }],
            is_error: *is_error,
        },
    }
}

/// Convert session messages for an LLM request. Author: kejiqing
#[must_use]
pub fn convert_runtime_messages(messages: &[ConversationMessage], cwd: &Path) -> Vec<InputMessage> {
    messages
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                MessageRole::System | MessageRole::User | MessageRole::Tool => "user",
                MessageRole::Assistant => "assistant",
            };
            let content = message
                .blocks
                .iter()
                .map(|block| runtime_block_to_input(block, cwd))
                .collect::<Vec<_>>();
            (!content.is_empty()).then(|| InputMessage {
                role: role.to_string(),
                content,
            })
        })
        .collect()
}

/// Same as [`convert_runtime_messages`] but maps Tool role to `user` content blocks
/// (gateway-solve-turn historical mapping). Author: kejiqing
#[must_use]
pub fn convert_runtime_messages_gateway(
    messages: &[ConversationMessage],
    cwd: &Path,
) -> Vec<InputMessage> {
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::System => "system",
                MessageRole::User | MessageRole::Tool => "user",
                MessageRole::Assistant => "assistant",
            }
            .to_string();
            let content = message
                .blocks
                .iter()
                .map(|block| runtime_block_to_input(block, cwd))
                .collect::<Vec<_>>();
            InputMessage { role, content }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn image_input_from_path_encodes_base64() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tiny.png");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(&[0x89, 0x50, 0x4e, 0x47]).expect("write");
        let block = image_input_from_path("tiny.png", "image/png", dir.path());
        match block {
            InputContentBlock::Image {
                media_type,
                data_base64,
            } => {
                assert_eq!(media_type, "image/png");
                assert_eq!(data_base64, STANDARD.encode([0x89, 0x50, 0x4e, 0x47]));
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn runtime_image_json_has_no_base64() {
        let block = ContentBlock::Image {
            path: "uploads/a.png".into(),
            mime: "image/png".into(),
            name: Some("a.png".into()),
        };
        let json = block.to_json();
        let obj = json.as_object().expect("object");
        assert_eq!(obj.get("type").and_then(|v| v.as_str()), Some("image"));
        assert_eq!(
            obj.get("path").and_then(|v| v.as_str()),
            Some("uploads/a.png")
        );
        assert!(obj.get("data_base64").is_none());
        assert!(obj.get("data").is_none());
    }
}
