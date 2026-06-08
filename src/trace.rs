//! Tracer trait and trace events (design §2.4).
//!
//! Structured event stream for the agentic loop. Implementations may write to console,
//! files, or OpenTelemetry (§7). C19 enriches events with token counts and duration.

use std::fmt;

/// Structured trace event emitted by the agentic loop.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceEvent {
    TurnStart { turn: u32 },
    ProviderCall { turn: u32 },
    ToolCall {
        turn: u32,
        id: String,
        name: String,
    },
    ToolResult {
        turn: u32,
        id: String,
        name: String,
        is_error: bool,
    },
    TurnEnd { turn: u32 },
    Error {
        /// `None` for errors outside a turn (e.g. setup); `Some` to keep the failing turn's context.
        turn: Option<u32>,
        message: String,
    },
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceEvent::TurnStart { turn } => {
                write!(f, "[strata] turn={turn} event=turn_start")
            }
            TraceEvent::ProviderCall { turn } => {
                write!(f, "[strata] turn={turn} event=provider_call")
            }
            TraceEvent::ToolCall { turn, id, name } => {
                write!(
                    f,
                    "[strata] turn={turn} event=tool_call name={name} id={id}"
                )
            }
            TraceEvent::ToolResult {
                turn,
                id,
                name,
                is_error,
            } => {
                write!(
                    f,
                    "[strata] turn={turn} event=tool_result name={name} id={id} is_error={is_error}"
                )
            }
            TraceEvent::TurnEnd { turn } => {
                write!(f, "[strata] turn={turn} event=turn_end")
            }
            TraceEvent::Error { turn: Some(turn), message } => {
                write!(f, "[strata] turn={turn} event=error message={message}")
            }
            TraceEvent::Error { turn: None, message } => {
                write!(f, "[strata] event=error message={message}")
            }
        }
    }
}

/// Observes structured loop events without affecting control flow.
pub trait Tracer {
    fn on_event(&self, event: TraceEvent);
}

/// Default tracer: human-readable lines on stderr.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleTracer;

impl Tracer for ConsoleTracer {
    fn on_event(&self, event: TraceEvent) {
        eprintln!("{event}");
    }
}

/// Silent tracer for tests or runs that should not emit output.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NoopTracer;

impl Tracer for NoopTracer {
    fn on_event(&self, _event: TraceEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_event_display_format() {
        assert_eq!(
            TraceEvent::TurnStart { turn: 0 }.to_string(),
            "[strata] turn=0 event=turn_start"
        );
        assert_eq!(
            TraceEvent::ToolCall {
                turn: 1,
                id: "call_1".into(),
                name: "calculator".into(),
            }
            .to_string(),
            "[strata] turn=1 event=tool_call name=calculator id=call_1"
        );
        assert_eq!(
            TraceEvent::Error {
                turn: Some(2),
                message: "tool panicked".into(),
            }
            .to_string(),
            "[strata] turn=2 event=error message=tool panicked"
        );
        assert_eq!(
            TraceEvent::Error {
                turn: None,
                message: "max turns exceeded (10)".into(),
            }
            .to_string(),
            "[strata] event=error message=max turns exceeded (10)"
        );
    }

    #[test]
    fn tracers_are_object_safe() {
        let events = [
            TraceEvent::TurnStart { turn: 0 },
            TraceEvent::ProviderCall { turn: 0 },
            TraceEvent::ToolCall {
                turn: 0,
                id: "c".into(),
                name: "t".into(),
            },
            TraceEvent::ToolResult {
                turn: 0,
                id: "c".into(),
                name: "t".into(),
                is_error: false,
            },
            TraceEvent::TurnEnd { turn: 0 },
            TraceEvent::Error {
                turn: Some(0),
                message: "oops".into(),
            },
        ];

        let console: &dyn Tracer = &ConsoleTracer;
        let noop: &dyn Tracer = &NoopTracer;
        for event in events {
            console.on_event(event.clone());
            noop.on_event(event);
        }
    }
}
