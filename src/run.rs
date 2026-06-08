//! Agentic loop (design §3).
//!
//! Single-thread `while` loop: call → parse → execute → repeat.
//! `build_request` and tool execution are stubbed until C12 / C15.

use crate::action::ActionBackend;
use crate::error::{LoopError, StrataError};
use crate::provider::{CompletionRequest, Provider};
use crate::session::Session;
use crate::tool::ToolRegistry;
use crate::trace::{TraceEvent, Tracer};

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

fn build_request(_session: &Session, _tools: &ToolRegistry) -> CompletionRequest {
    todo!("C12: assemble CompletionRequest from session.history and tool schemas")
}
