#![deny(warnings)]

//! calyx-web-api — the thin, read-only HTTP surface in front of `calyxd`.
//!
//! Binds `127.0.0.1:8121` (loopback ONLY; external exposure is the reverse
//! proxy's job, never this process's) and exposes only the website's read
//! endpoints. No write or mutating route is compiled in: `measure`/`search`/
//! `guard` are idempotent query POSTs (a body-carrying read), `kernel`/
//! `provenance`/`health` are GETs.
//!
//! ## Closed error envelope
//! EVERY non-success response — a scaffolded route, an unknown path (404), a
//! wrong method (405), an oversized body (413), a rate-limited caller (429), a
//! timed-out upstream (504), or any unhandled panic (500) — is the closed
//! `{code,message,remediation}` JSON envelope (mirrors the `calyxd` `CALYX_*`
//! taxonomy). The `code` is drawn from [`ErrorCode`], a CLOSED enum, so the
//! edge client branches on a stable wire string and never parses prose. A panic
//! payload, stack trace, or internal path is NEVER surfaced in a body. Messages
//! carry only static text or the echoed request shape (method + path, never the
//! query string), so no secret/PII can leak into an error.
//!
//! ## Resource guardrails (so a slow GPU call cannot pile up)
//! A single [`guardrails`] middleware enforces, per request: a body-size cap
//! (a TIGHTER cap on the GPU-backed routes — this bounds the panel/input size
//! handed to `calyxd`), a per-route token-bucket rate limit (tighter buckets
//! on the GPU routes), and a hard [`REQUEST_TIMEOUT`] that aborts a slow call
//! with a structured `CALYX_WEB_API_TIMEOUT` 504 rather than holding the
//! connection open. All rejections are the same closed envelope.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Request, State},
    http::{Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::path::{Path as FsPath, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Input, Modality, VaultId};
use calyx_registry::VaultPanelState;
use calyx_registry::measure::measure_constellation;
use calyx_registry::persistence::load_vault_panel_state;
use serde::Deserialize;
use serde_json::json;
use tower_http::catch_panic::CatchPanicLayer;

/// Loopback bind address. Loopback by construction; asserted by the binary.
pub const BIND_ADDR: &str = "127.0.0.1:8121";
/// The `calyxd` daemon this read API will query (wired by later endpoint work).
const UPSTREAM_CALYXD: &str = "127.0.0.1:8120";

/// Global request-body byte cap. Loopback inputs are small; anything larger is
/// rejected before a handler runs.
pub const MAX_BODY_BYTES: usize = 8 * 1024;
/// TIGHTER cap on the GPU-backed routes (`/measure`, `/search`, `/guard`). This
/// bounds the panel/input size submitted to `calyxd` — the resource limit that
/// keeps a single request from monopolising the GPU.
pub const MAX_GPU_BODY_BYTES: usize = 4 * 1024;
/// Hard per-request timeout: a slow `calyxd` call is aborted with a structured
/// 504 rather than left to pile up behind the single GPU.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Is this one of the GPU-backed (calyxd) routes that gets the tighter body cap
/// and rate-limit bucket?
fn is_gpu_route(path: &str) -> bool {
    matches!(path, "/v1/measure" | "/v1/search" | "/v1/guard")
}

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
    /// An unhandled internal fault (including a caught panic). Never leaks detail.
    Internal,
}

