//! Closed error envelope for `calyx-lawdemo-web`.
//!
//! Copies the guardrails pattern of `crates/calyx-web-api/src/error.rs`: a
//! CLOSED [`ErrorCode`] enum, each with a stable wire string, an HTTP status,
//! and a one-line operator remediation. EVERY non-success response is the same
//! `{code,message,remediation}` JSON envelope. Messages carry only static text
//! or an already-safe echoed request shape (a query id, an artifact index) —
//! never a secret, a raw filesystem path, or a panic payload — so the body is
//! safe verbatim. A panic is caught and rendered as a generic `Internal` 500.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// The closed catalog of error codes this service can emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// No route matched the request path.
    NotFound,
    /// The path exists, but not for the request method.
    MethodNotAllowed,
    /// The request carried a malformed or non-allowlisted value (an unknown
    /// query id, a bad artifact index, a malformed answer id). Fail loud.
    BadRequest,
    /// A configured artifact file is absent from the evidence dir.
    ArtifactMissing,
    /// A configured artifact's bytes did NOT match its pinned SHA-256. The
    /// service NEVER serves unverified bytes — this is a structured refusal.
    ArtifactUnverified,
    /// A live subprocess (the pinned `calyx` binary) exited non-zero. Its own
    /// structured stderr envelope is forwarded verbatim to the caller.
    LiveCommandFailed,
    /// A live subprocess exceeded its time budget and was aborted.
    LiveTimeout,
    /// Live mode was requested but the pinned binary is not configured/present.
    LiveUnavailable,
    /// The research chat surface is not enabled in this deployment.
    ChatDisabled,
    /// The DeepSeek reasoning API (or its transport) failed. Upstream forwarded.
    ChatUpstream,
    /// The reasoning model's final answer cited a cx_id that no tool returned. The
    /// answer is REFUSED, never rendered — the grounding differentiator, fail closed.
    ChatUngrounded,
    /// The per-episode tool-call / turn budget was exhausted.
    ChatBudget,
    /// The reasoning model emitted structurally unusable output (e.g. empty answer).
    ChatProtocol,
    /// The per-episode wall-clock budget elapsed.
    ChatTimeout,
    /// A tool call carried a malformed/invalid/out-of-allowlist argument.
    ChatToolArg,
    /// The model requested a tool that is not in the allowlisted registry.
    ChatToolUnknown,
    /// A reattach named an `episode_id` the registry does not hold: it never
    /// existed, or it finished and aged past the retention window. Fail loud —
    /// the client must say the episode is gone, never hang on a dead stream.
    EpisodeUnknown,
    /// A reattach asked to resume from a `seq` the bounded replay log no longer
    /// holds. The live tail is still deliverable, but the gap is REAL, so the
    /// request is refused rather than served as a silently incomplete episode.
    EpisodeReplayTruncated,
    /// An unhandled internal fault (including a caught panic). Never leaks detail.
    Internal,
}

impl ErrorCode {
    /// The complete closed catalog (for documentation + invariant tests).
    pub const ALL: [ErrorCode; 19] = [
        Self::NotFound,
        Self::MethodNotAllowed,
        Self::BadRequest,
        Self::ArtifactMissing,
        Self::ArtifactUnverified,
        Self::LiveCommandFailed,
        Self::LiveTimeout,
        Self::LiveUnavailable,
        Self::ChatDisabled,
        Self::ChatUpstream,
        Self::ChatUngrounded,
        Self::ChatBudget,
        Self::ChatProtocol,
        Self::ChatTimeout,
        Self::ChatToolArg,
        Self::ChatToolUnknown,
        Self::EpisodeUnknown,
        Self::EpisodeReplayTruncated,
        Self::Internal,
    ];

