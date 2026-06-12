//! Tracer trait and trace events (design §2.4).
//!
//! Structured event stream for the agentic loop. Implementations may write to console,
//! files, or OpenTelemetry (§7).

use std::fmt;

use crate::provider::TokenUsage;

/// Structured trace event emitted by the agentic loop.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceEvent {
    TurnStart { turn: u32 },
    ProviderCall {
        turn: u32,
        duration_ms: u64,
        usage: Option<TokenUsage>,
    },
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
        /// Set when the error follows a failed provider call.
        duration_ms: Option<u64>,
    },
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceEvent::TurnStart { turn } => {
                write!(f, "[strata] turn={turn} event=turn_start")
            }
            TraceEvent::ProviderCall {
                turn,
                duration_ms,
                usage,
            } => {
                write!(
                    f,
                    "[strata] turn={turn} event=provider_call duration_ms={duration_ms}"
                )?;
                if let Some(u) = usage {
                    write!(
                        f,
                        " prompt_tokens={} completion_tokens={} total_tokens={}",
                        u.prompt_tokens, u.completion_tokens, u.total_tokens
                    )?;
                }
                Ok(())
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
            TraceEvent::Error {
                turn,
                message,
                duration_ms,
            } => {
                // 记录（C19 审查）：message 未加引号/转义，且含空格。当前 message 恒在
                // duration_ms 之前、duration 恒在末位，朴素 key=value 解析仍可切出 duration；
                // 但 message 内部空格会干扰严格机读。若将来要严格机器解析，给 message 加引号
                // 或移到行末。
                if let Some(t) = turn {
                    write!(f, "[strata] turn={t} event=error message={message}")?;
                } else {
                    write!(f, "[strata] event=error message={message}")?;
                }
                if let Some(ms) = duration_ms {
                    write!(f, " duration_ms={ms}")?;
                }
                Ok(())
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
            TraceEvent::ProviderCall {
                turn: 0,
                duration_ms: 842,
                usage: Some(TokenUsage {
                    prompt_tokens: 120,
                    completion_tokens: 45,
                    total_tokens: 165,
                }),
            }
            .to_string(),
            "[strata] turn=0 event=provider_call duration_ms=842 prompt_tokens=120 completion_tokens=45 total_tokens=165"
        );
        assert_eq!(
            TraceEvent::ProviderCall {
                turn: 1,
                duration_ms: 12,
                usage: None,
            }
            .to_string(),
            "[strata] turn=1 event=provider_call duration_ms=12"
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
                message: "network error: timeout".into(),
                duration_ms: Some(30_001),
            }
            .to_string(),
            "[strata] turn=2 event=error message=network error: timeout duration_ms=30001"
        );
        assert_eq!(
            TraceEvent::Error {
                turn: None,
                message: "max turns exceeded (10)".into(),
                duration_ms: None,
            }
            .to_string(),
            "[strata] event=error message=max turns exceeded (10)"
        );
    }

    #[test]
    fn tracers_are_object_safe() {
        let events = [
            TraceEvent::TurnStart { turn: 0 },
            TraceEvent::ProviderCall {
                turn: 0,
                duration_ms: 1,
                usage: None,
            },
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
                duration_ms: None,
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
