//! Agentic loop semantics — M3 acceptance (no network).

mod common;

use std::cell::RefCell;

use common::{
    calculator_registry, err_network, error_tool_results, ok_message, ok_text, ok_tool,
    ok_with_usage, repeating_calculator_add_one, turn_end_count, MockProvider, RecordingTracer,
};
use strata::{
    run, ContentBlock, JsonToolCall, LoopError, Message, Role, Session, StrataError, TokenUsage,
    ToolRegistry, TraceEvent,
};

#[test]
fn plain_text_terminates_with_answer_and_trace() {
    let mut session = Session::with_history(vec![Message::user("ping")]);
    let provider = MockProvider::new(vec![ok_text("hello")]);
    let tracer = RecordingTracer(RefCell::new(Vec::new()));

    let answer = run(
        &mut session,
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &tracer,
        4,
    )
    .expect("run");

    assert_eq!(answer, "hello");
    assert_eq!(session.history.len(), 2);

    let events = tracer.0.borrow();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], TraceEvent::TurnStart { turn: 0 }));
    assert!(matches!(
        events[1],
        TraceEvent::ProviderCall {
            turn: 0,
            duration_ms: _,
            usage: None,
        }
    ));
    assert!(matches!(events[2], TraceEvent::TurnEnd { turn: 0 }));
}

#[test]
fn tool_loop_two_turns_full_cycle() {
    let mut session = Session::with_history(vec![Message::user("算 1+2")]);
    let provider = MockProvider::new(vec![
        ok_tool(
            "calculator",
            "call_1",
            serde_json::json!({ "expression": "1+2" }),
        ),
        ok_text("答案是 3"),
    ]);
    let tracer = RecordingTracer(RefCell::new(Vec::new()));

    let answer = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &tracer,
        4,
    )
    .expect("run");

    assert_eq!(answer, "答案是 3");
    assert_eq!(session.history.len(), 4);
    assert_eq!(session.turn, 1);
    assert_eq!(turn_end_count(&tracer.0.borrow()), 2);
}

#[test]
fn tool_trace_events_emitted() {
    let mut session = Session::with_history(vec![Message::user("算 1+2")]);
    let provider = MockProvider::new(vec![
        ok_tool(
            "calculator",
            "call_1",
            serde_json::json!({ "expression": "1+2" }),
        ),
        ok_text("done"),
    ]);
    let tracer = RecordingTracer(RefCell::new(Vec::new()));

    run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &tracer,
        4,
    )
    .expect("run");

    let events = tracer.0.borrow();
    assert!(events.iter().any(|e| matches!(
        e,
        TraceEvent::ToolCall {
            turn: 0,
            id,
            name,
        } if id == "call_1" && name == "calculator"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        TraceEvent::ToolResult {
            turn: 0,
            id,
            name,
            is_error: false,
        } if id == "call_1" && name == "calculator"
    )));
}

#[test]
fn second_provider_request_includes_tool_result() {
    let mut session = Session::with_history(vec![Message::user("算 1+2")]);
    let provider = MockProvider::new(vec![
        ok_tool(
            "calculator",
            "call_1",
            serde_json::json!({ "expression": "1+2" }),
        ),
        ok_text("答案是 3"),
    ]);

    run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(provider.calls(), 2);
    let requests = provider.recorded_requests();
    assert_eq!(requests.len(), 2);
    let second = &requests[1];
    assert!(second.messages.iter().any(|m| {
        m.role == Role::Tool
            && m.content.iter().any(|b| matches!(
                b,
                ContentBlock::ToolResult { content, is_error: false, .. } if content == "3"
            ))
    }));
}

#[test]
fn multiple_tool_calls_one_round() {
    let mut session = Session::with_history(vec![Message::user("two calcs")]);
    let provider = MockProvider::new(vec![
        ok_message(Message::assistant(vec![
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+1" }),
            },
            ContentBlock::ToolCall {
                id: "call_2".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "2*3" }),
            },
        ])),
        ok_text("done"),
    ]);

    run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(session.turn, 1);
    let ok_results: Vec<_> = session
        .history
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, is_error, .. } if !is_error => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ok_results, vec!["2", "6"]);
}