impl ErrorCode {
    /// The complete closed catalog (for documentation + invariant tests).
    pub const ALL: [ErrorCode; 7] = [
        Self::NotImplemented,
        Self::NotFound,
        Self::MethodNotAllowed,
        Self::PayloadTooLarge,
        Self::RateLimited,
        Self::Timeout,
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

/// A simple global token-bucket per route. "Global" (not per-IP) is the correct
/// key here: the service is loopback-only behind a reverse proxy, so every
/// request shares one peer — the bucket protects the single GPU from pile-up,
/// not individual clients (that is the proxy/WAF's job). Refill is wall-clock
/// based via a monotonic [`Instant`].
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Per-request resource limits: a route-keyed token-bucket rate limiter (GPU
/// routes get a tighter bucket) plus the request timeout. Carried as shared
/// state so tests can inject a tiny limit / short timeout deterministically.
pub struct Guardrails {
    capacity: f64,
    refill_per_sec: f64,
    gpu_capacity: f64,
    gpu_refill_per_sec: f64,
    timeout: Duration,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl Guardrails {
    /// Construct explicit guardrails (used by tests to force a tiny limit /
    /// short timeout).
    pub fn new(
        capacity: f64,
        refill_per_sec: f64,
        gpu_capacity: f64,
        gpu_refill_per_sec: f64,
        timeout: Duration,
    ) -> Self {
        Self {
            capacity,
            refill_per_sec,
            gpu_capacity,
            gpu_refill_per_sec,
            timeout,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Production limits: generous on light read routes, tight on GPU routes,
    /// with the standard [`REQUEST_TIMEOUT`].
    pub fn production() -> Self {
        Self::new(60.0, 30.0, 8.0, 2.0, REQUEST_TIMEOUT)
    }

    /// Take one token for `path`. Returns `true` if allowed, `false` if the
    /// bucket is empty (rate-limited).
    fn allow(&self, path: &str) -> bool {
        let (cap, refill) = if is_gpu_route(path) {
            (self.gpu_capacity, self.gpu_refill_per_sec)
        } else {
            (self.capacity, self.refill_per_sec)
        };
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate-limiter mutex poisoned");
        let bucket = buckets.entry(path.to_owned()).or_insert(Bucket {
            tokens: cap,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill).min(cap);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Build the application with the production guardrails.
pub fn app() -> Router {
    build_app(Arc::new(Guardrails::production()))
}

/// Build the application with explicit guardrails (testable injection). Wires
/// the route surface, the enveloped 404 + 405 fallbacks, the resource
/// [`guardrails`] (body cap + rate limit + timeout), and the panic-catch layer.
pub fn build_app(limiter: Arc<Guardrails>) -> Router {
    routes()
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(panic_catch_layer())
}

/// The read-only route surface (measure scaffolded to 501; the wired variant is
/// [`build_app_with_measure`]).
fn routes() -> Router {
    routes_base().route("/v1/measure", post(not_implemented))
}

/// Every read-only route except `/v1/measure`, so the wired and scaffolded
/// builders can each attach their own measure handler without route overlap.
fn routes_base() -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/search", post(not_implemented))
        .route("/v1/guard", post(not_implemented))
        .route("/v1/kernel", get(not_implemented))
        .route("/v1/provenance/{id}", get(provenance))
}

/// Per-request resource guardrails. Order: rate limit (cheapest reject) → body
/// cap (route-aware) → timeout around the handler.
pub async fn guardrails(
    State(limiter): State<Arc<Guardrails>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_owned();

    if !limiter.allow(&path) {
        let mut resp = ApiError::new(
            ErrorCode::RateLimited,
            format!("rate limit exceeded for {path}"),
        )
        .into_response();
        resp.headers_mut()
            .insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
        return resp;
    }

    let cap = if is_gpu_route(&path) {
        MAX_GPU_BODY_BYTES
    } else {
        MAX_BODY_BYTES
    };
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, cap).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::new(
                ErrorCode::PayloadTooLarge,
                format!("request body exceeds the {cap}-byte limit for {path}"),
            )
            .into_response();
        }
    };
    let req = Request::from_parts(parts, Body::from(bytes));

    match tokio::time::timeout(limiter.timeout, next.run(req)).await {
        Ok(resp) => resp,
        Err(_elapsed) => {
            tracing::warn!(
                "CALYX_WEB_API_TIMEOUT: request to {path} exceeded {:?}",
                limiter.timeout
            );
            ApiError::of(ErrorCode::Timeout).into_response()
        }
    }
}

/// Liveness of the web-API process itself (NOT `calyxd`). Real now.
async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "calyx-web-api",
            "readOnly": true,
            "upstream": UPSTREAM_CALYXD,
        })),
    )
}

/// Fail-loud placeholder for a scaffolded-but-unwired endpoint.
async fn not_implemented() -> ApiError {
    ApiError::of(ErrorCode::NotImplemented)
}

/// Vault + panel loaded once at startup, shared read-only across requests, used
/// by the wired `/v1/measure` endpoint.
pub struct MeasureCtx {
    vault: AsterVault,
    state: VaultPanelState,
}

impl MeasureCtx {
    /// Open the vault at `vault_dir` (whose final path component is the vault
    /// id) using the CLI-compatible salt `calyx-cli-vault:{id}:{name}` and load
    /// its panel. Fails loud at every step — there is no default or fallback.
    pub fn load(vault_dir: &FsPath, name: &str) -> Result<Self, String> {
        let vault_id: VaultId = vault_dir
            .file_name()
            .and_then(|component| component.to_str())
            .ok_or_else(|| format!("vault dir has no final component: {}", vault_dir.display()))?
            .parse()
            .map_err(|error| {
                format!(
                    "vault dir name is not a vault id ({}): {error}",
                    vault_dir.display()
                )
            })?;
        let salt = format!("calyx-cli-vault:{vault_id}:{name}").into_bytes();
        let vault = AsterVault::open(vault_dir, vault_id, salt, VaultOptions::default())
            .map_err(|error| format!("open vault {}: {error:?}", vault_dir.display()))?;
        let state = load_vault_panel_state(vault_dir)
            .map_err(|error| format!("load panel state {}: {error:?}", vault_dir.display()))?;
        Ok(Self { vault, state })
    }

