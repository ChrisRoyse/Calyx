//! Research-chat surface: a DeepSeek reasoning agent that agentically drives Calyx
//! queries over the canonical Cuyahoga Eighth District + Ohio Supreme Court corpus
//! and returns a fully grounded, cited answer.
//!
//! Wire shape: a single streaming `POST /api/chat` returning `text/event-stream`
//! (see [`event`] for the justification). When chat is disabled the same route returns
//! the typed `CALYX_LAWDEMO_CHAT_DISABLED` envelope. No authority is ever written; all
//! tools are read-only.

pub mod agent;
pub mod citations;
pub mod deepseek;
pub mod event;
pub mod session;
pub mod tools;

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::response::sse::{KeepAlive, Sse};
use axum::{Json, extract::State, response::IntoResponse, response::Response};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::error::{ApiError, ErrorCode};
use crate::live::ValidatedJson;

pub use agent::{Budget, EpisodeOutcome, PROMPT_VERSION, SYSTEM_PROMPT, run_episode};
pub use session::{CancelReason, REAP_INTERVAL, reap};

/// Body for `POST /api/chat`.
#[derive(Debug, Deserialize)]
pub struct ChatBody {
    /// The free-form user question.
    pub question: String,
}

/// Body for `POST /api/chat/cancel`: the `episode_id` from the `opened` SSE event.
#[derive(Debug, Deserialize)]
pub struct CancelBody {
    /// The server-assigned id echoed back to stop a running episode.
    pub episode_id: String,
}

/// Query for `GET /api/chat/stream` — the REATTACH route (#2061).
#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    /// The episode to rejoin.
    pub episode_id: String,
    /// The last `seq` the client already holds. Omitted/0 means "from the beginning".
    /// The standard SSE `Last-Event-ID` request header is honoured as an equivalent,
    /// so the browser's own resume convention and this explicit parameter agree.
    pub from_seq: Option<u64>,
}

/// Non-secret chat info for `/api/health` (NEVER the api key).
pub fn health_info(cfg: &crate::config::Config) -> serde_json::Value {
    match &cfg.chat {
        None => json!({ "enabled": false }),
        Some(chat) => json!({
            "enabled": true,
            "model": chat.model,
            "base_url": chat.base_url,
            "prompt_version": PROMPT_VERSION,
            "prompt_sha256": prompt_sha256(),
            "budget": {
                "max_tool_calls": chat.max_tool_calls,
                "max_turns": chat.max_turns,
                "max_tokens": chat.max_tokens,
                "wall_timeout_secs": chat.wall_timeout.as_secs(),
                "search_k_max": chat.search_k_max,
            },
            "tools": tools::TOOL_NAMES,
            // The resume contract (#2061). The console reads these to drive its
            // reattach loop, so the client's retry budget is derived from the server's
            // real grace period instead of a number duplicated in the page.
            "resume": {
                "route": "/api/chat/stream",
                "query": "episode_id&from_seq",
                "client_grace_secs": chat.client_grace.as_secs(),
                "retention_secs": chat.episode_retention.as_secs(),
                "replay_max_bytes": chat.replay_max_bytes,
            },
            "episodes": session::health_counts(),
        }),
    }
}

