//! DeepSeek [`Provider`](crate::Provider) — OpenAI-compatible blocking HTTP.
//!
//! Official config (OpenAI path): base `https://api.deepseek.com`, models `deepseek-v4-flash`
//! / `deepseek-v4-pro`. Legacy `deepseek-chat` / `deepseek-reasoner` deprecated 2026-07-24.

use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::message::{ContentBlock, Message, Role};
use crate::provider::{CompletionRequest, CompletionResponse, Provider};
use crate::tool::ToolSchema;

/// OpenAI-compatible API base (not the Anthropic `/anthropic` path).
pub const DEFAULT_API_BASE: &str = "https://api.deepseek.com";
/// Recommended default per DeepSeek docs (v4 flash).
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// DeepSeek provider using `reqwest::blocking` and OpenAI-compatible chat completions.
pub struct DeepSeekProvider {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
    /// Thinking mode toggle. MVP default `false` (non-thinking): faster, cheaper,
    /// deterministic; no `reasoning_content`. Opt in via `DEEPSEEK_THINKING`.
    thinking: bool,
}

impl DeepSeekProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            api_key: api_key.into(),
            base_url: DEFAULT_API_BASE.into(),
            model: DEFAULT_MODEL.into(),
            thinking: false,
        }
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let api_key = std::env::var("DEEPSEEK_API_KEY").map_err(|_| {
            ProviderError::Auth("DEEPSEEK_API_KEY not set".into())
        })?;
        let base_url = std::env::var("DEEPSEEK_API_BASE")
            .unwrap_or_else(|_| DEFAULT_API_BASE.into());
        let model =
            std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let thinking = std::env::var("DEEPSEEK_THINKING")
            .map(|s| {
                matches!(
                    s.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "enabled" | "on"
                )
            })
            .unwrap_or(false);

        Ok(Self {
            client: reqwest::blocking::Client::new(),
            api_key,
            base_url,
            model,
            thinking,
        })
    }

    /// Builds the chat-completions URL, tolerating a `base_url` that already ends
    /// with `/` or `/v1` (the `/v1` is OpenAI-compat, unrelated to model version).
    fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}{CHAT_COMPLETIONS_PATH}")
    }
}

impl Provider for DeepSeekProvider {
    fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let api_messages = encode_messages(&req.messages)?;
        let body = ApiChatRequest {
            model: self.model.clone(),
            messages: api_messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tools: encode_tools(&req.tools),
            thinking: Some(ApiThinking {
                kind: if self.thinking { "enabled" } else { "disabled" }.into(),
            }),
            reasoning_effort: if self.thinking {
                Some("high".into())
            } else {
                None
            },
        };

        let response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        parse_http_response(response)
    }
}

// --- OpenAI-compatible API types (private to this impl) ---

#[derive(Debug, Serialize)]
struct ApiChatRequest {
    model: String,
    messages: Vec<ApiMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ApiThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

/// OpenAI-format thinking toggle: `{"type": "enabled" | "disabled"}`.
#[derive(Debug, Serialize)]
struct ApiThinking {
    #[serde(rename = "type")]
    kind: String,
}

// Note: a thinking-mode response also carries `reasoning_content`, which is deliberately
// NOT declared here — serde ignores the unknown field on decode, so the reasoning trace is
// dropped and never echoed back to the API (verified by `decode_drops_reasoning_content`).
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ApiToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ApiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ApiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ApiTool {
    #[serde(rename = "type")]
    kind: String,
    function: ApiToolFunction,
}

#[derive(Debug, Serialize)]
struct ApiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ApiChatResponse {
    choices: Vec<ApiChoice>,
}

#[derive(Debug, Deserialize)]
struct ApiChoice {
    message: ApiMessage,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    error: Option<ApiErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: Option<String>,
}

fn encode_tools(tools: &[ToolSchema]) -> Option<Vec<ApiTool>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|t| ApiTool {
                kind: "function".into(),
                function: ApiToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect(),
    )
}

/// Pure translation of the unified history into API messages (no truncation).
/// Context compaction is deferred to M5 (decision 4) so tool_call/tool_result
/// pairing is never broken here.
fn encode_messages(messages: &[Message]) -> Result<Vec<ApiMessage>, ProviderError> {
    messages.iter().map(encode_message).collect()
}

fn encode_message(message: &Message) -> Result<ApiMessage, ProviderError> {
    match message.role {
        Role::System | Role::User => {
            let text = message.text();
            if text.is_empty() {
                return Err(ProviderError::InvalidResponse(format!(
                    "empty text for {:?} message",
                    message.role
                )));
            }
            Ok(ApiMessage {
                role: role_to_api(message.role),
                content: Some(text),
                tool_calls: None,
                tool_call_id: None,
            })
        }
        Role::Assistant => {
            let text = message.text();
            let tool_calls: Vec<ApiToolCall> = message
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolCall { id, name, args } => Some(ApiToolCall {
                        id: id.clone(),
                        kind: "function".into(),
                        function: ApiFunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(args).unwrap_or_else(|_| {
                                "{}".into()
                            }),
                        },
                    }),
                    _ => None,
                })
                .collect();

            Ok(ApiMessage {
                role: "assistant".into(),
                content: if text.is_empty() {
                    None
                } else {
                    Some(text)
                },
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
            })
        }
        Role::Tool => {
            let block = message.content.first().ok_or_else(|| {
                ProviderError::InvalidResponse("tool message has no content".into())
            })?;
            let (id, content) = match block {
                ContentBlock::ToolResult { id, content, .. } => (id.clone(), content.clone()),
                _ => {
                    return Err(ProviderError::InvalidResponse(
                        "tool message must contain ToolResult".into(),
                    ));
                }
            };
            Ok(ApiMessage {
                role: "tool".into(),
                content: Some(content),
                tool_calls: None,
                tool_call_id: Some(id),
            })
        }
    }
}