    /// Load from the required `CALYX_WEB_API_VAULT_DIR` + `CALYX_WEB_API_VAULT_NAME`
    /// env vars. Fail loud if either is unset.
    pub fn from_env() -> Result<Self, String> {
        let dir = std::env::var("CALYX_WEB_API_VAULT_DIR").map_err(|_| {
            "CALYX_WEB_API_VAULT_DIR is required (absolute path to the vault directory)".to_string()
        })?;
        let name = std::env::var("CALYX_WEB_API_VAULT_NAME").map_err(|_| {
            "CALYX_WEB_API_VAULT_NAME is required (vault name used at creation, for the salt)"
                .to_string()
        })?;
        Self::load(PathBuf::from(dir).as_path(), &name)
    }
}

/// Request body for `POST /v1/measure`.
#[derive(Deserialize)]
struct MeasureReq {
    text: String,
}

/// Measure the input text through the loaded vault panel and return the full
/// per-lens constellation (no-flatten). Byte-identical to the CLI `calyx
/// measure` for the same input (minus the call-time `created_at`/provenance).
/// A lens-runtime failure is logged in full and returned as a generic 500 (the
/// caller envelope never carries engine internals).
async fn measure(State(ctx): State<Arc<MeasureCtx>>, Json(req): Json<MeasureReq>) -> Response {
    let input = Input::new(Modality::Text, req.text.into_bytes());
    match measure_constellation(&ctx.vault, &ctx.state, input, now_ms()) {
        Ok(cx) => (StatusCode::OK, Json(cx)).into_response(),
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_MEASURE_FAILED");
            ApiError::of(ErrorCode::Internal).into_response()
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Build the app with `/v1/measure` wired to the loaded vault. Mirrors
/// [`build_app`] but attaches the stateful measure route.
pub fn build_app_with_measure(limiter: Arc<Guardrails>, ctx: Arc<MeasureCtx>) -> Router {
    let measure_route = Router::new()
        .route("/v1/measure", post(measure))
        .with_state(ctx);
    routes_base()
        .merge(measure_route)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(panic_catch_layer())
}

/// Production app with the measure endpoint wired (used by the binary).
pub fn app_with_measure(ctx: Arc<MeasureCtx>) -> Router {
    build_app_with_measure(Arc::new(Guardrails::production()), ctx)
}

/// `/v1/provenance/{id}` echoes the requested id into the fail-loud 501 so the
/// unwired route is unambiguous in logs.
async fn provenance(Path(id): Path<String>) -> ApiError {
    ApiError::new(
        ErrorCode::NotImplemented,
        format!("/v1/provenance/{id} is scaffolded but not yet wired to calyxd"),
    )
}

/// 404 — no route matched. Echoes method + PATH only (never the query string).
async fn fallback_404(method: Method, uri: Uri) -> ApiError {
    ApiError::new(
        ErrorCode::NotFound,
        format!("no route for {method} {}", uri.path()),
    )
}

/// 405 — the path exists but not for this method. axum sets the `Allow` header.
async fn fallback_405(method: Method, uri: Uri) -> ApiError {
    ApiError::new(
        ErrorCode::MethodNotAllowed,
        format!("{method} is not supported for {}", uri.path()),
    )
}

/// The panic-catching layer used by [`build_app`]. Exposed so the exact
/// production layer can be exercised with a synthetic panic in `tests/api.rs`.
pub fn panic_catch_layer() -> CatchPanicLayer<fn(Box<dyn Any + Send + 'static>) -> Response> {
    CatchPanicLayer::custom(on_panic as fn(Box<dyn Any + Send + 'static>) -> Response)
}

/// Convert a caught panic into a generic `CALYX_WEB_API_INTERNAL` 500. The
/// panic detail is logged server-side (robust diagnostics) but NEVER placed in
/// the response body — a caller sees only the generic envelope.
fn on_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    let detail = if let Some(s) = payload.downcast_ref::<&str>() {
        *s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "non-string panic payload"
    };
    tracing::error!("CALYX_WEB_API_INTERNAL: a request handler panicked: {detail}");
    ApiError::of(ErrorCode::Internal).into_response()
}