/// SHA-256 of the embedded system prompt (provenance for live prompt iteration).
pub fn prompt_sha256() -> String {
    let digest = Sha256::digest(SYSTEM_PROMPT.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// `POST /api/chat`. Streams the episode as SSE, or returns the disabled envelope.
pub async fn chat_handler(
    State(state): State<AppState>,
    body: Result<ValidatedJson<ChatBody>, ApiError>,
) -> Response {
    // Disabled: the typed envelope, regardless of body validity.
    let Some(chat) = state.config.chat.clone() else {
        return ApiError::of(ErrorCode::ChatDisabled).into_response();
    };
    // Enabled: now the body must be well-formed.
    let ValidatedJson(body) = match body {
        Ok(b) => b,
        Err(err) => return err.into_response(),
    };
    let question = body.question.trim().to_owned();
    if question.is_empty() {
        return ApiError::new(
            ErrorCode::BadRequest,
            "question must not be empty".to_owned(),
        )
        .into_response();
    }
    if question.len() > 4000 {
        return ApiError::new(
            ErrorCode::BadRequest,
            "question exceeds 4000 bytes".to_owned(),
        )
        .into_response();
    }

    let cfg = state.config.clone();
    let budget = Budget {
        max_tool_calls: chat.max_tool_calls,
        max_turns: chat.max_turns,
        max_tokens: chat.max_tokens,
        wall_timeout: chat.wall_timeout,
    };

    // Open the episode in the registry BEFORE anything can be published or cancelled,
    // so a cancel that races the first token still lands. The id is announced to the
    // client via the leading `opened` event, which is also seq 1 of the replay log —
    // a reattaching client therefore always relearns its own id from the stream.
    let (episode_id, cancel_rx) = session::open(chat.replay_max_bytes);
    session::publish(
        &episode_id,
        event::ChatEvent::Opened {
            episode_id: episode_id.clone(),
        },
    );

    let task_id = episode_id.clone();
    tokio::spawn(async move {
        let transport: Arc<dyn deepseek::ChatTransport> = Arc::new(deepseek::UreqTransport::new(
            chat.base_url.clone(),
            chat.api_key.clone(),
            chat.request_timeout,
        ));
        let runner = tools::CalyxToolRunner::new(cfg);
        use event::EventSink;
        let sink = session::RegistrySink::new(task_id.clone());

        let episode = run_episode(transport, &runner, &chat.model, &question, budget, &sink);
        tokio::select! {
            // Natural completion: `run_episode` already emitted its own terminal
            // `done` (or `error`) event.
            _ = episode => {}
            // Cancel signalled — by the user's Stop / New chat, or by the reaper
            // because no client has been attached for the grace period. DROP the
            // episode future (which kills the in-flight tool subprocess via
            // kill_on_drop and detaches the blocking DeepSeek request) and emit a
            // typed terminal `cancelled` + `done` naming the reason.
            reason = cancel_rx => {
                let reason = reason.unwrap_or(CancelReason::UserRequest);
                sink.emit(event::ChatEvent::Cancelled {
                    note: reason.note().to_owned(),
                });
                sink.emit(event::ChatEvent::Done);
            }
        }
        // Belt and braces: an attached stream must ALWAYS end on a terminal frame.
        session::finish_if_running(&task_id);
    });

    // The response is just an attachment at seq 0 — the exact same path a reattach
    // takes, so a resumed stream can never drift from a first connection.
    match session::attach(&episode_id, 0) {
        Ok(attachment) => sse_response(attachment),
        Err(err) => {
            tracing::error!(
                episode = %episode_id,
                "CALYX_LAWDEMO_INTERNAL: could not attach to the episode just opened"
            );
            err.into_response()
        }
    }
}

/// `GET /api/chat/stream?episode_id=…&from_seq=…` — REJOIN a running or recently
/// finished episode (#2061).
///
/// This is what makes a backgrounded tab survivable. A frozen or discarded page, a
/// dropped tunnel, or a sleeping laptop breaks the response body, not the episode; the
/// client comes back here with the last `seq` it holds and receives every missed event
/// followed by the live tail. `Last-Event-ID` is honoured so the resume point can come
/// from the SSE convention as well as the explicit parameter.
///
/// Fails closed: an unknown/aged-out id is `CALYX_LAWDEMO_EPISODE_UNKNOWN`, and a
/// resume point the bounded log no longer covers is
/// `CALYX_LAWDEMO_EPISODE_REPLAY_TRUNCATED`. A partial episode is never dressed up as
/// a complete one.
pub async fn chat_stream_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<StreamQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if state.config.chat.is_none() {
        return ApiError::of(ErrorCode::ChatDisabled).into_response();
    }
    let Ok(Query(query)) = query else {
        return ApiError::new(
            ErrorCode::BadRequest,
            "episode_id is required (and from_seq, when given, must be a whole number)".to_owned(),
        )
        .into_response();
    };
    let episode_id = query.episode_id.trim();
    if episode_id.is_empty() {
        return ApiError::new(
            ErrorCode::BadRequest,
            "episode_id must not be empty".to_owned(),
        )
        .into_response();
    }
    // Explicit `from_seq` wins; otherwise fall back to the SSE `Last-Event-ID` header.
    // A malformed header is a client defect, so it is refused rather than silently
    // treated as 0 — replaying an entire episode over a typo would look like success.
    let from_seq = match query.from_seq {
        Some(n) => n,
        None => match headers.get("last-event-id").map(|v| v.to_str()) {
            None => 0,
            Some(Ok(raw)) if raw.trim().is_empty() => 0,
            Some(Ok(raw)) => match raw.trim().parse::<u64>() {
                Ok(n) => n,
                Err(_) => {
                    return ApiError::new(
                        ErrorCode::BadRequest,
                        "Last-Event-ID must be the whole-number seq of the last event received"
                            .to_owned(),
                    )
                    .into_response();
                }
            },
            Some(Err(_)) => {
                return ApiError::new(
                    ErrorCode::BadRequest,
                    "Last-Event-ID must be ASCII".to_owned(),
                )
                .into_response();
            }
        },
    };

    match session::attach(episode_id, from_seq) {
        Ok(attachment) => sse_response(attachment),
        Err(err) => err.into_response(),
    }
}

/// Carrier for the attachment while the SSE stream is being unfolded.
struct SseState {
    replay: std::vec::IntoIter<Arc<session::Frame>>,
    live: tokio::sync::broadcast::Receiver<Arc<session::Frame>>,
    /// Held for the life of the stream: dropping it (including when axum drops the
    /// response body on client disconnect) is what starts the abandonment clock.
    _guard: session::AttachGuard,
    /// Set once a terminal frame has been delivered, so the stream closes cleanly.
    finished: bool,
}

/// Turn an attachment into the SSE response: replay first, then the live tail, ending
/// on the terminal frame.
fn sse_response(attachment: session::Attachment) -> Response {
    let state = SseState {
        replay: attachment.replay.into_iter(),
        live: attachment.live,
        _guard: attachment.guard,
        finished: false,
    };
    let stream = futures_util::stream::unfold(state, |mut st| async move {
        if st.finished {
            return None;
        }
        if let Some(frame) = st.replay.next() {
            st.finished = frame.terminal;
            return Some((Ok::<_, Infallible>(frame.to_sse()), st));
        }
        match st.live.recv().await {
            Ok(frame) => {
                st.finished = frame.terminal;
                Some((Ok(frame.to_sse()), st))
            }
            // This attachment fell further behind than the live fan-out holds. The
            // frames are NOT lost — they are in the replay log — but this stream can no
            // longer be completed correctly, so it ends here and the client reattaches
            // from its own last seq. Loud, because a healthy client never lags this far.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::error!(
                    skipped,
                    "CALYX_LAWDEMO_INTERNAL: an attached SSE client lagged past the live \
                     fan-out; ending the stream so it reattaches from its replay point"
                );
                None
            }
            // The episode was evicted from the registry. Nothing further can be served;
            // a reattach will correctly report EPISODE_UNKNOWN.
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `POST /api/chat/cancel`. Stops a running episode by `episode_id` (the value from the
/// leading `opened` SSE event). Idempotent and config-free: it never touches the chat
/// config or the DeepSeek key — it only fires an in-memory one-shot. Returns a small
/// JSON acknowledgement (`cancelled: true` when an active episode was signalled,
/// `false` when the id is unknown or the episode already finished).
pub async fn chat_cancel_handler(body: Result<ValidatedJson<CancelBody>, ApiError>) -> Response {
    let ValidatedJson(body) = match body {
        Ok(b) => b,
        Err(err) => return err.into_response(),
    };
    let episode_id = body.episode_id.trim().to_owned();
    if episode_id.is_empty() {
        return ApiError::new(
            ErrorCode::BadRequest,
            "episode_id must not be empty".to_owned(),
        )
        .into_response();
    }
    let outcome = session::cancel(&episode_id, CancelReason::UserRequest);
    Json(json!({
        "episode_id": episode_id,
        // Retained for wire compatibility with the existing console contract.
        "cancelled": outcome == session::CancelOutcome::Signalled,
        // The three cases the old boolean collapsed into one. A client that pressed
        // Stop needs to tell "I stopped it" from "it had already finished" from "that
        // episode is gone", because only the last one is a reason to complain.
        "outcome": outcome.as_str(),
        "note": match outcome {
            session::CancelOutcome::Signalled =>
                "cancel signal delivered; the episode will emit a terminal `cancelled` event",
            session::CancelOutcome::AlreadyFinished =>
                "that episode had already reached its terminal event; nothing to stop",
            session::CancelOutcome::Unknown =>
                "no episode with that id is held by this server (never opened, or aged out)",
        },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_sha256_is_stable_hex() {
        let h = prompt_sha256();
        assert_eq!(h.len(), 64);
        assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn health_info_hides_key_when_enabled() {
        // Build an enabled config and confirm the health info never contains the key.
        let mut map = crate::config::base_env_map();
        map.insert("LAWDEMO_WEB_CHAT_ENABLED".into(), "1".into());
        map.insert("DEEPSEEK_API_KEY".into(), "sk-SECRET-DO-NOT-LEAK".into());
        map.insert(
            "LEGAL_DEEPSEEK_BASE_URL".into(),
            "https://api.deepseek.com".into(),
        );
        map.insert("LEGAL_DEEPSEEK_MODEL".into(), "deepseek-v4-flash".into());
        let cfg = crate::config::Config::from_map(&map).unwrap();
        let info = health_info(&cfg);
        let serialized = serde_json::to_string(&info).unwrap();
        assert!(
            !serialized.contains("sk-SECRET"),
            "api key leaked into health info"
        );
        assert_eq!(info["enabled"], true);
        assert_eq!(info["model"], "deepseek-v4-flash");
        assert_eq!(info["prompt_version"], PROMPT_VERSION);
    }

    /// The console reads `chat_contract` out of `/api/health` to learn the reattach
    /// route, so the contract must name the stream endpoint and its resume parameter.
    #[test]
    fn health_info_publishes_the_reattach_contract() {
        let mut map = crate::config::base_env_map();
        map.insert("LAWDEMO_WEB_CHAT_ENABLED".into(), "1".into());
        map.insert("DEEPSEEK_API_KEY".into(), "sk-test".into());
        map.insert(
            "LEGAL_DEEPSEEK_BASE_URL".into(),
            "https://api.deepseek.com".into(),
        );
        map.insert("LEGAL_DEEPSEEK_MODEL".into(), "deepseek-v4-flash".into());
        let cfg = crate::config::Config::from_map(&map).unwrap();
        let info = health_info(&cfg);
        assert_eq!(info["resume"]["route"], "/api/chat/stream");
        assert_eq!(info["resume"]["query"], "episode_id&from_seq");
        // The grace period is what the console must beat when it reattaches, so it is
        // published rather than hidden in the server's config.
        assert_eq!(info["resume"]["client_grace_secs"], 90);
        assert_eq!(info["resume"]["retention_secs"], 900);
    }

    /// A cancel body naming an episode this server does not hold must be reported as
    /// `unknown` — never as a successful stop, which would let the console tell the
    /// user it stopped something it did not.
    #[tokio::test]
    async fn cancelling_an_unknown_episode_reports_unknown() {
        let outcome = session::cancel("ep-not-a-real-episode", CancelReason::UserRequest);
        assert_eq!(outcome, session::CancelOutcome::Unknown);
        assert_eq!(outcome.as_str(), "unknown");
    }
}
