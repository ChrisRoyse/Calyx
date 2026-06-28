use super::*;

/// The closed catalog of error codes this service can emit. Mirrors the
/// `calyxd` `CALYX_*` convention: a stable wire string + an HTTP status + a
/// one-line operator remediation. CLOSED — adding a variant is a deliberate
/// API change (the catalog invariants are asserted in `tests/api.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// A scaffolded route not yet wired to `calyxd`.
    NotImplemented,
    /// No route matched the request path.
    NotFound,
    /// The path exists, but not for the request method.
    MethodNotAllowed,
    /// The request body exceeded the route's byte cap.
    PayloadTooLarge,
    /// The caller exceeded the route's rate limit.
    RateLimited,
    /// The request exceeded [`REQUEST_TIMEOUT`] (slow upstream aborted).
    Timeout,
    /// The request body was malformed or carried an invalid value (e.g. k=0,
    /// unknown fusion mode). Fail loud — never silently clamp/default.
    BadRequest,
    /// The request lacked a valid shared-secret bearer (fail-closed; the origin
    /// is never anonymous — #1906/#587).
    Unauthorized,
    /// An unhandled internal fault (including a caught panic). Never leaks detail.
    Internal,
}

impl ErrorCode {
    /// The complete closed catalog (for documentation + invariant tests).
    pub const ALL: [ErrorCode; 9] = [
        Self::NotImplemented,
        Self::NotFound,
        Self::MethodNotAllowed,
        Self::PayloadTooLarge,
        Self::RateLimited,
        Self::Unauthorized,
        Self::Timeout,
        Self::BadRequest,
        Self::Internal,
    ];

    /// Stable wire code. The edge client branches on this; its meaning never changes.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotImplemented => "CALYX_WEB_API_NOT_IMPLEMENTED",
            Self::NotFound => "CALYX_WEB_API_NOT_FOUND",
            Self::MethodNotAllowed => "CALYX_WEB_API_METHOD_NOT_ALLOWED",
            Self::PayloadTooLarge => "CALYX_WEB_API_PAYLOAD_TOO_LARGE",
            Self::RateLimited => "CALYX_WEB_API_RATE_LIMITED",
            Self::Timeout => "CALYX_WEB_API_TIMEOUT",
            Self::BadRequest => "CALYX_WEB_API_BAD_REQUEST",
            Self::Unauthorized => "CALYX_WEB_API_UNAUTHORIZED",
            Self::Internal => "CALYX_WEB_API_INTERNAL",
        }
    }

    /// HTTP status this code maps to.
    pub const fn status(self) -> StatusCode {
        match self {
            Self::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// One-line operator remediation (every structured error carries one).
    pub const fn remediation(self) -> &'static str {
        match self {
            Self::NotImplemented => "wire this route to its calyxd query before calling it",
            Self::NotFound => "check the request path against the documented /v1 route surface",
            Self::MethodNotAllowed => {
                "use the documented method for this route (see the Allow header)"
            }
            Self::PayloadTooLarge => "shrink the request body below the route's byte cap",
            Self::RateLimited => "slow down and retry after the Retry-After interval",
            Self::Timeout => "retry; if it persists, the upstream calyxd call is too slow",
            Self::BadRequest => "fix the request body field named in the message and resend",
            Self::Unauthorized => "present a valid Authorization: Bearer <shared-secret> header",
            Self::Internal => {
                "retry; if it persists, inspect the calyx-web-api server logs for the logged fault"
            }
        }
    }

    /// Default caller-facing message when no route-specific detail is supplied.
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::NotImplemented => "this endpoint is scaffolded but not yet wired to calyxd",
            Self::NotFound => "no route matches this request path",
            Self::MethodNotAllowed => "this route does not support the request method",
            Self::PayloadTooLarge => "the request body is larger than this route allows",
            Self::RateLimited => "too many requests for this route",
            Self::Timeout => "the request exceeded the server time budget",
            Self::BadRequest => "the request body is malformed or carries an invalid value",
            Self::Unauthorized => "missing or invalid shared-secret bearer",
            Self::Internal => "an internal error occurred",
        }
    }
}

/// A structured API error: a closed [`ErrorCode`] plus a caller-facing message.
/// The message carries ONLY static text or echoed request shape (method/path) —
/// never a secret, a query string, or a panic payload — so it is safe verbatim.
#[derive(Debug, Clone)]
pub struct ApiError {
    code: ErrorCode,
    message: String,
}

impl ApiError {
    /// Construct with an explicit, already-safe message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Construct with the code's default message.
    pub fn of(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.default_message().to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.code.status(),
            Json(json!({
                "code": self.code.code(),
                "message": self.message,
                "remediation": self.code.remediation(),
            })),
        )
            .into_response()
    }
}
