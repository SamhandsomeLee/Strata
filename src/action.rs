//! ActionBackend trait and action types (design §2.3).
//!
//! [`ActionBackend`] turns an assistant [`Message`] into executable [`Action`]s.
//! MVP uses [`JsonToolCall`] (extracts `ContentBlock::ToolCall`); future
//! backends such as CodeAction can implement the same trait without changing the loop.

use crate::error::ParseError;
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

    /// Pre-execute validation. Failures become [`ParseError`] backfill in the loop, not `Err`.
    ///
    /// `args` 的契约是 JSON object（命名参数）或 `null`（无参数）。其它形态——裸字符串、
    /// 数字、布尔、数组——都视为模型没把工具参数包成对象，归为 [`ParseError::InvalidToolCall`]
    /// 回填，让模型纠正。args 内部字段的语义校验仍归各工具的 `execute` 负责。
    pub fn validate(&self) -> Result<(), ParseError> {
        if self.id.is_empty() {
            return Err(ParseError::InvalidToolCall("missing tool call id".into()));
        }
        if self.name.is_empty() {
            return Err(ParseError::InvalidToolCall("missing tool name".into()));
        }
        if !matches!(
            self.args,
            serde_json::Value::Object(_) | serde_json::Value::Null
        ) {
            return Err(ParseError::InvalidToolCall(format!(
                "tool arguments must be a JSON object, got: {}",
                self.args
            )));
        }
        Ok(())
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
    fn validate_rejects_malformed_tool_calls() {
        let empty_id = Action::new("", "calculator", serde_json::json!({}));
        assert!(matches!(
            empty_id.validate(),
            Err(ParseError::InvalidToolCall(_))
        ));

        let empty_name = Action::new("call_1", "", serde_json::json!({}));
        assert!(matches!(
            empty_name.validate(),
            Err(ParseError::InvalidToolCall(_))
        ));

        let string_args = Action::new(
            "call_1",
            "calculator",
            serde_json::Value::String("{bad".into()),
        );
        assert!(matches!(
            string_args.validate(),
            Err(ParseError::InvalidToolCall(_))
        ));

        let ok = Action::new("call_1", "calculator", serde_json::json!({ "x": 1 }));
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_rejects_non_object_args() {
        for bad in [
            serde_json::json!(42),
            serde_json::json!(true),
            serde_json::json!([1, 2, 3]),
        ] {
            let action = Action::new("call_1", "calculator", bad);
            assert!(matches!(
                action.validate(),
                Err(ParseError::InvalidToolCall(_))
            ));
        }

        // object 与 null 都是合法形态。
        assert!(Action::new("call_1", "t", serde_json::json!({})).validate().is_ok());
        assert!(Action::new("call_1", "t", serde_json::Value::Null).validate().is_ok());
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
