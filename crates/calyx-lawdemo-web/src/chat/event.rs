//! The Server-Sent-Events wire schema for a chat episode, and the sink seam.
//!
//! SSE SHAPE DECISION: a single streaming `POST /api/chat` that returns
//! `text/event-stream`. Justification: one episode is one logical flow (reasoning →
//! tool mining → validated answer); a single request keeps ALL state ephemeral and
//! server-side-free (no session store, no start/connect race). The SPA reads it with
//! `fetch()` + a `ReadableStream` reader rather than `EventSource` (which is GET-only).
//! Every event is `event: <name>\n` + `data: <json>\n\n`.
//!
//! TRANSPARENCY: the schema carries the COMPLETE tool payload (never truncated on the
//! wire beyond the per-tool `max_bytes` the agent itself requested), the full
//! `reasoning_content`, per-step rationale, and the citation-validation verdict — so
//! the UI can display everything the system did.

use serde_json::{Value, json};

use crate::error::ApiError;

/// One event emitted during an episode. `name()` is the SSE `event:` field; `data()`
/// is the JSON `data:` payload.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// Transport-layer preamble emitted by [`crate::chat::chat_handler`] BEFORE the
    /// reasoning loop starts. Carries the server-assigned `episode_id` the client
    /// echoes back to `POST /api/chat/cancel` to stop a running episode. Kept separate
    /// from [`Self::EpisodeStart`] (which the agent loop owns) so cancellation wiring
    /// touches only the transport layer.
    Opened { episode_id: String },
    /// Episode opened: the question, the budget, and the prompt/model provenance.
    EpisodeStart {
        question: String,
        model: String,
        prompt_version: &'static str,
        max_tool_calls: u32,
        wall_timeout_secs: u64,
        max_tokens: u32,
    },
    /// The reasoning model's full chain-of-thought for one turn.
    Reasoning {
        turn: u32,
        text: String,
        reasoning_tokens: u64,
    },
    /// Non-empty assistant prose emitted alongside tool calls (per-step rationale).
    Rationale { turn: u32, text: String },
    /// The agent chose to run a tool. `verb` is a human sentence built from real args.
    ToolCall {
        turn: u32,
        index: usize,
        id: String,
        name: String,
        args: Value,
        verb: String,
    },
    /// A tool finished. Carries the COMPLETE payload plus a headline + real latency.
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        headline: String,
        latency_ms: u64,
        cx_ids: Vec<String>,
        payload: Value,
    },
    /// The citation-grounding verdict for the model's final answer.
    Validation {
        cited: Vec<String>,
        valid: Vec<String>,
        invalid: Vec<String>,
        verdict: &'static str,
    },
    /// The validated final answer (Markdown).
    Answer { text: String },
    /// The validated citations, with best-effort real display provenance.
    Citations { items: Vec<Value> },
    /// The reproducibility footer.
    Footer {
        tool_calls_used: u32,
        max_tool_calls: u32,
        wall_ms: u64,
        turns: u32,
        note: String,
    },
    /// A typed error/refusal envelope (same `{code,message,remediation,upstream}` shape
    /// as the rest of the service). Terminal.
    Error(Value),
    /// The episode was cancelled at the user's request (STOP). Terminal, distinct from
    /// [`Self::Error`] so the client renders a structured "stopped — retry" recovery
    /// affordance rather than a failure. `note` is static, secret-free text.
    Cancelled { note: String },
    /// Terminal marker so the client can close cleanly.
    Done,
}

