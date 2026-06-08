//! ActionBackend trait and action types (design §2.3).
//!
//! [`ActionBackend`] turns an assistant [`Message`] into executable [`Action`]s.
//! MVP uses [`JsonToolCall`] (extracts `ContentBlock::ToolCall`); future
//! backends such as CodeAction can implement the same trait without changing the loop.

use crate::message::{ContentBlock, Message, Role};

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

/// JSON tool-call backend: extracts `ContentBlock::ToolCall` blocks in order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JsonToolCall;

impl ActionBackend for JsonToolCall {
    fn parse_actions(&self, assistant_msg: &Message) -> Vec<Action> {
        if assistant_msg.role != Role::Assistant {
            return Vec::new();
        }
        assistant_msg
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall { id, name, args } => Some(Action {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                }),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_new_and_equality() {
        let a = Action::new("call_1", "calculator", serde_json::json!({ "x": 1 }));
        let b = Action::new("call_1", "calculator", serde_json::json!({ "x": 1 }));
        assert_eq!(a, b);
        assert_eq!(a.id, "call_1");
        assert_eq!(a.name, "calculator");
    }

    #[test]
    fn extracts_single_tool_call() {
        let backend = JsonToolCall;
        let msg = Message::assistant(vec![ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "calculator".into(),
            args: serde_json::json!({ "expression": "1+2" }),
        }]);
        let actions = backend.parse_actions(&msg);
        assert_eq!(
            actions,
            vec![Action::new(
                "call_1",
                "calculator",
                serde_json::json!({ "expression": "1+2" })
            )]
        );
    }

    #[test]
    fn text_only_returns_empty() {
        let backend = JsonToolCall;
        let msg = Message::assistant(vec![ContentBlock::Text("done".into())]);
        assert!(backend.parse_actions(&msg).is_empty());
    }

    #[test]
    fn mixed_text_and_tool_call_extracts_tool_call() {
        let backend = JsonToolCall;
        let msg = Message::assistant(vec![
            ContentBlock::Text("let me calculate".into()),
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({}),
            },
        ]);
        assert_eq!(backend.parse_actions(&msg).len(), 1);
    }

    #[test]
    fn multiple_tool_calls_preserve_order() {
        let backend = JsonToolCall;
        let msg = Message::assistant(vec![
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "a".into(),
                args: serde_json::json!({}),
            },
            ContentBlock::ToolCall {
                id: "call_2".into(),
                name: "b".into(),
                args: serde_json::json!({ "x": 1 }),
            },
        ]);
        let actions = backend.parse_actions(&msg);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].name, "a");
        assert_eq!(actions[1].name, "b");
    }

    #[test]
    fn non_assistant_returns_empty() {
        let backend = JsonToolCall;
        let msg = Message {
            role: Role::User,
            content: vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({}),
            }],
        };
        assert!(backend.parse_actions(&msg).is_empty());
    }

    #[test]
    fn empty_content_returns_empty() {
        let backend = JsonToolCall;
        let msg = Message::assistant(vec![]);
        assert!(backend.parse_actions(&msg).is_empty());
    }

    #[test]
    fn json_tool_call_is_object_safe() {
        let backend: &dyn ActionBackend = &JsonToolCall;
        let msg = Message::assistant(vec![ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "calculator".into(),
            args: serde_json::json!({}),
        }]);
        assert_eq!(backend.parse_actions(&msg).len(), 1);

        let default_backend = <JsonToolCall as Default>::default();
        assert_eq!(default_backend, JsonToolCall);
        assert_eq!(default_backend.parse_actions(&msg).len(), 1);
    }
}
