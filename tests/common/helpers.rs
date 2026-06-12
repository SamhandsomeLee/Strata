//! Fixtures and assertion helpers for loop integration tests.

use strata::{Calculator, CompletionResponse, ContentBlock, Message, Session, TokenUsage, ToolRegistry};

use super::mock_provider::MockStep;

pub fn calculator_registry() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(Calculator));
    tools
}

pub fn ok_text(text: &str) -> MockStep {
    MockStep::Ok(CompletionResponse {
        message: Message::assistant(vec![ContentBlock::Text(text.into())]),
        usage: None,
    })
}

pub fn ok_message(message: Message) -> MockStep {
    MockStep::Ok(CompletionResponse {
        message,
        usage: None,
    })
}

pub fn ok_tool(name: &str, id: &str, args: serde_json::Value) -> MockStep {
    MockStep::Ok(CompletionResponse {
        message: Message::assistant(vec![ContentBlock::ToolCall {
            id: id.into(),
            name: name.into(),
            args,
        }]),
        usage: None,
    })
}

pub fn ok_with_usage(message: Message, usage: TokenUsage) -> MockStep {
    MockStep::Ok(CompletionResponse {
        message,
        usage: Some(usage),
    })
}

pub fn err_network(message: &str) -> MockStep {
    MockStep::Err(strata::ProviderError::Network(message.into()))
}

pub fn repeating_calculator_add_one() -> super::mock_provider::MockProvider {
    super::mock_provider::MockProvider::repeating(ok_tool(
        "calculator",
        "call_repeat",
        serde_json::json!({ "expression": "1+1" }),
    ))
}

pub fn error_tool_results(session: &Session) -> Vec<&ContentBlock> {
    session
        .history
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
        .collect()
}

pub fn turn_end_count(events: &[strata::TraceEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, strata::TraceEvent::TurnEnd { .. }))
        .count()
}
