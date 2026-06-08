//! Unified message model (design §2.1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// 内核**内部**的统一内容表示，也是 checkpoint/序列化用的格式。
/// 它**不是**任何 provider 的线上格式——OpenAI function call / Anthropic
/// tool_use / DeepSeek 等的相互翻译只发生在各自 `src/providers/` 实现里（design §2.2）。
/// 不要把这里的 serde 输出直接当作发给模型 API 的载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: vec![ContentBlock::Text(text.into())],
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text(text.into())],
        }
    }

    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content: blocks,
        }
    }

    pub fn tool(block: ContentBlock) -> Self {
        Self {
            role: Role::Tool,
            content: vec![block],
        }
    }

    /// 多个文本块按换行拼接，避免相邻段落首尾粘连。
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_json<T: Serialize + for<'de> Deserialize<'de>>(value: &T) -> T {
        let json = serde_json::to_string(value).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    #[test]
    fn role_round_trip() {
        for role in [
            Role::System,
            Role::User,
            Role::Assistant,
            Role::Tool,
        ] {
            assert_eq!(role, round_trip_json(&role));
        }
    }

    #[test]
    fn content_block_round_trip() {
        let blocks = [
            ContentBlock::Text("hello".into()),
            ContentBlock::ToolCall {
                id: "call_abc".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+2" }),
            },
            ContentBlock::ToolResult {
                id: "call_abc".into(),
                content: "3".into(),
                is_error: false,
            },
            ContentBlock::ToolResult {
                id: "call_xyz".into(),
                content: "unknown tool".into(),
                is_error: true,
            },
        ];

        for block in &blocks {
            assert_eq!(block, &round_trip_json(block));
        }
    }

    #[test]
    fn message_round_trip_mixed_assistant() {
        let message = Message::assistant(vec![
            ContentBlock::Text("我来算一下。".into()),
            ContentBlock::ToolCall {
                id: "call_abc".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+2" }),
            },
        ]);

        assert_eq!(message, round_trip_json(&message));
    }

    #[test]
    fn text_joins_multiple_blocks_with_newline() {
        let message = Message::assistant(vec![
            ContentBlock::Text("first".into()),
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "noop".into(),
                args: serde_json::json!({}),
            },
            ContentBlock::Text("second".into()),
        ]);
        assert_eq!(message.text(), "first\nsecond");
    }

    #[test]
    fn convenience_constructors_and_queries() {
        let user = Message::user("hi");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.text(), "hi");
        assert!(!user.has_tool_calls());

        let assistant = Message::assistant(vec![
            ContentBlock::Text("preamble".into()),
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({}),
            },
        ]);
        assert_eq!(assistant.text(), "preamble");
        assert!(assistant.has_tool_calls());

        let tool = Message::tool(ContentBlock::ToolResult {
            id: "call_1".into(),
            content: "42".into(),
            is_error: false,
        });
        assert_eq!(tool.role, Role::Tool);
        assert_eq!(tool.text(), "");
        assert!(!tool.has_tool_calls());
    }
}
