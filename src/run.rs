//! Agentic loop (design §3).
//!
//! Single-thread `while` loop: call → parse → execute → repeat.
//! Tool execution is stubbed until C15.

use crate::action::ActionBackend;
use crate::error::{LoopError, StrataError};
use crate::provider::{CompletionRequest, Provider};
use crate::session::Session;
use crate::tool::ToolRegistry;
use crate::trace::{TraceEvent, Tracer};

/// Default completion output token limit per provider call.
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Runs the agentic loop until the model returns a plain-text answer or an error propagates.
pub fn run(
    session: &mut Session,
    provider: &dyn Provider,
    tools: &ToolRegistry,
    backend: &dyn ActionBackend,
    tracer: &dyn Tracer,
    max_turns: u32,
) -> Result<String, StrataError> {
    loop {
        if session.turn >= max_turns {
            return Err(LoopError::MaxTurns { max_turns }.into());
        }
        tracer.on_event(TraceEvent::TurnStart {
            turn: session.turn,
        });

        let resp = provider.complete(build_request(session, tools))?;
        tracer.on_event(TraceEvent::ProviderCall {
            turn: session.turn,
        });
        session.history.push(resp.message.clone());

        let actions = backend.parse_actions(&resp.message);
        if actions.is_empty() {
            tracer.on_event(TraceEvent::TurnEnd {
                turn: session.turn,
            });
            return Ok(resp.message.text());
        }

        todo!("C15: execute actions, backfill ToolResult, increment turn");
    }
}

fn build_request(session: &Session, tools: &ToolRegistry) -> CompletionRequest {
    CompletionRequest {
        messages: session.history.clone(),
        tools: tools.schemas(),
        max_tokens: DEFAULT_MAX_TOKENS,
        temperature: None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::action::JsonToolCall;
    use crate::error::ProviderError;
    use crate::message::{ContentBlock, Message};
    use crate::provider::{CompletionResponse, Provider};

    struct TextProvider;

    impl Provider for TextProvider {
        fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                message: Message::assistant(vec![ContentBlock::Text("hello".into())]),
            })
        }
    }

    struct RecordingTracer(RefCell<Vec<TraceEvent>>);

    impl Tracer for RecordingTracer {
        fn on_event(&self, event: TraceEvent) {
            self.0.borrow_mut().push(event);
        }
    }

    #[test]
    fn build_request_assembles_session_and_tools() {
        let session = Session::with_history(vec![Message::user("hi")]);
        let tools = ToolRegistry::new();
        let req = build_request(&session, &tools);

        assert_eq!(req.messages.len(), 1);
        assert!(req.tools.is_empty());
        assert_eq!(req.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(req.temperature, None);
    }

    #[test]
    fn plain_text_termination_emits_trace_and_returns_answer() {
        let mut session = Session::with_history(vec![Message::user("ping")]);
        let provider = TextProvider;
        let tools = ToolRegistry::new();
        let backend = JsonToolCall;
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let answer = run(
            &mut session,
            &provider,
            &tools,
            &backend,
            &tracer,
            4,
        )
        .expect("run");

        assert_eq!(answer, "hello");
        assert_eq!(session.history.len(), 2);

        let events = tracer.0.borrow();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], TraceEvent::TurnStart { turn: 0 }));
        assert!(matches!(events[1], TraceEvent::ProviderCall { turn: 0 }));
        assert!(matches!(events[2], TraceEvent::TurnEnd { turn: 0 }));
    }
}