#[test]
fn unknown_tool_backfills_and_recovers() {
    let mut session = Session::with_history(vec![Message::user("call missing")]);
    let provider = MockProvider::new(vec![
        ok_tool("missing_tool", "call_1", serde_json::json!({})),
        ok_text("ok"),
    ]);

    let answer = run(
        &mut session,
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(answer, "ok");
    assert!(!error_tool_results(&session).is_empty());
}

#[test]
fn invalid_tool_args_backfill() {
    let mut session = Session::with_history(vec![Message::user("bad args")]);
    let provider = MockProvider::new(vec![
        ok_tool("calculator", "call_1", serde_json::json!({})),
        ok_text("recovered"),
    ]);

    let answer = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(answer, "recovered");
    assert_eq!(error_tool_results(&session).len(), 1);
}

#[test]
fn tool_execution_error_backfill() {
    let mut session = Session::with_history(vec![Message::user("divide")]);
    let provider = MockProvider::new(vec![
        ok_tool(
            "calculator",
            "call_1",
            serde_json::json!({ "expression": "1/0" }),
        ),
        ok_text("recovered"),
    ]);

    let answer = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(answer, "recovered");
    assert!(error_tool_results(&session).iter().any(|b| matches!(
        b,
        ContentBlock::ToolResult { content, .. } if content.contains("division by zero")
    )));
}

#[test]
fn parse_errors_backfill_non_object_args() {
    let mut session = Session::with_history(vec![Message::user("bad args")]);
    let provider = MockProvider::new(vec![
        ok_tool(
            "calculator",
            "call_1",
            serde_json::Value::String("{bad".into()),
        ),
        ok_text("recovered"),
    ]);

    let answer = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(answer, "recovered");
    let errors = error_tool_results(&session);
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        errors[0],
        ContentBlock::ToolResult { content, .. } if content.contains("must be a JSON object")
    ));
}

#[test]
fn parse_errors_backfill_scalar_args() {
    let mut session = Session::with_history(vec![Message::user("scalar")]);
    let provider = MockProvider::new(vec![
        ok_tool("calculator", "call_1", serde_json::json!(42)),
        ok_text("recovered"),
    ]);

    run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(error_tool_results(&session).len(), 1);
}

#[test]
fn empty_tool_call_id_backfills_and_continues() {
    let mut session = Session::with_history(vec![Message::user("no id")]);
    let provider = MockProvider::new(vec![
        ok_message(Message::assistant(vec![ContentBlock::ToolCall {
            id: String::new(),
            name: "calculator".into(),
            args: serde_json::json!({ "expression": "1+1" }),
        }])),
        ok_text("ok"),
    ]);

    let answer = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(answer, "ok");
    assert!(matches!(
        error_tool_results(&session)[0],
        ContentBlock::ToolResult { id, .. } if id == "parse_error_0_0"
    ));
}

#[test]
fn empty_tool_name_backfills_and_continues() {
    let mut session = Session::with_history(vec![Message::user("no name")]);
    let provider = MockProvider::new(vec![
        ok_message(Message::assistant(vec![ContentBlock::ToolCall {
            id: "call_1".into(),
            name: String::new(),
            args: serde_json::json!({}),
        }])),
        ok_text("ok"),
    ]);

    let answer = run(
        &mut session,
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    assert_eq!(answer, "ok");
    assert!(error_tool_results(&session).iter().any(|b| matches!(
        b,
        ContentBlock::ToolResult { content, .. } if content.contains("missing tool name")
    )));
}

#[test]
fn parse_and_tool_errors_same_turn() {
    let mut session = Session::with_history(vec![Message::user("mixed")]);
    let provider = MockProvider::new(vec![
        ok_message(Message::assistant(vec![
            ContentBlock::ToolCall {
                id: "call_bad".into(),
                name: "calculator".into(),
                args: serde_json::Value::String("not json".into()),
            },
            ContentBlock::ToolCall {
                id: "call_ok".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+1" }),
            },
        ])),
        ok_text("done"),
    ]);

    run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("run");

    let ok_results: Vec<_> = session
        .history
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, is_error, .. } if !is_error => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ok_results, vec!["2"]);
    assert_eq!(error_tool_results(&session).len(), 1);
}

