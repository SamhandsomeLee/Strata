//! Agentic loop (design §3).
//!
//! Single-thread `while` loop: call → parse → execute → repeat.

use std::time::Instant;

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
                duration_ms: None,
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

        // 仅测 provider 调用耗时——它是每轮耗时的主项。设计 §6 提到「每个 turn 的耗时」，
        // 更细的工具执行耗时 / turn 级总耗时暂不记（MVP 不需要），需要时作独立增强。
        // `as u64`：as_millis() 返回 u128，溢出需 ~5.8 亿年，实际无害。
        // 记录（C19 审查）：耗时目前只覆盖 provider 调用本身，未含工具执行耗时或整轮耗时。
        // provider 调用是耗时主项，对 MVP 足够；若将来要严格对齐设计 §6「每个 turn 的耗时」，
        // 可另加 turn 级 duration 或在 ToolResult 上带耗时，建议作为独立改动而非扩到这里。
        // `as u64` 截断仅在 elapsed 超过约 5.8 亿年时发生，实际无害。
        let started = Instant::now();
        let completion = provider.complete(build_request(session, tools));
        let duration_ms = started.elapsed().as_millis() as u64;

        let resp = match completion {
            Ok(resp) => {
                tracer.on_event(TraceEvent::ProviderCall {
                    turn: session.turn,
                    duration_ms,
                    usage: resp.usage,
                });
                resp
            }
            Err(e) => {
                // 记录（C19 审查）：失败轮的 Error 事件不带 token usage。少数 provider 在
                // 限流/超时响应里也会回 usage，此处会丢；MVP 不处理，仅标记。
                tracer.on_event(TraceEvent::Error {
                    turn: Some(session.turn),
                    message: e.to_string(),
                    duration_ms: Some(duration_ms),
                });
                return Err(e.into());
            }
        };

        session.history.push(resp.message.clone());

        let actions = backend.parse_actions(&resp.message);
        if actions.is_empty() {
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

        tracer.on_event(TraceEvent::TurnEnd {
            turn: session.turn,
        });

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
    use super::*;
    use crate::message::Message;

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
}
