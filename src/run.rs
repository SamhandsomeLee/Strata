//! Agentic loop (design §3).
//!
//! Single-thread `while` loop: call → parse → execute → repeat.

use crate::action::ActionBackend;
use crate::error::{LoopError, StrataError, ToolError};
use crate::message::{ContentBlock, Message};
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
        // `session.turn` 计的是「工具往返轮」而非「provider 调用次数」：仅在执行过工具后
        // 才 `+= 1`（见循环末尾），纯文本终止轮不计数。因此 max_turns 实际约束的是工具
        // 循环的往返上限——失控只可能来自工具反复调用，这正是要兜底的对象。
        // 注意：这里仅做提前返回，带部分结果的优雅终止与边界测试属 C17/C20，本轮不实现。
        if session.turn >= max_turns {
            return Err(LoopError::MaxTurns { max_turns }.into());
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

        for action in actions {
            tracer.on_event(TraceEvent::ToolCall {
                turn: session.turn,
                id: action.id.clone(),
                name: action.name.clone(),
            });

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

            session.history.push(Message::tool(ContentBlock::ToolResult {
                id: action.id.clone(),
                content,
                is_error,
            }));

            tracer.on_event(TraceEvent::ToolResult {
                turn: session.turn,
                id: action.id,
                name: action.name,
                is_error,
            });
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::action::JsonToolCall;
    use crate::error::ProviderError;
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
    }

    impl ScriptedProvider {
        fn new(responses: Vec<Message>) -> Self {
            Self {
                responses: RefCell::new(responses),
            }
        }
    }

    impl Provider for ScriptedProvider {
        fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            let mut queue = self.responses.borrow_mut();
            assert!(!queue.is_empty(), "ScriptedProvider: no more responses");
            let message = queue.remove(0);
            Ok(CompletionResponse { message })
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
}
