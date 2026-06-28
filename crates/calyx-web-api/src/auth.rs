use super::*;

/// The shared-secret bearer the deployed origin requires on EVERY request
/// (#1906/#587). Loaded once at startup from `CALYX_WEB_API_BEARER_SECRET`
/// (fail-loud if unset — the origin is never anonymous). Must equal the value the
/// Worker sends as `Authorization: Bearer <CALYX_ORIGIN_SHARED_SECRET>`.
pub struct AuthCtx {
    expected_bearer: String,
}

impl AuthCtx {
    /// Construct from an explicit secret (used by tests).
    pub fn new(secret: impl Into<String>) -> Result<Self, String> {
        let secret = secret.into();
        if secret.trim().is_empty() {
            return Err("bearer secret must be non-empty".to_string());
        }
        Ok(Self {
            expected_bearer: secret,
        })
    }

    /// Load from the required `CALYX_WEB_API_BEARER_SECRET` env var. Fail loud if
    /// unset/empty — there is NO anonymous mode.
    pub fn from_env() -> Result<Self, String> {
        let secret = std::env::var("CALYX_WEB_API_BEARER_SECRET").map_err(|_| {
            "CALYX_WEB_API_BEARER_SECRET is required (the shared-secret bearer; no anonymous access)"
                .to_string()
        })?;
        Self::new(secret)
    }
}

/// Constant-time byte-equality (no early-exit timing oracle on the secret).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Fail-closed bearer auth: EVERY request must carry
/// `Authorization: Bearer <expected>` or it gets a 401 closed envelope +
/// `WWW-Authenticate: Bearer realm="calyx-origin"` (matching the HHEM origin
/// contract). Runs before the handlers; no route is anonymous.
pub async fn require_bearer(
    State(auth): State<Arc<AuthCtx>>,
    req: Request,
    next: Next,
) -> Response {
    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let ok = matches!(presented, Some(token)
        if constant_time_eq(token.as_bytes(), auth.expected_bearer.as_bytes()));
    if !ok {
        let mut resp = ApiError::of(ErrorCode::Unauthorized).into_response();
        resp.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer realm=\"calyx-origin\""),
        );
        return resp;
    }
    next.run(req).await
}
