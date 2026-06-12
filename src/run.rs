//! Agentic loop (design §3).
//!
//! Single-thread `while` loop: call → parse → execute → repeat.

use crate::action::{Action, ActionBackend};
use crate::error::{LoopError, StrataError, ToolError};
use crate::message::{ContentBlock, Message};
use crate::provider::{CompletionRequest, Provider};
use crate::session::Session;
use crate::tool::ToolRegistry;
use crate::trace::{TraceEvent, Tracer};

/// Default completion output token limit per provider call.
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Runs the agentic loop until the model returns a plain-text answer or an error propagates.
///
/// # `max_turns` 语义
///
/// `max_turns` 约束的是**工具往返轮数**，不是 provider 调用次数：`session.turn` 仅在
/// 执行过工具后才递增，纯文本终止轮不计数。由此推出两个反直觉的边界，调用方需注意：
///
/// - `max_turns == 0`：provider 一次都不会被调用，纯文本问答也会立即返回
///   [`LoopError::MaxTurns`]。至少要传 `1` 才能拿到一次回答。
/// - 「一次工具调用 + 最终回答」至少需要 `max_turns >= 2`：`max_turns == 1` 时模型发起
///   工具调用、执行回填后 `turn` 增至 1，下一轮 provider 调用前即被截停，模型永远看不到
///   工具结果。给工具任务留够轮数。
///
/// 超限时返回 [`LoopError::MaxTurns`]，其 `partial` 字段携带终止前最后一条 assistant
/// 纯文本（若有）。注意 [`LoopError`] 的 `Display` 只是摘要、**不含** `partial`——要展示
/// 部分结果，须读取该结构体字段。
pub fn run(
    session: &mut Session,
    provider: &dyn Provider,
    tools: &ToolRegistry,
    backend: &dyn ActionBackend,
    tracer: &dyn Tracer,
    max_turns: u32,
) -> Result<String, StrataError> {
    loop {
        // `session.turn` 计的是「工具往返轮」而非「provider 调用次数」：仅在执行过工具后
        // 才 `+= 1`（见循环末尾），纯文本终止轮不计数。因此 max_turns 实际约束的是工具
        // 循环的往返上限——失控只可能来自工具反复调用，这正是要兜底的对象。
        if session.turn >= max_turns {
            let partial = session.last_assistant_text();
            tracer.on_event(TraceEvent::Error {
                turn: Some(session.turn),
                message: format!("max turns exceeded ({max_turns})"),
            });
            return Err(LoopError::MaxTurns {
                max_turns,
                partial,
            }
            .into());
        }
        tracer.on_event(TraceEvent::TurnStart {
            turn: session.turn,
        });

        // provider 调用失败经 `?` 上抛为 StrataError::Provider，由应用层处理。
        // 此处刻意不发 TraceEvent::Error：失败点的结构化追踪（含 token/耗时/错误）统一在
        // C19 补齐。ProviderCall 事件放在 complete 成功返回之后，语义是「调用成功才记」，
        // 失败轮不会出现 ProviderCall，也符合预期。
        let resp = provider.complete(build_request(session, tools))?;
        tracer.on_event(TraceEvent::ProviderCall {
            turn: session.turn,
        });
        session.history.push(resp.message.clone());

        let actions = backend.parse_actions(&resp.message);
        if actions.is_empty() {
            // TurnEnd 目前只在纯文本终止轮发出，执行工具的轮次结束时不发，因此 TurnStart
            // 与 TurnEnd 暂不成对。这与 §3 伪代码字面一致（伪代码也只在终止处发 TurnEnd），
            // 不算违背设计，但属可观测性缺口：C19 补全事件流时应把 TurnEnd 移到每轮末尾
            // （turn += 1 之前），让每个 TurnStart 都有配对的 TurnEnd。
            tracer.on_event(TraceEvent::TurnEnd {
                turn: session.turn,
            });
            return Ok(resp.message.text());
        }

        for (index, action) in actions.into_iter().enumerate() {
            let turn = session.turn;
            let id = backfill_id(&action, turn, index);
            let name = trace_name(&action.name);

            if let Err(parse_err) = action.validate() {
                backfill_tool_result(
                    session,
                    tracer,
                    turn,
                    &id,
                    &name,
                    parse_err.to_string(),
                    true,
                );
                continue;
            }

            let result = match tools.get(&action.name) {
                Some(tool) => tool.execute(action.args),
                None => Err(ToolError::Unknown {
                    name: action.name.clone(),
                }),
            };

            let (content, is_error) = match result {
                Ok(out) => (out, false),
                Err(e) => (e.to_string(), true),
            };

            backfill_tool_result(session, tracer, turn, &id, &name, content, is_error);
        }

        // 仅在执行过工具后递增：与开头 max_turns 检查呼应，turn 即「已完成的工具往返轮数」。
        session.turn += 1;
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

fn backfill_id(action: &Action, turn: u32, index: usize) -> String {
    if action.id.is_empty() {
        format!("parse_error_{turn}_{index}")
    } else {
        action.id.clone()
    }
}

fn trace_name(name: &str) -> String {
    if name.is_empty() {
        "unknown".into()
    } else {
        name.to_string()
    }
}

fn backfill_tool_result(
    session: &mut Session,
    tracer: &dyn Tracer,
    turn: u32,
    id: &str,
    name: &str,
    content: String,
    is_error: bool,
) {
    tracer.on_event(TraceEvent::ToolCall {
        turn,
        id: id.to_string(),
        name: name.to_string(),
    });
    session.history.push(Message::tool(ContentBlock::ToolResult {
        id: id.to_string(),
        content,
        is_error,
    }));
    tracer.on_event(TraceEvent::ToolResult {
        turn,
        id: id.to_string(),
        name: name.to_string(),
        is_error,
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::action::JsonToolCall;
    use crate::error::{LoopError, ProviderError};
    use crate::message::{ContentBlock, Message};
    use crate::provider::{CompletionResponse, Provider};
    use crate::tools::Calculator;

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

    struct ScriptedProvider {
        responses: RefCell<Vec<Message>>,
        call_count: RefCell<u32>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<Message>) -> Self {
            Self {
                responses: RefCell::new(responses),
                call_count: RefCell::new(0),
            }
        }

        fn calls(&self) -> u32 {
            *self.call_count.borrow()
        }
    }

    impl Provider for ScriptedProvider {
        fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            *self.call_count.borrow_mut() += 1;
            let mut queue = self.responses.borrow_mut();
            assert!(!queue.is_empty(), "ScriptedProvider: no more responses");
            let message = queue.remove(0);
            Ok(CompletionResponse { message })
        }
    }

    struct RepeatingToolCallProvider;

    impl Provider for RepeatingToolCallProvider {
        fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                message: Message::assistant(vec![ContentBlock::ToolCall {
                    id: "call_repeat".into(),
                    name: "calculator".into(),
                    args: serde_json::json!({ "expression": "1+1" }),
                }]),
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

    #[test]
    fn tool_loop_success_two_turns() {
        let mut session = Session::with_history(vec![Message::user("算 1+2")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+2" }),
            }]),
            Message::assistant(vec![ContentBlock::Text("答案是 3".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
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

        assert_eq!(answer, "答案是 3");
        assert_eq!(session.history.len(), 4);
        assert_eq!(session.turn, 1);
    }

    #[test]
    fn tool_loop_emits_tool_trace_events() {
        let mut session = Session::with_history(vec![Message::user("算 1+2")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+2" }),
            }]),
            Message::assistant(vec![ContentBlock::Text("done".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &tracer,
            4,
        )
        .expect("run");

        let events = tracer.0.borrow();
        assert!(
            events.iter().any(|e| matches!(
                e,
                TraceEvent::ToolCall {
                    turn: 0,
                    id,
                    name,
                } if id == "call_1" && name == "calculator"
            ))
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                TraceEvent::ToolResult {
                    turn: 0,
                    id,
                    name,
                    is_error: false,
                } if id == "call_1" && name == "calculator"
            ))
        );
    }

    #[test]
    fn unknown_tool_backfills_error_and_continues() {
        let mut session = Session::with_history(vec![Message::user("call missing")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "missing_tool".into(),
                args: serde_json::json!({}),
            }]),
            Message::assistant(vec![ContentBlock::Text("ok".into())]),
        ]);
        let tools = ToolRegistry::new();
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let answer = run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &tracer,
            4,
        )
        .expect("run");

        assert_eq!(answer, "ok");
        let tool_msg = session.history.iter().find(|m| {
            m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
        });
        assert!(tool_msg.is_some());
    }

    #[test]
    fn tool_invalid_args_backfills_error() {
        let mut session = Session::with_history(vec![Message::user("bad args")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({}),
            }]),
            Message::assistant(vec![ContentBlock::Text("recovered".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let answer = run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &tracer,
            4,
        )
        .expect("run");

        assert_eq!(answer, "recovered");
        let has_error_result = session.history.iter().any(|m| {
            m.content.iter().any(|b| {
                matches!(
                    b,
                    ContentBlock::ToolResult {
                        is_error: true,
                        ..
                    }
                )
            })
        });
        assert!(has_error_result);
    }

    #[test]
    fn multiple_tool_calls_in_one_turn() {
        let mut session = Session::with_history(vec![Message::user("two calcs")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![
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
            ]),
            Message::assistant(vec![ContentBlock::Text("done".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &tracer,
            4,
        )
        .expect("run");

        assert_eq!(session.turn, 1);
        let tool_results: Vec<_> = session
            .history
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { content, is_error, .. } if !is_error => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tool_results, vec!["2", "6"]);
    }

    #[test]
    fn max_turns_zero_fails_immediately() {
        let mut session = Session::with_history(vec![Message::user("go")]);
        let provider = ScriptedProvider::new(vec![Message::assistant(vec![
            ContentBlock::Text("never".into()),
        ])]);
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let err = run(
            &mut session,
            &provider,
            &ToolRegistry::new(),
            &JsonToolCall,
            &tracer,
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
        assert_eq!(session.history.len(), 1);
    }

    #[test]
    fn max_turns_stops_infinite_tool_loop() {
        let mut session = Session::with_history(vec![Message::user("loop")]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let err = run(
            &mut session,
            &RepeatingToolCallProvider,
            &tools,
            &JsonToolCall,
            &tracer,
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
        assert!(session.history.len() > 1);
    }

    #[test]
    fn max_turns_partial_from_last_assistant_text() {
        let mut session = Session::with_history(vec![Message::user("calc")]);
        let provider = ScriptedProvider::new(vec![Message::assistant(vec![
            ContentBlock::Text("step1".into()),
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+1" }),
            },
        ])]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let err = run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &tracer,
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
        assert_eq!(session.turn, 1);
    }

    #[test]
    fn max_turns_emits_error_trace() {
        let mut session = Session::with_history(vec![Message::user("go")]);
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let _ = run(
            &mut session,
            &RepeatingToolCallProvider,
            &ToolRegistry::new(),
            &JsonToolCall,
            &tracer,
            0,
        );

        let events = tracer.0.borrow();
        assert!(events.iter().any(|e| matches!(
            e,
            TraceEvent::Error {
                turn: Some(0),
                message,
            } if message == "max turns exceeded (0)"
        )));
    }

    fn error_tool_results(session: &Session) -> Vec<&ContentBlock> {
        session
            .history
            .iter()
            .flat_map(|m| m.content.iter())
            .filter(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
            .collect()
    }

    #[test]
    fn non_object_string_args_backfills_parse_error() {
        let mut session = Session::with_history(vec![Message::user("bad args")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::Value::String("{bad".into()),
            }]),
            Message::assistant(vec![ContentBlock::Text("recovered".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));

        let answer = run(
            &mut session,
            &provider,
            &tools,
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
    fn non_object_scalar_args_backfills_and_does_not_panic() {
        let mut session = Session::with_history(vec![Message::user("scalar args")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!(42),
            }]),
            Message::assistant(vec![ContentBlock::Text("recovered".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));

        let answer = run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &RecordingTracer(RefCell::new(Vec::new())),
            4,
        )
        .expect("run");

        assert_eq!(answer, "recovered");
        assert_eq!(error_tool_results(&session).len(), 1);
    }

    #[test]
    fn empty_tool_call_id_backfills_and_continues() {
        let mut session = Session::with_history(vec![Message::user("no id")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: String::new(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1+1" }),
            }]),
            Message::assistant(vec![ContentBlock::Text("ok".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));
        let tracer = RecordingTracer(RefCell::new(Vec::new()));

        let answer = run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &tracer,
            4,
        )
        .expect("run");

        assert_eq!(answer, "ok");
        let errors = error_tool_results(&session);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0],
            ContentBlock::ToolResult { id, .. } if id == "parse_error_0_0"
        ));
    }

    #[test]
    fn empty_tool_name_backfills_and_continues() {
        let mut session = Session::with_history(vec![Message::user("no name")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: String::new(),
                args: serde_json::json!({}),
            }]),
            Message::assistant(vec![ContentBlock::Text("ok".into())]),
        ]);
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

        assert_eq!(answer, "ok");
        assert!(error_tool_results(&session)
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { content, .. } if content.contains("missing tool name"))));
    }

    #[test]
    fn tool_execution_failed_backfills_and_continues() {
        let mut session = Session::with_history(vec![Message::user("divide")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "calculator".into(),
                args: serde_json::json!({ "expression": "1/0" }),
            }]),
            Message::assistant(vec![ContentBlock::Text("recovered".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));

        let answer = run(
            &mut session,
            &provider,
            &tools,
            &JsonToolCall,
            &RecordingTracer(RefCell::new(Vec::new())),
            4,
        )
        .expect("run");

        assert_eq!(answer, "recovered");
        assert!(error_tool_results(&session)
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { content, .. } if content.contains("division by zero"))));
    }

    #[test]
    fn parse_and_tool_errors_same_turn() {
        let mut session = Session::with_history(vec![Message::user("mixed")]);
        let provider = ScriptedProvider::new(vec![
            Message::assistant(vec![
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
            ]),
            Message::assistant(vec![ContentBlock::Text("done".into())]),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(Calculator));

        run(
            &mut session,
            &provider,
            &tools,
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
}