fn decode_assistant_message(api: ApiMessage) -> Result<Message, ProviderError> {
    if api.role != "assistant" {
        return Err(ProviderError::InvalidResponse(format!(
            "expected assistant message, got {}",
            api.role
        )));
    }

    // Thinking-mode `reasoning_content` was already dropped at deserialize time (see ApiMessage):
    // it is not part of the final answer and must not be echoed back on later turns.
    let mut blocks = Vec::new();
    if let Some(text) = api.content.filter(|s| !s.is_empty()) {
        blocks.push(ContentBlock::Text(text));
    }
    if let Some(calls) = api.tool_calls {
        for call in calls {
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| {
                    serde_json::Value::String(call.function.arguments.clone())
                });
            blocks.push(ContentBlock::ToolCall {
                id: call.id,
                name: call.function.name,
                args,
            });
        }
    }

    if blocks.is_empty() {
        return Err(ProviderError::InvalidResponse(
            "assistant message has no content or tool_calls".into(),
        ));
    }

    Ok(Message::assistant(blocks))
}

fn role_to_api(role: Role) -> String {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
    .into()
}

fn parse_http_response(
    response: reqwest::blocking::Response,
) -> Result<CompletionResponse, ProviderError> {
    let status = response.status();
    let body = response
        .text()
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ProviderError::Auth(api_error_message(&body)));
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(ProviderError::RateLimit(api_error_message(&body)));
    }
    if !status.is_success() {
        return Err(ProviderError::Network(format!(
            "HTTP {}: {}",
            status.as_u16(),
            truncate_body(&body, 512)
        )));
    }

    let parsed: ApiChatResponse = serde_json::from_str(&body).map_err(|e| {
        ProviderError::InvalidResponse(format!("failed to parse response JSON: {e}"))
    })?;

    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ProviderError::InvalidResponse("missing choices".into()))?;

    let message = decode_assistant_message(choice.message)?;
    Ok(CompletionResponse { message })
}

fn api_error_message(body: &str) -> String {
    serde_json::from_str::<ApiErrorBody>(body)
        .ok()
        .and_then(|b| b.error.and_then(|e| e.message))
        .unwrap_or_else(|| truncate_body(body, 256))
}

fn truncate_body(body: &str, max: usize) -> String {
    if body.len() <= max {
        body.to_string()
    } else {
        format!("{}…", &body[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_user_message() {
        let api = encode_message(&Message::user("hello")).expect("encode");
        assert_eq!(api.role, "user");
        assert_eq!(api.content.as_deref(), Some("hello"));
    }

    #[test]
    fn decode_text_response() {
        let msg = decode_assistant_message(ApiMessage {
            role: "assistant".into(),
            content: Some("hi there".into()),
            tool_calls: None,
            tool_call_id: None,
        })
        .expect("decode");
        assert_eq!(msg.text(), "hi there");
        assert!(!msg.has_tool_calls());
    }

    #[test]
    fn decode_drops_reasoning_content() {
        // A thinking-mode response carries `reasoning_content`; it must deserialize fine
        // (unknown field ignored) and not leak into the unified message.
        let json = r#"{"role":"assistant","content":"final answer","reasoning_content":"step by step..."}"#;
        let api: ApiMessage = serde_json::from_str(json).expect("deserialize");
        let msg = decode_assistant_message(api).expect("decode");
        assert_eq!(msg.text(), "final answer");
    }

    #[test]
    fn decode_tool_calls_response() {
        let msg = decode_assistant_message(ApiMessage {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ApiToolCall {
                id: "call_1".into(),
                kind: "function".into(),
                function: ApiFunctionCall {
                    name: "calculator".into(),
                    arguments: r#"{"expression":"1+2"}"#.into(),
                },
            }]),
            tool_call_id: None,
        })
        .expect("decode");
        assert!(msg.has_tool_calls());
    }

    #[test]
    fn defaults_match_official_docs() {
        assert_eq!(DEFAULT_API_BASE, "https://api.deepseek.com");
        assert_eq!(DEFAULT_MODEL, "deepseek-v4-flash");
    }

    #[test]
    fn chat_url_strips_trailing_slash_and_v1() {
        let cases = [
            "https://api.deepseek.com",
            "https://api.deepseek.com/",
            "https://api.deepseek.com/v1",
            "https://api.deepseek.com/v1/",
        ];
        for base in cases {
            let provider = DeepSeekProvider {
                client: reqwest::blocking::Client::new(),
                api_key: "k".into(),
                base_url: base.into(),
                model: DEFAULT_MODEL.into(),
                thinking: false,
            };
            assert_eq!(
                provider.chat_url(),
                "https://api.deepseek.com/v1/chat/completions",
                "base={base}"
            );
        }
    }
}