    /// Stable wire code. The SPA branches on this; its meaning never changes.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "CALYX_LAWDEMO_NOT_FOUND",
            Self::MethodNotAllowed => "CALYX_LAWDEMO_METHOD_NOT_ALLOWED",
            Self::BadRequest => "CALYX_LAWDEMO_BAD_REQUEST",
            Self::ArtifactMissing => "CALYX_LAWDEMO_ARTIFACT_MISSING",
            Self::ArtifactUnverified => "CALYX_LAWDEMO_ARTIFACT_UNVERIFIED",
            Self::LiveCommandFailed => "CALYX_LAWDEMO_LIVE_COMMAND_FAILED",
            Self::LiveTimeout => "CALYX_LAWDEMO_LIVE_TIMEOUT",
            Self::LiveUnavailable => "CALYX_LAWDEMO_LIVE_UNAVAILABLE",
            Self::ChatDisabled => "CALYX_LAWDEMO_CHAT_DISABLED",
            Self::ChatUpstream => "CALYX_LAWDEMO_CHAT_UPSTREAM",
            Self::ChatUngrounded => "CALYX_LAWDEMO_CHAT_UNGROUNDED",
            Self::ChatBudget => "CALYX_LAWDEMO_CHAT_BUDGET",
            Self::ChatProtocol => "CALYX_LAWDEMO_CHAT_PROTOCOL",
            Self::ChatTimeout => "CALYX_LAWDEMO_CHAT_TIMEOUT",
            Self::ChatToolArg => "CALYX_LAWDEMO_CHAT_TOOL_ARG",
            Self::ChatToolUnknown => "CALYX_LAWDEMO_CHAT_TOOL_UNKNOWN",
            Self::EpisodeUnknown => "CALYX_LAWDEMO_EPISODE_UNKNOWN",
            Self::EpisodeReplayTruncated => "CALYX_LAWDEMO_EPISODE_REPLAY_TRUNCATED",
            Self::Internal => "CALYX_LAWDEMO_INTERNAL",
        }
    }

    /// HTTP status this code maps to.
    pub const fn status(self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::BadRequest => StatusCode::BAD_REQUEST,
            // An unverifiable or absent artifact is a server-state fault, not a
            // client error: the caller asked for a valid demo, the deployment
            // cannot honour it safely.
            Self::ArtifactMissing => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ArtifactUnverified => StatusCode::INTERNAL_SERVER_ERROR,
            // A live upstream (the resident via the pinned binary) failed.
            Self::LiveCommandFailed => StatusCode::BAD_GATEWAY,
            Self::LiveTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::LiveUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::ChatDisabled => StatusCode::SERVICE_UNAVAILABLE,
            Self::ChatUpstream => StatusCode::BAD_GATEWAY,
            Self::ChatUngrounded => StatusCode::UNPROCESSABLE_ENTITY,
            Self::ChatBudget => StatusCode::TOO_MANY_REQUESTS,
            Self::ChatProtocol => StatusCode::BAD_GATEWAY,
            Self::ChatTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::ChatToolArg => StatusCode::BAD_REQUEST,
            Self::ChatToolUnknown => StatusCode::BAD_REQUEST,
            // The episode is genuinely absent from the registry.
            Self::EpisodeUnknown => StatusCode::NOT_FOUND,
            // The resource existed but the requested portion is permanently gone.
            Self::EpisodeReplayTruncated => StatusCode::GONE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// One-line operator remediation (every structured error carries one).
    pub const fn remediation(self) -> &'static str {
        match self {
            Self::NotFound => "check the request path against the documented /api route surface",
            Self::MethodNotAllowed => "use the documented method for this route",
            Self::BadRequest => "send an allowlisted value for the field named in the message",
            Self::ArtifactMissing => {
                "place the pinned artifact in LAWDEMO_WEB_EVIDENCE_DIR before serving artifact mode"
            }
            Self::ArtifactUnverified => {
                "the artifact bytes do not match the configured SHA-256 pin; re-verify the evidence source"
            }
            Self::LiveCommandFailed => {
                "inspect the forwarded upstream envelope; the resident or vault may be unreachable"
            }
            Self::LiveTimeout => "retry; if it persists the resident call is too slow or hung",
            Self::LiveUnavailable => {
                "set LAWDEMO_WEB_CALYX_BINARY to the pinned calyx binary and confirm the resident is up"
            }
            Self::ChatDisabled => {
                "set LAWDEMO_WEB_CHAT_ENABLED=1 and provide DEEPSEEK_API_KEY, LEGAL_DEEPSEEK_BASE_URL, LEGAL_DEEPSEEK_MODEL"
            }
            Self::ChatUpstream => {
                "inspect the forwarded upstream envelope; the DeepSeek API key/base URL or network may be at fault"
            }
            Self::ChatUngrounded => {
                "the model cited sources no tool returned; this is a fail-closed refusal, not a bug — retry the question"
            }
            Self::ChatBudget => {
                "raise LAWDEMO_WEB_CHAT_MAX_TOOL_CALLS/MAX_TURNS or narrow the question"
            }
            Self::ChatProtocol => {
                "retry; if it persists the reasoning model returned unusable output"
            }
            Self::ChatTimeout => "raise LAWDEMO_WEB_CHAT_WALL_TIMEOUT_SECS or narrow the question",
            Self::ChatToolArg => {
                "the model supplied an invalid tool argument; this is surfaced to the agent to self-correct"
            }
            Self::ChatToolUnknown => {
                "the model requested a tool outside the allowlist; surfaced to the agent to self-correct"
            }
            Self::EpisodeUnknown => {
                "the episode finished and aged out, or was never opened on this server; ask the question again"
            }
            Self::EpisodeReplayTruncated => {
                "the episode outgrew LAWDEMO_WEB_CHAT_REPLAY_MAX_BYTES so the missed events are gone; ask the question again"
            }
            Self::Internal => "retry; if it persists, inspect the calyx-lawdemo-web server logs",
        }
    }

    /// Default caller-facing message when no route-specific detail is supplied.
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::NotFound => "no route matches this request path",
            Self::MethodNotAllowed => "this route does not support the request method",
            Self::BadRequest => "the request carried a malformed or non-allowlisted value",
            Self::ArtifactMissing => "a configured demo artifact is absent from the evidence dir",
            Self::ArtifactUnverified => "a demo artifact failed its SHA-256 verification",
            Self::LiveCommandFailed => "the live calyx command exited with an error",
            Self::LiveTimeout => "the live calyx command exceeded its time budget",
            Self::LiveUnavailable => "live mode is not available in this deployment",
            Self::ChatDisabled => "the research chat surface is not enabled in this deployment",
            Self::ChatUpstream => "the DeepSeek reasoning API request failed",
            Self::ChatUngrounded => {
                "the answer cited sources that no tool returned and was refused rather than rendered"
            }
            Self::ChatBudget => "the chat episode exhausted its tool-call or turn budget",
            Self::ChatProtocol => "the reasoning model produced unusable output",
            Self::ChatTimeout => "the chat episode exceeded its wall-clock budget",
            Self::ChatToolArg => "a tool call carried an invalid argument",
            Self::ChatToolUnknown => "the model requested a tool outside the allowlist",
            Self::EpisodeUnknown => "no episode with that id is held by this server",
            Self::EpisodeReplayTruncated => {
                "the episode's replay log no longer covers the requested resume point"
            }
            Self::Internal => "an internal error occurred",
        }
    }
}