#[test]
fn max_turns_zero_no_provider_call() {
    let mut session = Session::with_history(vec![Message::user("go")]);
    let provider = MockProvider::new(vec![ok_text("never")]);

    let err = run(
        &mut session,
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        0,
    )
    .expect_err("run");

    assert!(matches!(
        err,
        StrataError::Loop(LoopError::MaxTurns {
            max_turns: 0,
            partial: None,
        })
    ));
    assert_eq!(provider.calls(), 0);
}

#[test]
fn max_turns_stops_infinite_tool_loop() {
    let mut session = Session::with_history(vec![Message::user("loop")]);
    let provider = repeating_calculator_add_one();

    let err = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        2,
    )
    .expect_err("run");

    assert!(matches!(
        err,
        StrataError::Loop(LoopError::MaxTurns {
            max_turns: 2,
            partial: None,
        })
    ));
    assert_eq!(session.turn, 2);
}

#[test]
fn max_turns_partial_from_last_assistant_text() {
    let mut session = Session::with_history(vec![Message::user("calc")]);
    let provider = MockProvider::new(vec![ok_message(Message::assistant(vec![
        ContentBlock::Text("step1".into()),
        ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "calculator".into(),
            args: serde_json::json!({ "expression": "1+1" }),
        },
    ]))]);

    let err = run(
        &mut session,
        &provider,
        &calculator_registry(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        1,
    )
    .expect_err("run");

    assert!(matches!(
        err,
        StrataError::Loop(LoopError::MaxTurns {
            max_turns: 1,
            partial: Some(ref text),
        }) if text == "step1"
    ));
    assert_eq!(provider.calls(), 1);
}

#[test]
fn max_turns_emits_error_trace() {
    let tracer = RecordingTracer(RefCell::new(Vec::new()));

    let _ = run(
        &mut Session::with_history(vec![Message::user("go")]),
        &repeating_calculator_add_one(),
        &ToolRegistry::new(),
        &JsonToolCall,
        &tracer,
        0,
    );

    assert!(tracer.0.borrow().iter().any(|e| matches!(
        e,
        TraceEvent::Error {
            turn: Some(0),
            message,
            duration_ms: None,
        } if message == "max turns exceeded (0)"
    )));
}

#[test]
fn provider_failure_traced_with_duration() {
    let tracer = RecordingTracer(RefCell::new(Vec::new()));

    let err = run(
        &mut Session::with_history(vec![Message::user("ping")]),
        &MockProvider::new(vec![err_network("connection reset")]),
        &ToolRegistry::new(),
        &JsonToolCall,
        &tracer,
        4,
    )
    .expect_err("run");

    assert!(matches!(err, StrataError::Provider(_)));
    assert!(tracer.0.borrow().iter().any(|e| matches!(
        e,
        TraceEvent::Error {
            turn: Some(0),
            message,
            duration_ms: Some(_),
        } if message.contains("connection reset")
    )));
    assert!(
        !tracer
            .0
            .borrow()
            .iter()
            .any(|e| matches!(e, TraceEvent::ProviderCall { .. }))
    );
}

#[test]
fn provider_usage_in_trace() {
    let tracer = RecordingTracer(RefCell::new(Vec::new()));

    run(
        &mut Session::with_history(vec![Message::user("ping")]),
        &MockProvider::new(vec![ok_with_usage(
            Message::assistant(vec![ContentBlock::Text("ok".into())]),
            TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        )]),
        &ToolRegistry::new(),
        &JsonToolCall,
        &tracer,
        4,
    )
    .expect("run");

    assert!(tracer.0.borrow().iter().any(|e| matches!(
        e,
        TraceEvent::ProviderCall {
            turn: 0,
            duration_ms: _,
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
        }
    )));
}

#[test]
fn mock_script_exhausted_returns_provider_error() {
    let provider = MockProvider::new(vec![ok_text("once")]);

    run(
        &mut Session::with_history(vec![Message::user("q")]),
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect("first run");

    let err = run(
        &mut Session::with_history(vec![Message::user("q2")]),
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &RecordingTracer(RefCell::new(Vec::new())),
        4,
    )
    .expect_err("second run");

    assert!(matches!(err, StrataError::Provider(_)));
    assert!(
        err.to_string()
            .contains("mock script exhausted")
    );
}
