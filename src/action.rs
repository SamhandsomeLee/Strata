//! ActionBackend trait and action types (design §2.3).
//!
//! [`ActionBackend`] turns an assistant [`Message`] into executable [`Action`]s.
//! MVP uses [`JsonToolCall`] (C11 extracts `ContentBlock::ToolCall`); future
//! backends such as CodeAction can implement the same trait without changing the loop.

use crate::message::Message;

/// Normalized tool invocation for the agentic loop.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

impl Action {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
        }
    }
}

/// Parses model intent from an assistant message. An empty return means no tool calls
/// (loop normal termination per design §3).
pub trait ActionBackend {
    fn parse_actions(&self, assistant_msg: &Message) -> Vec<Action>;
}

/// JSON tool-call backend. C11 will extract `ContentBlock::ToolCall` blocks.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JsonToolCall;

impl ActionBackend for JsonToolCall {
    fn parse_actions(&self, _assistant_msg: &Message) -> Vec<Action> {
        // C11: iterate assistant_msg.content and map ToolCall → Action
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, Role};

    #[test]
    fn action_new_and_equality() {
        let a = Action::new("call_1", "calculator", serde_json::json!({ "x": 1 }));
        let b = Action::new("call_1", "calculator", serde_json::json!({ "x": 1 }));
        assert_eq!(a, b);
        assert_eq!(a.id, "call_1");
        assert_eq!(a.name, "calculator");
    }

    #[test]
    fn json_tool_call_is_object_safe_and_returns_empty() {
        let backend: &dyn ActionBackend = &JsonToolCall;
        let msg = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({}),
            }],
        };
        assert!(backend.parse_actions(&msg).is_empty());

        let default_backend = <JsonToolCall as Default>::default();
        assert_eq!(default_backend, JsonToolCall);
        assert!(default_backend.parse_actions(&msg).is_empty());
    }
}
