//! DeepSeek OpenAI-compatible chat-completions client + the transport seam.
//!
//! PROVIDER CONTRACT (verified 2026-07-23 against DeepSeek's current official docs):
//!   * `deepseek-reasoner` is deprecated 2026-07-24 and maps to
//!     `deepseek-v4-flash` with thinking enabled.
//!   * `POST /chat/completions` with a `tools` array → the reasoning model returns
//!     `finish_reason:"tool_calls"` with a native `tool_calls[]` array. **Native
//!     function-calling is supported on the reasoning model**, so this client uses it
//!     rather than a hand-rolled JSON action protocol.
//!   * The model emits `reasoning_content` on every turn and can request MULTIPLE
//!     tool calls in one turn (observed 3 parallel calls). `usage.completion_tokens_
//!     details.reasoning_tokens` is populated.
//!   * In thinking mode, assistant turns that perform tool calls MUST preserve their
//!     full `reasoning_content` in every subsequent request; DeepSeek documents a 400
//!     response when it is omitted.
//!
//! The [`ChatTransport`] trait is the test seam: production uses [`UreqTransport`];
//! unit tests drive the loop with recorded real responses (see `tests/fixtures`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A message as sent TO the API. `reasoning_content` is populated only for assistant
/// tool-call turns, exactly as required by DeepSeek's thinking-mode contract.
#[derive(Debug, Clone, Serialize)]
pub struct OutgoingMessage {
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl OutgoingMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system",
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
    /// An assistant turn. `content` is always sent (empty string is valid alongside
    /// tool calls). Thinking content is retained exactly when the turn has tool calls.
    pub fn assistant(
        content: String,
        reasoning_content: Option<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        let has_tool_calls = !tool_calls.is_empty();
        Self {
            role: "assistant",
            content: Some(content),
            reasoning_content: has_tool_calls.then_some(reasoning_content).flatten(),
            tool_calls: if has_tool_calls {
                Some(tool_calls)
            } else {
                None
            },
            tool_call_id: None,
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool",
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A function/tool call, used both when parsed from a response and when echoed back
/// into the next request's assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub index: Option<u32>,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// The model emits arguments as a JSON *string* (OpenAI convention).
    pub arguments: String,
}

/// The request body for `POST /chat/completions`.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<OutgoingMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    pub max_tokens: u32,
    pub reasoning_effort: &'static str,
    pub thinking: ThinkingMode,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThinkingMode {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

/// A decoded chat-completions response (only the fields the loop needs).
#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub finish_reason: Option<String>,
    pub message: ResponseMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub completion_tokens_details: Option<TokenDetails>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TokenDetails {
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl ChatResponse {
    /// The first choice's message, or `None` for an empty-choices response.
    pub fn first(&self) -> Option<&Choice> {
        self.choices.first()
    }
    pub fn reasoning_tokens(&self) -> u64 {
        self.usage
            .as_ref()
            .and_then(|u| u.completion_tokens_details.as_ref())
            .map(|d| d.reasoning_tokens)
            .unwrap_or(0)
    }
}

/// A transport failure. `Upstream` carries the API's own status + best-effort body so
/// the honest error envelope can forward what the provider actually said.
#[derive(Debug)]
pub enum TransportError {
    /// The provider returned a non-2xx HTTP status.
    Upstream { status: u16, body: String },
    /// The request never completed (DNS/TLS/timeout/connection).
    Network(String),
    /// A 2xx body that could not be decoded into [`ChatResponse`].
    Decode(String),
}

impl TransportError {
    /// A single-line, secret-free summary safe to log and surface.
    pub fn summary(&self) -> String {
        match self {
            Self::Upstream { status, body } => {
                let mut b = body.clone();
                b.truncate(600);
                format!("deepseek HTTP {status}: {b}")
            }
            Self::Network(e) => format!("deepseek network error: {e}"),
            Self::Decode(e) => format!("deepseek response decode error: {e}"),
        }
    }
}

/// The seam the agent loop calls. Blocking (ureq is blocking); the loop invokes it via
/// `spawn_blocking`. `Send + Sync + 'static` so it can live behind an `Arc` in state.
pub trait ChatTransport: Send + Sync + 'static {
    fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, TransportError>;
}

/// Production transport: a blocking `ureq` POST to `{base_url}/chat/completions`.
pub struct UreqTransport {
    base_url: String,
    api_key: String,
    timeout: std::time::Duration,
}

impl UreqTransport {
    pub fn new(base_url: String, api_key: String, timeout: std::time::Duration) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            timeout,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

impl ChatTransport for UreqTransport {
    fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, TransportError> {
        let body = serde_json::to_vec(req)
            .map_err(|e| TransportError::Network(format!("request serialize: {e}")))?;
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            // Keep the response object for 4xx/5xx so the provider's actionable,
            // secret-free JSON error body is not discarded as a bare status code.
            .http_status_as_error(false)
            .build();
        let agent: ureq::Agent = config.into();
        let result = agent
            .post(self.endpoint())
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send(&body[..]);
        match result {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let text = resp
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| TransportError::Decode(format!("read body: {e}")))?;
                if !(200..300).contains(&status) {
                    return Err(TransportError::Upstream {
                        status,
                        body: truncate(&text, 600),
                    });
                }
                serde_json::from_str::<ChatResponse>(&text)
                    .map_err(|e| TransportError::Decode(format!("{e}: {}", truncate(&text, 400))))
            }
            Err(other) => Err(TransportError::Network(other.to_string())),
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    let mut out = s.to_owned();
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outgoing_assistant_tool_turn_serializes_reasoning() {
        let msg = OutgoingMessage::assistant(
            "".into(),
            Some("I need to search the corpus.".into()),
            vec![ToolCall {
                index: Some(0),
                id: "call_1".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "corpus_search".into(),
                    arguments: "{\"query\":\"x\"}".into(),
                },
            }],
        );
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["reasoning_content"], "I need to search the corpus.");
        assert_eq!(v["tool_calls"][0]["function"]["name"], "corpus_search");
        assert_eq!(v["tool_calls"][0]["type"], "function");
    }

    #[test]
    fn tool_message_carries_call_id() {
        let m = OutgoingMessage::tool("call_9", "{\"hits\":[]}");
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "call_9");
        assert!(v.get("tool_calls").is_none());
    }

    #[test]
    fn decodes_recorded_tool_call_response() {
        // The real recorded response: model requests a tool.
        let raw = include_str!("../../tests/fixtures/deepseek/chat_tool_call_turn.json");
        let resp: ChatResponse = serde_json::from_str(raw).expect("decode recorded response");
        let choice = resp.first().expect("a choice");
        assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
        let calls = choice.message.tool_calls.as_ref().expect("tool_calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "corpus_search");
        // reasoning_content is present and must be re-sent with this tool-call turn.
        assert!(choice.message.reasoning_content.is_some());
    }

    #[test]
    fn decodes_recorded_parallel_tool_calls() {
        let raw = include_str!("../../tests/fixtures/deepseek/chat_parallel_tool_calls.json");
        let resp: ChatResponse = serde_json::from_str(raw).expect("decode");
        let calls = resp
            .first()
            .unwrap()
            .message
            .tool_calls
            .as_ref()
            .expect("tool_calls");
        // The real model requested THREE tools in a single turn.
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn decodes_recorded_final_answer() {
        let raw = include_str!("../../tests/fixtures/deepseek/chat_final_answer.json");
        let resp: ChatResponse = serde_json::from_str(raw).expect("decode");
        let choice = resp.first().unwrap();
        assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
        assert!(choice.message.tool_calls.is_none());
        let content = choice.message.content.as_deref().unwrap_or_default();
        assert!(content.contains("Direct Answer"));
        assert!(content.contains("[cx:"));
        assert!(resp.reasoning_tokens() > 0);
    }
}
