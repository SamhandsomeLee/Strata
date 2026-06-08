//! Provider trait and completion request/response types (design §2.2).
//!
//! Model-specific auth, API serialization, and native tool-call translation live in
//! `src/providers/` implementations (C10+). This module exposes only the trait boundary.

use crate::error::ProviderError;
use crate::message::Message;
use crate::tool::ToolSchema;

/// Unified completion input. Must not expose any provider-specific concepts.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl CompletionRequest {
    pub fn new(messages: Vec<Message>, max_tokens: u32) -> Self {
        Self {
            messages,
            tools: Vec::new(),
            max_tokens,
            temperature: None,
        }
    }
}

/// Unified completion output: one assistant turn in the internal message model.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionResponse {
    pub message: Message,
}

/// Model-agnostic provider boundary. Implementations translate to/from native API formats.
pub trait Provider {
    fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ContentBlock;

    struct EchoProvider;

    impl Provider for EchoProvider {
        fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                message: Message::assistant(vec![ContentBlock::Text("ok".into())]),
            })
        }
    }

    #[test]
    fn completion_request_constructs_with_defaults() {
        let req = CompletionRequest::new(vec![Message::user("hi")], 256);
        assert_eq!(req.messages.len(), 1);
        assert!(req.tools.is_empty());
        assert_eq!(req.max_tokens, 256);
        assert_eq!(req.temperature, None);
    }

    #[test]
    fn tool_schema_holds_json_parameters() {
        use crate::tool::ToolSchema;

        let schema = ToolSchema {
            name: "calculator".into(),
            description: "Evaluate arithmetic.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "expression": { "type": "string" } }
            }),
        };
        assert_eq!(schema.parameters["type"], "object");
    }

    #[test]
    fn provider_trait_is_object_safe() {
        let provider: &dyn Provider = &EchoProvider;
        let resp = provider
            .complete(CompletionRequest::new(vec![Message::user("ping")], 64))
            .expect("echo");
        assert_eq!(resp.message.text(), "ok");
    }
}