impl ChatEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Opened { .. } => "opened",
            Self::EpisodeStart { .. } => "episode_start",
            Self::Reasoning { .. } => "reasoning",
            Self::Rationale { .. } => "rationale",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolResult { .. } => "tool_result",
            Self::Validation { .. } => "validation",
            Self::Answer { .. } => "answer",
            Self::Citations { .. } => "citations",
            Self::Footer { .. } => "footer",
            Self::Error(_) => "error",
            Self::Cancelled { .. } => "cancelled",
            Self::Done => "done",
        }
    }

    pub fn data(&self) -> Value {
        match self {
            Self::Opened { episode_id } => json!({ "episode_id": episode_id }),
            Self::EpisodeStart {
                question,
                model,
                prompt_version,
                max_tool_calls,
                wall_timeout_secs,
                max_tokens,
            } => json!({
                "question": question,
                "model": model,
                "prompt_version": prompt_version,
                "budget": {
                    "max_tool_calls": max_tool_calls,
                    "wall_timeout_secs": wall_timeout_secs,
                    "max_tokens": max_tokens,
                },
            }),
            Self::Reasoning {
                turn,
                text,
                reasoning_tokens,
            } => json!({ "turn": turn, "text": text, "reasoning_tokens": reasoning_tokens }),
            Self::Rationale { turn, text } => json!({ "turn": turn, "text": text }),
            Self::ToolCall {
                turn,
                index,
                id,
                name,
                args,
                verb,
            } => json!({
                "turn": turn, "index": index, "id": id,
                "name": name, "args": args, "verb": verb,
            }),
            Self::ToolResult {
                id,
                name,
                ok,
                headline,
                latency_ms,
                cx_ids,
                payload,
            } => json!({
                "id": id, "name": name, "ok": ok, "headline": headline,
                "latency_ms": latency_ms, "cx_ids": cx_ids, "payload": payload,
            }),
            Self::Validation {
                cited,
                valid,
                invalid,
                verdict,
            } => json!({
                "cited": cited, "valid": valid, "invalid": invalid, "verdict": verdict,
            }),
            Self::Answer { text } => json!({ "text": text }),
            Self::Citations { items } => json!({ "items": items }),
            Self::Footer {
                tool_calls_used,
                max_tool_calls,
                wall_ms,
                turns,
                note,
            } => json!({
                "tool_calls_used": tool_calls_used, "max_tool_calls": max_tool_calls,
                "wall_ms": wall_ms, "turns": turns, "note": note,
            }),
            Self::Error(env) => env.clone(),
            Self::Cancelled { note } => json!({ "note": note }),
            Self::Done => json!({}),
        }
    }

    /// Convenience: build a terminal error event from an [`ApiError`].
    pub fn error(err: &ApiError) -> Self {
        Self::Error(err.body())
    }

    /// Serialize as a full SSE frame: `event: <name>\ndata: <json>\n\n`.
    pub fn to_frame(&self) -> String {
        format!(
            "event: {}\ndata: {}\n\n",
            self.name(),
            serde_json::to_string(&self.data()).unwrap_or_else(|_| "{}".to_owned())
        )
    }
}

/// The emit seam. Production sends into an SSE channel; tests collect into a `Vec`.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: ChatEvent);
}

/// A test/inspection sink that records every emitted event.
#[derive(Default)]
pub struct CollectingSink {
    events: std::sync::Mutex<Vec<ChatEvent>>,
}

impl CollectingSink {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn events(&self) -> Vec<ChatEvent> {
        self.events.lock().expect("sink lock").clone()
    }
    /// Names of emitted events, in order (handy for asserting the episode shape).
    pub fn names(&self) -> Vec<&'static str> {
        self.events().iter().map(ChatEvent::name).collect()
    }
}

impl EventSink for CollectingSink {
    fn emit(&self, event: ChatEvent) {
        self.events.lock().expect("sink lock").push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_well_formed_sse() {
        let ev = ChatEvent::Answer {
            text: "hello".into(),
        };
        let frame = ev.to_frame();
        assert!(frame.starts_with("event: answer\ndata: "));
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains("\"text\":\"hello\""));
    }

    #[test]
    fn collecting_sink_records_in_order() {
        let sink = CollectingSink::new();
        sink.emit(ChatEvent::Done);
        sink.emit(ChatEvent::Answer { text: "x".into() });
        assert_eq!(sink.names(), vec!["done", "answer"]);
    }

    #[test]
    fn opened_frame_carries_episode_id() {
        let ev = ChatEvent::Opened {
            episode_id: "ep-7".into(),
        };
        assert_eq!(ev.name(), "opened");
        assert_eq!(ev.data()["episode_id"], "ep-7");
        let frame = ev.to_frame();
        assert!(frame.starts_with("event: opened\ndata: "));
        assert!(frame.contains("\"episode_id\":\"ep-7\""));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn cancelled_frame_is_well_formed_and_terminal_typed() {
        let ev = ChatEvent::Cancelled {
            note: "stopped by user".into(),
        };
        assert_eq!(ev.name(), "cancelled");
        assert_eq!(ev.data()["note"], "stopped by user");
        let frame = ev.to_frame();
        assert!(frame.starts_with("event: cancelled\ndata: "));
        assert!(frame.ends_with("\n\n"));
    }
}
