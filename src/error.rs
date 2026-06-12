//! Error taxonomy (design §6).
//!
//! # 上抛 vs 回填
//!
//! - **可上抛**：[`ProviderError`]、[`LoopError`] → 包装为 [`StrataError`]，由应用层处理。
//! - **不上抛**：[`ParseError`]、[`ToolError`] → 循环内转为对话回填，**不得** `Into<StrataError>`。

use thiserror::Error;

/// Provider 层错误：网络、鉴权、限流、响应协议解析失败。可上抛或重试。
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("network error: {0}")]
    Network(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("rate limited: {0}")]
    RateLimit(String),

    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
}

/// 循环失控保护。可上抛，由应用层终止并展示原因。
#[derive(Debug, Error)]
pub enum LoopError {
    #[error("max turns exceeded ({max_turns})")]
    MaxTurns {
        max_turns: u32,
        /// Last assistant plain-text before termination, if any.
        partial: Option<String>,
    },
}

/// 模型输出或 action 解析错误。循环内回填，不进 [`StrataError`]。
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid JSON: {0}")]
    InvalidJson(String),

    #[error("invalid tool call: {0}")]
    InvalidToolCall(String),
}

/// 工具执行错误。循环内回填为 `ToolResult { is_error: true }`，不进 [`StrataError`]。
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool: {name}")]
    Unknown { name: String },

    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),
}

/// 应用层可见的错误边界：仅包含可上抛的 provider / loop 错误。
#[derive(Debug, Error)]
pub enum StrataError {
    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    Loop(#[from] LoopError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_display() {
        let err = ProviderError::Network("connection reset".into());
        assert_eq!(err.to_string(), "network error: connection reset");

        let err = ProviderError::InvalidResponse("missing choices".into());
        assert_eq!(
            err.to_string(),
            "invalid provider response: missing choices"
        );
    }

    #[test]
    fn loop_error_display() {
        let err = LoopError::MaxTurns {
            max_turns: 10,
            partial: None,
        };
        assert_eq!(err.to_string(), "max turns exceeded (10)");
    }

    #[test]
    fn parse_and_tool_errors_are_backfill_only() {
        let parse = ParseError::InvalidJson("trailing comma".into());
        assert_eq!(parse.to_string(), "invalid JSON: trailing comma");

        let tool = ToolError::Unknown {
            name: "calculator".into(),
        };
        assert_eq!(tool.to_string(), "unknown tool: calculator");
    }

    #[test]
    fn strata_error_from_provider_and_loop() {
        let provider: StrataError =
            ProviderError::Auth("invalid api key".into()).into();
        assert!(matches!(provider, StrataError::Provider(_)));

        let loop_err: StrataError = LoopError::MaxTurns {
            max_turns: 5,
            partial: Some("partial".into()),
        }
        .into();
        assert!(matches!(loop_err, StrataError::Loop(_)));
    }
}