/// A structured API error: a closed [`ErrorCode`] plus a caller-facing message,
/// and an OPTIONAL forwarded upstream envelope. When `forwarded` is set (a live
/// subprocess emitted its own `{code,message,remediation}` JSON on stderr) the
/// upstream envelope is passed through verbatim under the `upstream` key rather
/// than being reworded — the SPA shows exactly what the accepted code path said.
#[derive(Debug, Clone)]
pub struct ApiError {
    code: ErrorCode,
    message: String,
    forwarded: Option<serde_json::Value>,
}

impl ApiError {
    /// Construct with an explicit, already-safe message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            forwarded: None,
        }
    }

    /// Construct with the code's default message.
    pub fn of(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.default_message().to_owned(),
            forwarded: None,
        }
    }

    /// Attach a verbatim upstream envelope forwarded from a live subprocess.
    pub fn with_upstream(mut self, upstream: serde_json::Value) -> Self {
        self.forwarded = Some(upstream);
        self
    }

    /// The code (for tests and internal branching).
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The JSON body this error renders (also used to compose responses).
    pub fn body(&self) -> serde_json::Value {
        let mut value = json!({
            "code": self.code.code(),
            "message": self.message,
            "remediation": self.code.remediation(),
        });
        if let Some(upstream) = &self.forwarded {
            value["upstream"] = upstream.clone();
        }
        value
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.code.status(), Json(self.body())).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_codes_have_distinct_wire_strings() {
        let mut seen = std::collections::HashSet::new();
        for code in ErrorCode::ALL {
            assert!(
                seen.insert(code.code()),
                "duplicate wire code: {}",
                code.code()
            );
            assert!(code.code().starts_with("CALYX_LAWDEMO_"));
            assert!(!code.remediation().is_empty());
            assert!(!code.default_message().is_empty());
        }
    }

    #[test]
    fn body_carries_the_closed_triple() {
        let body = ApiError::of(ErrorCode::BadRequest).body();
        assert_eq!(body["code"], "CALYX_LAWDEMO_BAD_REQUEST");
        assert!(body["message"].is_string());
        assert!(body["remediation"].is_string());
        assert!(body.get("upstream").is_none());
    }

    #[test]
    fn forwarded_upstream_is_passed_through_verbatim() {
        let upstream = json!({"code": "CALYX_KERNEL_QUERY_OUT_OF_SCOPE", "message": "x"});
        let body = ApiError::of(ErrorCode::LiveCommandFailed)
            .with_upstream(upstream.clone())
            .body();
        assert_eq!(body["upstream"], upstream);
    }
}
