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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use axum::{
    Json, Router,
    body::Body,
    extract::{MatchedPath, Path, Request, State},
    http::{Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::path::{Path as FsPath, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::manifest::is_vault_seq_quarantined;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, Input, Modality, Result as CalyxResult, VaultId, VaultStore};
use calyx_ledger::{
    LedgerCfStore, LedgerEntry, QuarantineLookup, VerifyResult, get_answer_trace, verify_chain,
};
use calyx_lodestar::{
    KernelParams, RecallTestParams, measured_kernel_with_contributions_from_vault,
};
use calyx_registry::VaultPanelState;
use calyx_registry::measure::measure_constellation;
use calyx_registry::persistence::load_vault_panel_state;
use calyx_search::{FusionChoice, GuardChoice, measure_query_vectors, search_outcome};
use calyx_ward::{GuardProfile, NoveltyAction, guard as ward_guard};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::catch_panic::CatchPanicLayer;

/// Loopback bind address. Loopback by construction; asserted by the binary.
pub const BIND_ADDR: &str = "127.0.0.1:8121";
/// The `calyxd` daemon this read API will query (wired by later endpoint work).
const UPSTREAM_CALYXD: &str = "127.0.0.1:8120";

/// The HHEM faithfulness backend (#1272) whose liveness this origin aggregates
/// so the single edge circuit breaker (#1908) covers BOTH backends. Loopback;
/// overridable via `CALYX_WEB_API_HHEM_ADDR` for FSV/tests. HHEM is systemd
/// socket-activated (#1807), so a bare TCP connect always succeeds even when the
/// service is dead — the liveness probe MUST speak HTTP and read a status line.
const HHEM_ORIGIN_ADDR_DEFAULT: &str = "127.0.0.1:8799";
/// Hard ceiling on the HHEM liveness probe so a hung/socket-activated-but-dead
/// HHEM cannot stall `/v1/health` (the edge cron hits it every 300s).
const HHEM_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

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

/// The read-only route surface fully scaffolded (`measure` + `provenance` to
/// 501). The wired variants are [`build_app_with_provenance`] and
/// [`build_app_with_measure_and_provenance`].
fn routes() -> Router {
    routes_base()
        .route("/v1/health", get(health))
        .route("/v1/measure", post(not_implemented))
        .route("/v1/search", post(not_implemented))
        .route("/v1/guard", post(not_implemented))
        .route("/v1/kernel", get(not_implemented))
        .route("/v1/provenance/{id}", get(provenance_stub))
}

/// The shared base with NO route attached — every endpoint (including the now
/// stateful `/v1/health`) is added by the scaffolded or wired builder, so each
/// can choose its handler without a route-overlap panic. The deployed origin
/// uses the stateful [`health_full`] (real gpu/vault/panelVersion for the edge
/// circuit breaker); the scaffold uses the stateless [`health`].
fn routes_base() -> Router {
    Router::new()
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

/// Stateless liveness of the web-API process itself (used by the scaffold
/// builders, which have no loaded vault). The deployed origin serves
/// [`health_full`] instead.
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

/// Full origin health for the edge circuit breaker (#579/#1903): liveness PLUS
/// the REAL dependency state the breaker gates on — `gpu`, `vault`,
/// `panelVersion`. `vault` is `ready` (the vault loaded fail-loud at startup or
/// the service would not be up). `gpu` is probed HONESTLY by measuring a tiny
/// probe through the content embedder (the GPU-backed dependency): an
/// unreachable/empty embedder yields `degraded`, NEVER a fake `ok`. Always 200
/// (the breaker decides via the gpu/vault fields); `status` is `ok` only when
/// both are good.
/// Probe the HHEM faithfulness backend (#1272) for liveness over loopback so the
/// single edge circuit breaker (#1908) trips when EITHER backend fails. Returns
/// `"ok"` iff HHEM answers an HTTP request within [`HHEM_PROBE_TIMEOUT`] — a 401
/// `unauthorized` still proves the process is serving, so we check ONLY that it
/// spoke HTTP/1.x, never that auth succeeded. Returns `"degraded"` (fail-LOUD via
/// a `tracing::warn!`, NEVER a fabricated `"ok"`) on connect refusal, timeout, or
/// non-HTTP bytes.
///
/// Why an HTTP request and not a bare TCP connect: HHEM's listening socket is
/// systemd socket-activated (#1807), so the kernel ACCEPTS connections even when
/// `hhem-origin.service` is down — only an actual request-then-read distinguishes
/// a live service (fast status line) from a dead one (hangs -> timeout).
async fn probe_hhem_faithfulness() -> &'static str {
    let addr = std::env::var("CALYX_WEB_API_HHEM_ADDR")
        .unwrap_or_else(|_| HHEM_ORIGIN_ADDR_DEFAULT.to_string());
    probe_hhem_faithfulness_at(&addr).await
}

/// The address-explicit core of [`probe_hhem_faithfulness`], exposed so FSV/unit
/// tests can drive it against synthetic loopback listeners (live HTTP, silent
/// hang, non-HTTP bytes, no listener) with deterministic expected outcomes — no
/// env races, no dependence on the deployed HHEM.
pub async fn probe_hhem_faithfulness_at(addr: &str) -> &'static str {
    let probe = async {
        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|error| format!("connect {addr}: {error}"))?;
        stream
            .write_all(
                b"GET /v1/health HTTP/1.0
Host: 127.0.0.1
Connection: close

",
            )
            .await
            .map_err(|error| format!("write {addr}: {error}"))?;
        let mut buf = [0u8; 16];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|error| format!("read {addr}: {error}"))?;
        if buf[..n].starts_with(b"HTTP/") {
            Ok(())
        } else {
            Err(format!("non-HTTP response from {addr}: {:?}", &buf[..n]))
        }
    };
    match tokio::time::timeout(HHEM_PROBE_TIMEOUT, probe).await {
        Ok(Ok(())) => "ok",
        Ok(Err(detail)) => {
            tracing::warn!(detail = %detail, "CALYX_WEB_API_HHEM_PROBE_FAILED");
            "degraded"
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = HHEM_PROBE_TIMEOUT.as_millis() as u64,
                "CALYX_WEB_API_HHEM_PROBE_TIMEOUT"
            );
            "degraded"
        }
    }
}

async fn health_full(State(ctx): State<Arc<MeasureCtx>>) -> impl IntoResponse {
    let gpu = match measure_query_vectors(&ctx.state, "health") {
        Ok(measured)
            if measured
                .iter()
                .any(|(_, vector)| vector.as_dense().is_some()) =>
        {
            "ok"
        }
        Ok(_) => "degraded",
        Err(error) => {
            tracing::warn!(error = ?error, "CALYX_WEB_API_HEALTH_EMBEDDER_PROBE_FAILED");
            "degraded"
        }
    };
    let vault = "ready";
    let faithfulness = probe_hhem_faithfulness().await;
    let status = if gpu == "ok" && faithfulness == "ok" {
        "ok"
    } else {
        "degraded"
    };
    (
        StatusCode::OK,
        Json(json!({
            "status": status,
            "service": "calyx-web-api",
            "readOnly": true,
            "gpu": gpu,
            "vault": vault,
            "faithfulness": faithfulness,
            "panelVersion": u64::from(ctx.state.panel.version),
            "upstream": UPSTREAM_CALYXD,
        })),
    )
}

/// Fail-loud placeholder for a scaffolded-but-unwired endpoint.
async fn not_implemented() -> ApiError {
    ApiError::of(ErrorCode::NotImplemented)
}

// ---------------------------------------------------------------------------
// Bounded TTL response cache for the idempotent read endpoints (#1898)
// ---------------------------------------------------------------------------

/// One cached response: the EXACT serialized body bytes (so a hit replays
/// byte-for-byte) plus the monotonic instant it was stored (for TTL + `Age`).
struct CacheEntry {
    body: Arc<[u8]>,
    stored: Instant,
}

/// A bounded, TTL-expiring in-memory response cache keyed by a request-derived
/// string.
///
/// `/v1/search` (by `(query,k,guard,fusion)`) and `/v1/provenance/{id}` (by id)
/// are PURE for a given vault/ledger state — provenance in particular does a
/// full `scan()` + `verify_chain()` on every call (#1898). A short TTL bounds
/// staleness against an out-of-band vault rebuild (which also restarts this
/// process and so clears the cache) while cutting that per-request work.
///
/// Bounded two ways so memory can never run away: an expired entry is dropped
/// the moment it is read, and an insertion beyond `capacity` evicts expired
/// entries first and then the oldest-stored key. A **zero TTL disables caching
/// entirely** (every request recomputes), so the layer can be turned off via
/// env without a code change. Never caches a non-200 / error response.
pub struct ResponseCache {
    ttl: Duration,
    capacity: usize,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl ResponseCache {
    /// Explicit construction (tests inject a tiny TTL/capacity deterministically).
    /// `capacity` is floored at 1 so the cache always holds at least one entry.
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity: capacity.max(1),
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Build from the optional `CALYX_WEB_API_CACHE_TTL_SECS` (default 30, `0`
    /// disables) and `CALYX_WEB_API_CACHE_CAPACITY` (default 256) env vars. A
    /// present-but-unparseable value is a HARD error (fail loud — never a silent
    /// fallback to the default).
    pub fn from_env() -> Result<Self, String> {
        let ttl_secs = parse_env_u64("CALYX_WEB_API_CACHE_TTL_SECS", 30)?;
        let capacity = parse_env_u64("CALYX_WEB_API_CACHE_CAPACITY", 256)? as usize;
        Ok(Self::new(Duration::from_secs(ttl_secs), capacity))
    }

    /// Caching is on iff the TTL is non-zero.
    fn enabled(&self) -> bool {
        !self.ttl.is_zero()
    }

    /// Look up `key`. Returns the cached body bytes + their current age when a
    /// FRESH (non-expired) entry exists; drops the entry and returns `None` when
    /// it has expired or is absent (so an expired entry can never be served).
    fn get(&self, key: &str) -> Option<(Arc<[u8]>, Duration)> {
        if !self.enabled() {
            return None;
        }
        let mut entries = self.entries.lock().expect("response-cache mutex poisoned");
        if let Some(entry) = entries.get(key) {
            let age = entry.stored.elapsed();
            if age < self.ttl {
                return Some((Arc::clone(&entry.body), age));
            }
        }
        entries.remove(key);
        None
    }

    /// Store `body` under `key`. Evicts expired entries, then the oldest-stored
    /// key, until `len <= capacity` — a hard memory bound.
    fn put(&self, key: String, body: Arc<[u8]>) {
        if !self.enabled() {
            return;
        }
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("response-cache mutex poisoned");
        entries.insert(key, CacheEntry { body, stored: now });
        if entries.len() > self.capacity {
            let ttl = self.ttl;
            entries.retain(|_, entry| now.duration_since(entry.stored) < ttl);
            while entries.len() > self.capacity {
                let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.stored)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                entries.remove(&oldest);
            }
        }
    }
}

/// Parse a non-negative integer env var, returning `default` when unset and a
/// LOUD error when present-but-unparseable (never silently defaulted).
fn parse_env_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(raw) => raw.trim().parse::<u64>().map_err(|error| {
            format!("{name} must be a non-negative integer ({error}); got {raw:?}")
        }),
    }
}

/// Build a `200 OK` JSON response from already-serialized `body` bytes, tagging
/// it with the standard cache-observability headers: `X-Cache: HIT|MISS`
/// (Varnish/CloudFront/Fastly convention) and `Age` (seconds since stored,
/// RFC 9111 §5.1). A HIT replays the EXACT cached bytes, so it is byte-identical
/// to the MISS that populated it.
fn cached_json_response(body: Arc<[u8]>, cache_status: &'static str, age: Duration) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-cache", cache_status)
        .header(header::AGE, age.as_secs())
        .body(Body::from(body.to_vec()))
        .expect("static headers + byte body is always a valid response")
}

/// Serialize `body`, store it in `cache` under `key`, and return the `MISS`
/// response carrying the freshly-serialized bytes. A serialization failure is
/// logged in full and returned as a generic 500 (never cached).
fn store_and_respond(cache: &ResponseCache, key: String, body: &Value) -> Response {
    match serde_json::to_vec(body) {
        Ok(bytes) => {
            let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
            cache.put(key, Arc::clone(&bytes));
            cached_json_response(bytes, "MISS", Duration::ZERO)
        }
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_CACHE_SERIALIZE_FAILED");
            ApiError::of(ErrorCode::Internal).into_response()
        }
    }
}

/// Vault + panel loaded once at startup, shared read-only across requests, used
/// by the wired `/v1/measure` endpoint.
pub struct MeasureCtx {
    vault: AsterVault,
    state: VaultPanelState,
    /// The vault directory — needed by `/v1/search` to open the persisted
    /// search indexes (`idx/search/*`) under it.
    vault_dir: PathBuf,
    /// Bounded TTL cache for the idempotent `/v1/search` results (#1898).
    cache: ResponseCache,
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
        Ok(Self {
            vault,
            state,
            vault_dir: vault_dir.to_path_buf(),
            cache: ResponseCache::from_env()?,
        })
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

/// Request body for `POST /v1/search`. `k`/`guard`/`fusion` are optional with
/// safe defaults (10 / off / rrf); invalid values fail loud (BadRequest), never
/// silently clamp.
#[derive(Deserialize)]
struct SearchReq {
    query: String,
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    guard: Option<bool>,
    #[serde(default)]
    fusion: Option<String>,
}

/// Run the real Sextant search over the loaded vault and return ranked evidence
/// with stored provenance. The ranking path is the SAME `calyx_search::
/// search_outcome` the CLI `calyx search` uses (no duplication, no mocks), so
/// HTTP results match the CLI byte-for-byte on the same vault.
async fn search(State(ctx): State<Arc<MeasureCtx>>, Json(req): Json<SearchReq>) -> Response {
    let k = req.k.unwrap_or(10);
    if k == 0 {
        return ApiError::new(ErrorCode::BadRequest, "k must be greater than zero").into_response();
    }
    let (fusion, fusion_label) = match req.fusion.as_deref() {
        None | Some("rrf") => (FusionChoice::Rrf, "rrf"),
        Some("weighted-rrf") => (FusionChoice::WeightedRrf, "weighted-rrf"),
        Some("single-lens") => (FusionChoice::SingleLens, "single-lens"),
        Some("kernel-first") => (FusionChoice::KernelFirst, "kernel-first"),
        Some("pipeline") => (FusionChoice::Pipeline, "pipeline"),
        Some(other) => {
            return ApiError::new(
                ErrorCode::BadRequest,
                format!(
                    "unknown fusion '{other}' (rrf|weighted-rrf|single-lens|kernel-first|pipeline)"
                ),
            )
            .into_response();
        }
    };
    let guard_on = req.guard.unwrap_or(false);
    let guard = if guard_on {
        GuardChoice::InRegion
    } else {
        GuardChoice::Off
    };

    // Idempotent for (query,k,guard,fusion) at a fixed vault state — serve a
    // fresh cache hit byte-for-byte rather than re-running Sextant (#1898). The
    // \u{1f} (unit separator) cannot appear in the label/bool fields and so
    // keeps the composite key unambiguous across the free-text query.
    let cache_key = format!(
        "search\u{1f}{k}\u{1f}{guard_on}\u{1f}{fusion_label}\u{1f}{}",
        req.query
    );
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    match search_outcome(
        &ctx.vault,
        &ctx.state,
        &ctx.vault_dir,
        &req.query,
        k,
        fusion,
        guard,
        None,
        false,
    ) {
        Ok(outcome) => {
            let hits: Vec<Value> = outcome
                .hits
                .iter()
                .map(|hit| {
                    json!({
                        "rank": hit.rank,
                        "cxId": hit.cx_id.to_string(),
                        "score": hit.score,
                        "provenance": {
                            "ledgerSeq": hit.provenance.seq,
                            "chainHash": hex_hash(&hit.provenance.hash),
                        },
                    })
                })
                .collect();
            let body = json!({
                "query": req.query,
                "k": k,
                "guardTau": outcome.guard_tau,
                "hits": hits,
            });
            store_and_respond(&ctx.cache, cache_key, &body)
        }
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_SEARCH_FAILED");
            ApiError::of(ErrorCode::Internal).into_response()
        }
    }
}

/// The Guard CF key for the default calibrated profile (`profile\0default`),
/// matching the CLI `calyx guard calibrate` write. Read-only here.
const GUARD_DEFAULT_KEY: &[u8] = b"profile\0default";

/// Read the calibrated [`GuardProfile`] from the vault's Guard CF. `Ok(None)`
/// when no profile has been calibrated (caller maps to a structured error — the
/// guard is NEVER run against an uncalibrated/absent profile).
fn read_guard_profile(vault: &AsterVault) -> Result<Option<GuardProfile>, String> {
    let snapshot = vault.snapshot();
    let Some(bytes) = vault
        .read_cf_at(snapshot, ColumnFamily::Guard, GUARD_DEFAULT_KEY)
        .map_err(|error| format!("read guard CF: {error:?}"))?
    else {
        return Ok(None);
    };
    serde_json::from_slice::<GuardProfile>(&bytes)
        .map(Some)
        .map_err(|error| format!("decode guard profile: {error}"))
}

/// Measure `text` through the active text lenses and extract the dense vector for
/// every `required_slot` of the profile. Fails if any required slot is not
/// measurable (fail loud — never guard on a partial slot set).
fn required_dense(
    state: &VaultPanelState,
    text: &str,
    profile: &GuardProfile,
) -> Result<std::collections::BTreeMap<calyx_core::SlotId, Vec<f32>>, ApiError> {
    let measured = measure_query_vectors(state, text).map_err(|error| {
        tracing::error!(error = ?error, "CALYX_WEB_API_GUARD_MEASURE_FAILED");
        ApiError::of(ErrorCode::Internal)
    })?;
    let by_slot: std::collections::BTreeMap<_, _> = measured.into_iter().collect();
    let mut out = std::collections::BTreeMap::new();
    for slot in &profile.required_slots {
        let dense = by_slot
            .get(slot)
            .and_then(|vector| vector.as_dense())
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::BadRequest,
                    format!("input is not measurable for required guard slot {slot}"),
                )
            })?;
        out.insert(*slot, dense.to_vec());
    }
    Ok(out)
}

/// Request body for `POST /v1/guard`: an answer + its evidence, both measured
/// fresh through the panel into the profile's required slots.
#[derive(Deserialize)]
struct GuardReq {
    answer: String,
    evidence: String,
    #[serde(default)]
    high_stakes: Option<bool>,
}

/// `POST /v1/guard` — real calibrated Ward verdict. Loads the calibrated profile
/// from the vault, measures answer + evidence into the required slots, and runs
/// `calyx_ward::guard` (per-slot cosine vs conformal tau — NO flattened average,
/// INVARIANT A3). Returns accept|new-region|quarantine|refuse + the full
/// per-slot decomposition + the conformal FAR.
async fn guard_handler(State(ctx): State<Arc<MeasureCtx>>, Json(req): Json<GuardReq>) -> Response {
    if req.answer.trim().is_empty() || req.evidence.trim().is_empty() {
        return ApiError::new(
            ErrorCode::BadRequest,
            "answer and evidence must both be non-empty",
        )
        .into_response();
    }
    let profile = match read_guard_profile(&ctx.vault) {
        Ok(Some(profile)) => profile,
        Ok(None) => {
            return ApiError::new(
                ErrorCode::BadRequest,
                "no calibrated guard profile in this vault; run `calyx guard calibrate` first",
            )
            .into_response();
        }
        Err(detail) => {
            tracing::error!("CALYX_WEB_API_GUARD_PROFILE_FAILED: {detail}");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let produced = match required_dense(&ctx.state, &req.answer, &profile) {
        Ok(slots) => slots,
        Err(error) => return error.into_response(),
    };
    let matched = match required_dense(&ctx.state, &req.evidence, &profile) {
        Ok(slots) => slots,
        Err(error) => return error.into_response(),
    };
    let high_stakes = req.high_stakes.unwrap_or(true);
    let verdict = match ward_guard(&profile, &produced, &matched, high_stakes) {
        Ok(verdict) => verdict,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_GUARD_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let verdict_str = if verdict.overall_pass {
        "accept"
    } else {
        match verdict.action {
            Some(NoveltyAction::NewRegion) => "new-region",
            Some(NoveltyAction::Quarantine) => "quarantine",
            Some(NoveltyAction::RejectClosed) | None => "refuse",
        }
    };
    // Per-slot aspect from the persisted calibration (#1899): each calibrated
    // slot carries its SlotKind (Identity/Content/Stylistic) + conformal FAR.
    // Aspect is null for a slot the profile did not calibrate, or one calibrated
    // before slot_kind was persisted — surfaced honestly, never fabricated.
    let calib_per_slot = profile.calibration.as_ref().map(|meta| &meta.per_slot);
    let per_slot: Vec<Value> = verdict
        .per_slot
        .iter()
        .map(|slot| {
            let aspect = calib_per_slot
                .and_then(|map| map.get(&slot.slot))
                .and_then(|meta| meta.slot_kind)
                .map(|kind| kind.label());
            json!({
                "slot": slot.slot.get(),
                "cosine": slot.cos,
                "tau": slot.tau,
                "pass": slot.pass,
                "aspect": aspect,
            })
        })
        .collect();
    // Conformal FAR per aspect class — the worst-case (max) calibrated FAR bound
    // across the slots sharing an aspect.
    let mut far_by_aspect: std::collections::BTreeMap<&'static str, f32> =
        std::collections::BTreeMap::new();
    if let Some(map) = calib_per_slot {
        for meta in map.values() {
            if let Some(kind) = meta.slot_kind {
                far_by_aspect
                    .entry(kind.label())
                    .and_modify(|far| *far = far.max(meta.far))
                    .or_insert(meta.far);
            }
        }
    }
    let far = profile.calibration.as_ref().map(|meta| meta.far);
    let body = json!({
        "verdict": verdict_str,
        "overallPass": verdict.overall_pass,
        "provisional": verdict.provisional,
        "highStakes": high_stakes,
        "far": far,
        "farByAspect": far_by_aspect,
        "perSlot": per_slot,
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// The recall gate for the website kernel (calyxdocs/12: kernel must recall the
/// corpus at >= 0.95).
const KERNEL_RECALL_GATE: f32 = 0.95;

/// `GET /v1/kernel` — the real doc-corpus kernel for the loaded vault, with
/// MEASURED kernel-only recall (built by `calyx_lodestar::measured_kernel_from_vault`
/// reading per-concept embeddings straight from the constellations — no mock, no
/// fabricated recall). Members carry their A2 trust (anchored/provisional);
/// recall is measured against the full corpus index at gate 0.95.
async fn kernel_handler(State(ctx): State<Arc<MeasureCtx>>) -> Response {
    // The kernel is idempotent for a fixed vault and its leave-one-out
    // recallContribution is O(n) recall tests (#1901), so memoize the whole
    // artifact behind the bounded TTL cache (#1898) rather than recompute it per
    // call. Constant key — `/v1/kernel` takes no parameters.
    let cache_key = "kernel".to_string();
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    // The content slot is the active dense text lens (probe-measured so we don't
    // guess); without one there is nothing to embed a kernel over.
    let content_slot = match measure_query_vectors(&ctx.state, "calyx") {
        Ok(measured) => match measured
            .iter()
            .find_map(|(slot, vector)| vector.as_dense().map(|_| *slot))
        {
            Some(slot) => slot,
            None => {
                return ApiError::new(
                    ErrorCode::BadRequest,
                    "vault has no active dense text lens to build a kernel over",
                )
                .into_response();
            }
        },
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_KERNEL_PROBE_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let kernel_params = KernelParams::default();
    let recall_params = RecallTestParams {
        min_recall_ratio: KERNEL_RECALL_GATE,
        ..RecallTestParams::default()
    };
    let (measured, contributions) = match measured_kernel_with_contributions_from_vault(
        &ctx.vault,
        content_slot,
        &kernel_params,
        &recall_params,
        8,
        0.5,
    ) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_KERNEL_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let unanchored: std::collections::BTreeSet<_> = measured
        .kernel
        .groundedness
        .unanchored_members
        .iter()
        .copied()
        .collect();
    let contribution_by_id: std::collections::BTreeMap<_, _> = contributions
        .iter()
        .map(|(id, drop)| (*id, *drop))
        .collect();
    // Concept label = the constellation's real `label:` anchor value, read from
    // the vault — null when the concept carries no label anchor (no fabrication).
    let snapshot = ctx.vault.snapshot();
    let members: Vec<Value> = measured
        .kernel
        .members
        .iter()
        .map(|cx_id| {
            let label = match ctx.vault.get(*cx_id, snapshot) {
                Ok(cx) => cx.anchors.iter().find_map(|anchor| match &anchor.kind {
                    AnchorKind::Label(value) => Some(value.clone()),
                    _ => None,
                }),
                Err(error) => {
                    tracing::error!(error = ?error, cx_id = %cx_id, "CALYX_WEB_API_KERNEL_LABEL_READ_FAILED");
                    None
                }
            };
            json!({
                "id": cx_id.to_string(),
                "trust": if unanchored.contains(cx_id) { "provisional" } else { "anchored" },
                "recallContribution": contribution_by_id.get(cx_id),
                "label": label,
            })
        })
        .collect();
    let recall = &measured.recall;
    let body = json!({
        "kernelId": measured.kernel.kernel_id.to_string(),
        "panelVersion": measured.kernel.panel_version,
        "recallGate": KERNEL_RECALL_GATE,
        "members": members,
        "kernelSize": measured.kernel.members.len(),
        "corpusSize": measured.corpus_size,
        "groundedFraction": measured.kernel.groundedness.reached_anchor,
        "recall": {
            "measured": true,
            "kernelOnly": recall.kernel_only,
            "full": recall.full,
            "ratio": recall.ratio,
            "gate": KERNEL_RECALL_GATE,
            "passed": recall.ratio >= KERNEL_RECALL_GATE,
            "nQueriesTested": recall.n_queries_tested,
            "approxFactor": recall.approx_factor,
            "warning": recall.warning,
        },
    });
    store_and_respond(&ctx.cache, cache_key, &body)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// /v1/provenance/{id} — real Ledger answer-trace (#577)
// ---------------------------------------------------------------------------

/// Real vault-manifest-backed quarantine: a ledger seq is quarantined iff the
/// vault manifest says so (mirrors the CLI `calyx provenance` path). Never
/// silently treats a quarantined entry as trusted.
struct VaultQuarantine {
    vault_dir: PathBuf,
}

impl QuarantineLookup for VaultQuarantine {
    fn contains_quarantined(&self, range: std::ops::Range<u64>) -> CalyxResult<bool> {
        for seq in range {
            if is_vault_seq_quarantined(&self.vault_dir, seq)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// The vault's OWN append-only Ledger CF (via [`AsterLedgerCfStore`]) + its
/// manifest quarantine, opened once at startup. Unifies the origin: provenance
/// reads the SAME vault as measure/search/guard/kernel — no separate ledger
/// directory. Read-only by construction (this service never appends).
pub struct ProvenanceCtx {
    store: AsterLedgerCfStore,
    quarantine: VaultQuarantine,
    /// Bounded TTL cache for `/v1/provenance/{id}` (#1898) — the headline win,
    /// since each miss does a full ledger `scan()` + `verify_chain()`.
    cache: ResponseCache,
}

impl ProvenanceCtx {
    /// Open the vault's Ledger CF at `vault_dir`. Fails loud if the vault holds
    /// no real Aster ledger state — the service never serves provenance over an
    /// unreadable ledger.
    pub fn open(vault_dir: &FsPath) -> Result<Self, String> {
        let store = AsterLedgerCfStore::open(vault_dir)
            .map_err(|error| format!("open vault ledger {}: {error:?}", vault_dir.display()))?;
        // Fail-loud startup probe: an unscannable ledger is a hard error now.
        store
            .scan()
            .map_err(|error| format!("scan vault ledger {}: {error:?}", vault_dir.display()))?;
        Ok(Self {
            store,
            quarantine: VaultQuarantine {
                vault_dir: vault_dir.to_path_buf(),
            },
            cache: ResponseCache::from_env()?,
        })
    }

    /// Load from the required `CALYX_WEB_API_VAULT_DIR` env var (the SAME vault
    /// as measure). Fail loud if unset (no default, no fallback).
    pub fn from_env() -> Result<Self, String> {
        let dir = std::env::var("CALYX_WEB_API_VAULT_DIR").map_err(|_| {
            "CALYX_WEB_API_VAULT_DIR is required (absolute path to the vault directory)".to_string()
        })?;
        Self::open(PathBuf::from(dir).as_path())
    }
}

/// Lower-hex encode a fixed hash (BLAKE3 chain hashes are surfaced as hex).
fn hex_hash(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Serialize one ledger entry to the #577 wire shape
/// `{seq,kind,subject,prevHash,entryHash,payload}`. The payload is decoded back
/// to JSON for the caller; an undecodable payload surfaces as `null` (the entry
/// hashes still prove what bytes were committed).
fn entry_json(entry: &LedgerEntry) -> Value {
    json!({
        "seq": entry.seq,
        "kind": serde_json::to_value(entry.kind).unwrap_or(Value::Null),
        "subject": serde_json::to_value(&entry.subject).unwrap_or(Value::Null),
        "prevHash": hex_hash(&entry.prev_hash),
        "entryHash": hex_hash(&entry.entry_hash),
        "payload": serde_json::from_slice::<Value>(&entry.payload).unwrap_or(Value::Null),
    })
}

/// Surface the real `verify_chain` verdict (Intact / Broken / Corrupt).
fn chain_json(result: &VerifyResult) -> Value {
    match result {
        VerifyResult::Intact { count } => json!({ "result": "intact", "count": count }),
        VerifyResult::Broken { at_seq, .. } => json!({ "result": "broken", "atSeq": at_seq }),
        VerifyResult::Corrupt { at_seq, reason } => {
            json!({ "result": "corrupt", "atSeq": at_seq, "reason": reason })
        }
    }
}

/// `GET /v1/provenance/{id}` — the real Ledger answer-trace for an answer id.
///
/// The `{id}` path segment is the answer id (matched against the `Query`
/// subject bytes of `Answer` ledger entries). Returns the answer trace's
/// constituent entries (answer + linked kernel + guard) in the #577 shape, the
/// `verify_chain` verdict over the whole ledger, and a `trusted` bool that is
/// true ONLY when the answer trace is itself trusted (complete + no warnings,
/// mirroring `AnswerTrace::is_trusted`) AND the hash chain verifies Intact. An
/// unknown id returns a structured `found:false` body (200), never a 500.
async fn provenance_wired(
    State(ctx): State<Arc<ProvenanceCtx>>,
    Path(id): Path<String>,
) -> Response {
    // Serve a fresh cache hit byte-for-byte rather than re-scanning the whole
    // ledger + re-verifying the chain (#1898). Keyed by the answer id; the TTL
    // bounds staleness against an out-of-band ledger append.
    let cache_key = format!("provenance\u{1f}{id}");
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    // Source-of-truth scan: every read is straight off the on-disk ledger.
    let row_count = match ctx.store.scan() {
        Ok(rows) => rows.len() as u64,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_PROVENANCE_SCAN_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let chain = match verify_chain(&ctx.store, 0..row_count) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_PROVENANCE_VERIFY_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let trace = match get_answer_trace(&ctx.store, &ctx.quarantine, id.as_bytes()) {
        Ok(trace) => trace,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_PROVENANCE_TRACE_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };

    let mut entries: Vec<Value> = [
        trace.answer_entry.as_ref(),
        trace.kernel_entry.as_ref(),
        trace.guard_entry.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(entry_json)
    .collect();
    entries.sort_by_key(|value| value["seq"].as_u64().unwrap_or(0));

    let chain_intact = matches!(chain, VerifyResult::Intact { .. });
    let body = json!({
        "id": id,
        "found": trace.answer_entry.is_some(),
        "trusted": trace.is_trusted() && chain_intact,
        "complete": trace.complete,
        "warnings": trace.warnings.iter().map(|warning| format!("{warning:?}")).collect::<Vec<_>>(),
        "chain": chain_json(&chain),
        "entries": entries,
    });
    store_and_respond(&ctx.cache, cache_key, &body)
}

/// Build the app with `/v1/provenance/{id}` wired to a real Ledger but
/// `/v1/measure` still scaffolded (501). Used by the provenance FSV tests so a
/// real on-disk ledger can be exercised over HTTP without a loaded vault.
pub fn build_app_with_provenance(limiter: Arc<Guardrails>, prov: Arc<ProvenanceCtx>) -> Router {
    let prov_route = Router::new()
        .route("/v1/provenance/{id}", get(provenance_wired))
        .with_state(prov);
    routes_base()
        .route("/v1/health", get(health))
        .route("/v1/measure", post(not_implemented))
        .route("/v1/search", post(not_implemented))
        .route("/v1/guard", post(not_implemented))
        .route("/v1/kernel", get(not_implemented))
        .merge(prov_route)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(panic_catch_layer())
}

/// Build the app with the vault-backed routes (`/v1/health` full, `/v1/measure`,
/// `/v1/search`, `/v1/guard`, `/v1/kernel`) wired (provenance still scaffolded).
/// Used by the vault-endpoint FSV tests so the real Sextant + Ward + Lodestar
/// paths are exercised over HTTP without needing a ledger.
pub fn build_app_with_search(
    limiter: Arc<Guardrails>,
    measure_ctx: Arc<MeasureCtx>,
    auth: Arc<AuthCtx>,
) -> Router {
    let vault_route = Router::new()
        .route("/v1/health", get(health_full))
        .route("/v1/measure", post(measure))
        .route("/v1/search", post(search))
        .route("/v1/guard", post(guard_handler))
        .route("/v1/kernel", get(kernel_handler))
        .with_state(measure_ctx);
    routes_base()
        .route("/v1/provenance/{id}", get(provenance_stub))
        .merge(vault_route)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(middleware::from_fn_with_state(auth, require_bearer))
        .layer(panic_catch_layer())
}

// ---------------------------------------------------------------------------
// Calyx-native Prometheus metrics surface (#1249 G11, #597)
// ---------------------------------------------------------------------------

/// State for the `/metrics` collector: the SAME loaded vault panel
/// ([`MeasureCtx`]) and on-disk ledger ([`ProvenanceCtx`]) the `/v1` data
/// endpoints serve, so every gauge reflects the EXACT state the origin answers
/// from — never a separate, drift-prone health view.
pub struct MetricsCtx {
    measure: Arc<MeasureCtx>,
    prov: Arc<ProvenanceCtx>,
    /// Per-route request RED metrics (rate/errors/duration), accumulated by the
    /// [`track_metrics`] middleware and rendered alongside the engine gauges.
    http: Arc<HttpMetrics>,
}

// ---------------------------------------------------------------------------
// Per-route HTTP request metrics (RED: rate, errors, duration) — #597
// ---------------------------------------------------------------------------

/// Histogram bucket upper bounds (seconds), matching the axum/Prometheus
/// reference exponential ladder. Cumulative `le` semantics are produced at
/// observe time (each observation increments every bucket whose bound it falls
/// under), so the rendered `_bucket{le=...}` series are already monotonic.
const DURATION_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// A single route's latency histogram: cumulative bucket counts + sum + count.
#[derive(Default)]
struct DurationHisto {
    /// `bucket_counts[i]` = observations with latency <= `DURATION_BUCKETS[i]`.
    bucket_counts: [u64; DURATION_BUCKETS.len()],
    /// Sum of all observed latencies (seconds) — the `_sum` series.
    sum: f64,
    /// Total observations — the `+Inf` bucket and `_count` series.
    count: u64,
}

impl DurationHisto {
    fn observe(&mut self, secs: f64) {
        for (i, upper) in DURATION_BUCKETS.iter().enumerate() {
            if secs <= *upper {
                self.bucket_counts[i] += 1;
            }
        }
        self.sum += secs;
        self.count += 1;
    }
}

/// Thread-safe accumulator for per-route request metrics. Cardinality is bounded
/// because the route label is the matched route TEMPLATE (e.g. `/v1/provenance/{id}`),
/// never the concrete path — so `{id}` values can never explode the series count.
pub struct HttpMetrics {
    /// `(method, route_template, status_code)` -> request count.
    requests: Mutex<HashMap<(String, String, u16), u64>>,
    /// `(method, route_template)` -> latency histogram.
    durations: Mutex<HashMap<(String, String), DurationHisto>>,
}

impl HttpMetrics {
    fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            durations: Mutex::new(HashMap::new()),
        }
    }

    /// Record one completed request. Mutex poisoning is a hard, fail-loud bug
    /// (a panic while holding the lock) — we surface it rather than mask it.
    fn record(&self, method: &str, route: &str, code: u16, latency_secs: f64) {
        let key = (method.to_string(), route.to_string(), code);
        *self
            .requests
            .lock()
            .expect("HttpMetrics.requests mutex poisoned")
            .entry(key)
            .or_insert(0) += 1;
        self.durations
            .lock()
            .expect("HttpMetrics.durations mutex poisoned")
            .entry((method.to_string(), route.to_string()))
            .or_default()
            .observe(latency_secs);
    }

    /// Render the per-route counter + histogram in Prometheus text format.
    /// Series are sorted so the exposition is byte-stable for a given state.
    fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# HELP calyx_http_requests_total Total HTTP requests by method, matched route, and status code.\n\
             # TYPE calyx_http_requests_total counter\n",
        );
        let requests = self
            .requests
            .lock()
            .expect("HttpMetrics.requests mutex poisoned");
        let mut rows: Vec<(&(String, String, u16), &u64)> = requests.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        for ((method, route, code), count) in rows {
            out.push_str(&format!(
                "calyx_http_requests_total{{method=\"{method}\",route=\"{route}\",code=\"{code}\"}} {count}\n"
            ));
        }
        drop(requests);

        out.push_str(
            "# HELP calyx_http_request_duration_seconds HTTP request latency by method and matched route.\n\
             # TYPE calyx_http_request_duration_seconds histogram\n",
        );
        let durations = self
            .durations
            .lock()
            .expect("HttpMetrics.durations mutex poisoned");
        let mut hist: Vec<(&(String, String), &DurationHisto)> = durations.iter().collect();
        hist.sort_by(|a, b| a.0.cmp(b.0));
        for ((method, route), histo) in hist {
            for (i, upper) in DURATION_BUCKETS.iter().enumerate() {
                out.push_str(&format!(
                    "calyx_http_request_duration_seconds_bucket{{method=\"{method}\",route=\"{route}\",le=\"{upper}\"}} {}\n",
                    histo.bucket_counts[i]
                ));
            }
            out.push_str(&format!(
                "calyx_http_request_duration_seconds_bucket{{method=\"{method}\",route=\"{route}\",le=\"+Inf\"}} {count}\n\
                 calyx_http_request_duration_seconds_sum{{method=\"{method}\",route=\"{route}\"}} {sum:.6}\n\
                 calyx_http_request_duration_seconds_count{{method=\"{method}\",route=\"{route}\"}} {count}\n",
                count = histo.count,
                sum = histo.sum,
            ));
        }
        out
    }
}

/// Middleware: time every matched request and record it under its route
/// TEMPLATE label. Applied as a `route_layer` so `MatchedPath` is populated
/// (a global `layer` runs before routing and would see no matched path). The
/// `/metrics` scrape itself is excluded so a scrape never inflates its own
/// counters.
async fn track_metrics(
    State(http): State<Arc<HttpMetrics>>,
    request: Request,
    next: Next,
) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let method = request.method().as_str().to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    if route != "/metrics" {
        http.record(
            &method,
            &route,
            response.status().as_u16(),
            started.elapsed().as_secs_f64(),
        );
    }
    response
}

/// A point-in-time snapshot of the engine-native signals exported on `/metrics`.
/// Split out from gathering so the Prometheus exposition rendering is a PURE
/// function over plain values (synthetically testable for every code path,
/// including the broken/corrupt-chain and scan-failure edges).
struct MetricsSnapshot {
    /// 1 iff the measure panel/vault loaded and answered a probe query.
    vault_ready: i64,
    /// 1 iff the embedder produced a dense query vector (GPU path live).
    gpu_ready: i64,
    /// 1 iff the HHEM faithfulness backend probe returned ok.
    faithfulness_ready: i64,
    /// The loaded panel version (monotonic; bumps on vault rebuild).
    panel_version: u64,
    /// 1 iff the on-disk ledger scanned without error.
    scan_ok: i64,
    /// Number of entries in the append-only ledger.
    ledger_rows: u64,
    /// 1 iff `verify_chain` returned `Intact` over the whole ledger.
    chain_intact: i64,
    /// The seq of the first broken/corrupt entry, or `-1` when intact/unknown.
    chain_broken_seq: i64,
    /// How long gathering this snapshot took (collector self-instrumentation).
    scrape_duration_seconds: f64,
}

/// Render a [`MetricsSnapshot`] as Prometheus text exposition format (v0.0.4).
///
/// PURE: no I/O, deterministic for a given snapshot. Metric names are
/// `calyx_`-prefixed per the Prometheus exporter naming convention; all are
/// gauges (state snapshots that can go down). `calyx_origin_healthy` is the
/// single roll-up the breaker/alerts gate on: vault + gpu + faithfulness +
/// chain all green.
fn render_metrics(s: &MetricsSnapshot) -> String {
    let healthy = i64::from(
        s.vault_ready == 1 && s.gpu_ready == 1 && s.faithfulness_ready == 1 && s.chain_intact == 1,
    );
    format!(
        "# HELP calyx_up Whether the calyx-web-api origin process is serving (1 whenever scraped).\n\
         # TYPE calyx_up gauge\n\
         calyx_up 1\n\
         # HELP calyx_origin_healthy Roll-up: 1 iff vault+gpu+faithfulness+ledger-chain are all green.\n\
         # TYPE calyx_origin_healthy gauge\n\
         calyx_origin_healthy {healthy}\n\
         # HELP calyx_vault_ready Whether the measure panel/vault is loaded and answering (1) or not (0).\n\
         # TYPE calyx_vault_ready gauge\n\
         calyx_vault_ready {vault_ready}\n\
         # HELP calyx_gpu_ready Whether the embedder produced a dense query vector (GPU path live).\n\
         # TYPE calyx_gpu_ready gauge\n\
         calyx_gpu_ready {gpu_ready}\n\
         # HELP calyx_faithfulness_ready Whether the HHEM faithfulness backend probe returned ok.\n\
         # TYPE calyx_faithfulness_ready gauge\n\
         calyx_faithfulness_ready {faithfulness_ready}\n\
         # HELP calyx_panel_version Loaded vault panel version (bumps on vault rebuild).\n\
         # TYPE calyx_panel_version gauge\n\
         calyx_panel_version {panel_version}\n\
         # HELP calyx_ledger_scan_ok Whether the on-disk ledger scanned without error.\n\
         # TYPE calyx_ledger_scan_ok gauge\n\
         calyx_ledger_scan_ok {scan_ok}\n\
         # HELP calyx_ledger_rows Number of entries in the append-only ledger.\n\
         # TYPE calyx_ledger_rows gauge\n\
         calyx_ledger_rows {ledger_rows}\n\
         # HELP calyx_ledger_chain_intact Whether verify_chain returned Intact over the whole ledger.\n\
         # TYPE calyx_ledger_chain_intact gauge\n\
         calyx_ledger_chain_intact {chain_intact}\n\
         # HELP calyx_ledger_chain_broken_seq Seq of the first broken/corrupt ledger entry, or -1 when intact.\n\
         # TYPE calyx_ledger_chain_broken_seq gauge\n\
         calyx_ledger_chain_broken_seq {chain_broken_seq}\n\
         # HELP calyx_scrape_duration_seconds How long gathering the metrics snapshot took.\n\
         # TYPE calyx_scrape_duration_seconds gauge\n\
         calyx_scrape_duration_seconds {scrape:.6}\n",
        healthy = healthy,
        vault_ready = s.vault_ready,
        gpu_ready = s.gpu_ready,
        faithfulness_ready = s.faithfulness_ready,
        panel_version = s.panel_version,
        scan_ok = s.scan_ok,
        ledger_rows = s.ledger_rows,
        chain_intact = s.chain_intact,
        chain_broken_seq = s.chain_broken_seq,
        scrape = s.scrape_duration_seconds,
    )
}

/// `GET /metrics` — Prometheus exposition of engine-native health surfaces
/// (#1249 G11, #597). Gathers the live vault/gpu/faithfulness probe and a
/// source-of-truth ledger `scan()` + `verify_chain()`, then renders via the
/// pure [`render_metrics`]. Bearer-locked like every other route (the box
/// Prometheus presents the shared secret via `bearer_token_file`); served only
/// on the loopback bind, never exposed through the public tunnel ingress.
async fn metrics_handler(State(ctx): State<Arc<MetricsCtx>>) -> Response {
    let started = Instant::now();

    let (gpu_ready, vault_ready) = match measure_query_vectors(&ctx.measure.state, "health") {
        Ok(measured) => {
            let dense = measured
                .iter()
                .any(|(_, vector)| vector.as_dense().is_some());
            (i64::from(dense), 1)
        }
        Err(error) => {
            tracing::warn!(error = ?error, "CALYX_WEB_API_METRICS_EMBEDDER_PROBE_FAILED");
            (0, 0)
        }
    };
    let faithfulness_ready = i64::from(probe_hhem_faithfulness().await == "ok");
    let panel_version = u64::from(ctx.measure.state.panel.version);

    // Source-of-truth: scan the on-disk ledger and verify the hash chain on
    // every scrape (the ledger is small; #1898 caches the per-answer path, but
    // the chain verdict must be live so a tamper is observable within one
    // scrape interval).
    let (scan_ok, ledger_rows, chain_intact, chain_broken_seq) = match ctx.prov.store.scan() {
        Ok(entries) => {
            let rows = entries.len() as u64;
            match verify_chain(&ctx.prov.store, 0..rows) {
                Ok(VerifyResult::Intact { count }) => (1, count, 1, -1),
                Ok(VerifyResult::Broken { at_seq, .. }) => (1, rows, 0, at_seq as i64),
                Ok(VerifyResult::Corrupt { at_seq, .. }) => (1, rows, 0, at_seq as i64),
                Err(error) => {
                    tracing::error!(error = ?error, "CALYX_WEB_API_METRICS_VERIFY_FAILED");
                    (1, rows, 0, -1)
                }
            }
        }
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_METRICS_SCAN_FAILED");
            (0, 0, 0, -1)
        }
    };

    let snapshot = MetricsSnapshot {
        vault_ready,
        gpu_ready,
        faithfulness_ready,
        panel_version,
        scan_ok,
        ledger_rows,
        chain_intact,
        chain_broken_seq,
        scrape_duration_seconds: started.elapsed().as_secs_f64(),
    };
    // Engine gauges (pure) + per-route RED metrics (#597), one exposition body.
    let body = render_metrics(&snapshot) + &ctx.http.render();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

/// Build the production app with BOTH `/v1/measure` (vault) and
/// `/v1/provenance/{id}` (ledger) wired. Each stateful route is its own
/// `with_state` sub-router merged onto the shared base, avoiding route overlap.
pub fn build_app_with_measure_and_provenance(
    limiter: Arc<Guardrails>,
    measure_ctx: Arc<MeasureCtx>,
    prov: Arc<ProvenanceCtx>,
    auth: Arc<AuthCtx>,
) -> Router {
    // Shared per-route request-metrics accumulator: written by the track_metrics
    // route_layer, read by the /metrics handler — one source of truth (#597).
    let http_metrics = Arc::new(HttpMetrics::new());
    // The `/metrics` collector shares the SAME vault + ledger handles the data
    // endpoints use, so its gauges can never drift from what the origin serves.
    let metrics_ctx = Arc::new(MetricsCtx {
        measure: Arc::clone(&measure_ctx),
        prov: Arc::clone(&prov),
        http: Arc::clone(&http_metrics),
    });
    let metrics_route = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(metrics_ctx);
    let measure_route = Router::new()
        .route("/v1/health", get(health_full))
        .route("/v1/measure", post(measure))
        .route("/v1/search", post(search))
        .route("/v1/guard", post(guard_handler))
        .route("/v1/kernel", get(kernel_handler))
        .with_state(measure_ctx);
    let prov_route = Router::new()
        .route("/v1/provenance/{id}", get(provenance_wired))
        .with_state(prov);
    routes_base()
        .merge(metrics_route)
        .merge(measure_route)
        .merge(prov_route)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        // route_layer runs AFTER routing (so MatchedPath is set) but inside the
        // global guardrails/bearer layers; it records the per-route RED metrics.
        .route_layer(middleware::from_fn_with_state(http_metrics, track_metrics))
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(middleware::from_fn_with_state(auth, require_bearer))
        .layer(panic_catch_layer())
}

/// Production app with measure + provenance + bearer auth wired (used by the binary).
pub fn app_with_measure_and_provenance(
    measure_ctx: Arc<MeasureCtx>,
    prov: Arc<ProvenanceCtx>,
    auth: Arc<AuthCtx>,
) -> Router {
    build_app_with_measure_and_provenance(
        Arc::new(Guardrails::production()),
        measure_ctx,
        prov,
        auth,
    )
}

/// `/v1/provenance/{id}` scaffold (used by [`build_app`]/[`app`]): echoes the
/// requested id into the fail-loud 501 so the unwired route is unambiguous in
/// logs.
async fn provenance_stub(Path(id): Path<String>) -> ApiError {
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

// ---------------------------------------------------------------------------
// ResponseCache unit tests (#1898) — real cache, synthetic keys/bodies, no mocks
// ---------------------------------------------------------------------------
#[cfg(test)]
mod cache_tests {
    use super::*;

    fn bytes(s: &str) -> Arc<[u8]> {
        Arc::from(s.as_bytes().to_vec().into_boxed_slice())
    }

    #[test]
    fn hit_returns_byte_identical_body() {
        let cache = ResponseCache::new(Duration::from_secs(60), 16);
        let body = bytes(r#"{"id":"a","found":true}"#);
        cache.put("k".to_string(), Arc::clone(&body));
        let (got, age) = cache.get("k").expect("fresh entry must hit");
        assert_eq!(&*got, &*body, "hit must replay the exact stored bytes");
        assert!(age < Duration::from_secs(60), "fresh entry age < ttl");
    }

    #[test]
    fn absent_key_misses() {
        let cache = ResponseCache::new(Duration::from_secs(60), 16);
        assert!(cache.get("never-stored").is_none());
    }

    #[test]
    fn entry_expires_after_ttl_and_is_dropped() {
        let cache = ResponseCache::new(Duration::from_millis(40), 16);
        cache.put("k".to_string(), bytes("v"));
        assert!(cache.get("k").is_some(), "before TTL: HIT");
        std::thread::sleep(Duration::from_millis(70));
        assert!(cache.get("k").is_none(), "after TTL: MISS (expired)");
        // The expired entry must have been evicted, not merely hidden.
        assert!(
            !cache.entries.lock().unwrap().contains_key("k"),
            "expired entry must be dropped on read"
        );
    }

    #[test]
    fn zero_ttl_disables_caching() {
        let cache = ResponseCache::new(Duration::ZERO, 16);
        cache.put("k".to_string(), bytes("v"));
        assert!(cache.get("k").is_none(), "TTL=0 never serves a hit");
        assert!(
            cache.entries.lock().unwrap().is_empty(),
            "TTL=0 never stores an entry"
        );
    }

    #[test]
    fn capacity_is_a_hard_bound_evicting_oldest() {
        let cache = ResponseCache::new(Duration::from_secs(60), 2);
        cache.put("a".to_string(), bytes("1"));
        std::thread::sleep(Duration::from_millis(5));
        cache.put("b".to_string(), bytes("2"));
        std::thread::sleep(Duration::from_millis(5));
        cache.put("c".to_string(), bytes("3")); // exceeds capacity 2
        let len = cache.entries.lock().unwrap().len();
        assert_eq!(len, 2, "len never exceeds capacity");
        assert!(cache.get("a").is_none(), "oldest-stored key 'a' evicted");
        assert!(cache.get("b").is_some(), "'b' retained");
        assert!(cache.get("c").is_some(), "'c' retained");
    }

    #[test]
    fn age_reflects_time_since_store() {
        let cache = ResponseCache::new(Duration::from_secs(60), 16);
        cache.put("k".to_string(), bytes("v"));
        std::thread::sleep(Duration::from_millis(30));
        let (_, age) = cache.get("k").expect("hit");
        assert!(
            age >= Duration::from_millis(25),
            "age tracks elapsed: {age:?}"
        );
    }

    // --- /metrics exposition rendering (#1249 G11, #597) ----------------
    // PURE render path exercised with synthetic snapshots: a healthy origin,
    // a tampered (broken-chain) ledger, and a total scan failure. Each asserts
    // the EXACT series a Prometheus scrape would parse — no mocks, no I/O.

    fn metric_value(body: &str, name: &str) -> Option<f64> {
        body.lines()
            .find(|l| !l.starts_with('#') && l.split(' ').next() == Some(name))
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|v| v.parse::<f64>().ok())
    }

    #[test]
    fn render_metrics_healthy_origin_all_green() {
        let body = render_metrics(&MetricsSnapshot {
            vault_ready: 1,
            gpu_ready: 1,
            faithfulness_ready: 1,
            panel_version: 7,
            scan_ok: 1,
            ledger_rows: 126,
            chain_intact: 1,
            chain_broken_seq: -1,
            scrape_duration_seconds: 0.012_345,
        });
        // Content-shape: a TYPE line precedes every sample (Prometheus requires
        // the TYPE before the first sample for a name).
        assert!(body.contains("# TYPE calyx_origin_healthy gauge"));
        assert_eq!(metric_value(&body, "calyx_up"), Some(1.0));
        assert_eq!(metric_value(&body, "calyx_origin_healthy"), Some(1.0));
        assert_eq!(metric_value(&body, "calyx_ledger_rows"), Some(126.0));
        assert_eq!(metric_value(&body, "calyx_ledger_chain_intact"), Some(1.0));
        assert_eq!(
            metric_value(&body, "calyx_ledger_chain_broken_seq"),
            Some(-1.0)
        );
        assert_eq!(metric_value(&body, "calyx_panel_version"), Some(7.0));
    }

    #[test]
    fn render_metrics_broken_chain_flips_healthy_and_exposes_seq() {
        // Tamper edge: chain broken at seq 42 → not intact, not healthy, the
        // broken seq is surfaced so an alert can name the failing entry.
        let body = render_metrics(&MetricsSnapshot {
            vault_ready: 1,
            gpu_ready: 1,
            faithfulness_ready: 1,
            panel_version: 7,
            scan_ok: 1,
            ledger_rows: 100,
            chain_intact: 0,
            chain_broken_seq: 42,
            scrape_duration_seconds: 0.001,
        });
        assert_eq!(metric_value(&body, "calyx_ledger_chain_intact"), Some(0.0));
        assert_eq!(
            metric_value(&body, "calyx_ledger_chain_broken_seq"),
            Some(42.0)
        );
        assert_eq!(
            metric_value(&body, "calyx_origin_healthy"),
            Some(0.0),
            "a broken chain must drop the health roll-up even with gpu/vault up"
        );
    }

    #[test]
    fn render_metrics_scan_failure_is_unhealthy_with_zero_rows() {
        // Edge: ledger unreadable → scan_ok 0, rows 0, not intact, not healthy.
        let body = render_metrics(&MetricsSnapshot {
            vault_ready: 0,
            gpu_ready: 0,
            faithfulness_ready: 0,
            panel_version: 0,
            scan_ok: 0,
            ledger_rows: 0,
            chain_intact: 0,
            chain_broken_seq: -1,
            scrape_duration_seconds: 0.0,
        });
        assert_eq!(metric_value(&body, "calyx_ledger_scan_ok"), Some(0.0));
        assert_eq!(metric_value(&body, "calyx_ledger_rows"), Some(0.0));
        assert_eq!(metric_value(&body, "calyx_origin_healthy"), Some(0.0));
        // calyx_up is still 1: the process answered the scrape.
        assert_eq!(metric_value(&body, "calyx_up"), Some(1.0));
    }

    // --- per-route HTTP RED metrics (#597) -------------------------------
    // Synthetic requests with KNOWN inputs → assert the exact counter and
    // histogram series a Prometheus scrape would parse.

    #[test]
    fn http_metrics_counts_requests_by_method_route_code() {
        let m = HttpMetrics::new();
        // 2 OK + 1 error on the same route, plus a different route once.
        m.record("POST", "/v1/measure", 200, 0.02);
        m.record("POST", "/v1/measure", 200, 0.2);
        m.record("POST", "/v1/measure", 500, 1.5);
        m.record("GET", "/v1/health", 200, 0.001);
        let body = m.render();
        assert!(body.contains(
            "calyx_http_requests_total{method=\"POST\",route=\"/v1/measure\",code=\"200\"} 2"
        ));
        assert!(body.contains(
            "calyx_http_requests_total{method=\"POST\",route=\"/v1/measure\",code=\"500\"} 1"
        ));
        assert!(body.contains(
            "calyx_http_requests_total{method=\"GET\",route=\"/v1/health\",code=\"200\"} 1"
        ));
        assert!(body.contains("# TYPE calyx_http_request_duration_seconds histogram"));
    }

    #[test]
    fn http_histogram_buckets_are_cumulative_and_inf_equals_count() {
        let m = HttpMetrics::new();
        // latencies: 0.02 (<=0.025), 0.2 (<=0.25), 1.5 (<=2.5)
        for (s, lat) in [(200u16, 0.02f64), (200, 0.2), (500, 1.5)] {
            m.record("POST", "/v1/measure", s, lat);
        }
        let body = m.render();
        // le=0.025 covers only the 0.02 obs → 1
        assert!(body.contains(
            "calyx_http_request_duration_seconds_bucket{method=\"POST\",route=\"/v1/measure\",le=\"0.025\"} 1"
        ));
        // le=0.25 covers 0.02 and 0.2 → 2 (cumulative)
        assert!(body.contains(
            "calyx_http_request_duration_seconds_bucket{method=\"POST\",route=\"/v1/measure\",le=\"0.25\"} 2"
        ));
        // le=2.5 covers all three → 3
        assert!(body.contains(
            "calyx_http_request_duration_seconds_bucket{method=\"POST\",route=\"/v1/measure\",le=\"2.5\"} 3"
        ));
        // +Inf == _count == 3, _sum == 1.72
        assert!(body.contains(
            "calyx_http_request_duration_seconds_bucket{method=\"POST\",route=\"/v1/measure\",le=\"+Inf\"} 3"
        ));
        assert!(body.contains(
            "calyx_http_request_duration_seconds_count{method=\"POST\",route=\"/v1/measure\"} 3"
        ));
        assert!(body.contains(
            "calyx_http_request_duration_seconds_sum{method=\"POST\",route=\"/v1/measure\"} 1.720000"
        ));
    }

    #[test]
    fn http_metrics_empty_renders_headers_only_no_samples() {
        // Edge: zero requests → TYPE/HELP present, no sample lines.
        let body = HttpMetrics::new().render();
        assert!(body.contains("# TYPE calyx_http_requests_total counter"));
        assert!(
            !body.contains("calyx_http_requests_total{"),
            "no sample lines when nothing recorded"
        );
    }

    #[test]
    fn http_histogram_slow_request_only_in_inf_bucket() {
        // Edge: a 12s request exceeds every finite bound — it must NOT appear in
        // le=10 but must be in +Inf and _count.
        let m = HttpMetrics::new();
        m.record("GET", "/v1/kernel", 504, 12.0);
        let body = m.render();
        assert!(body.contains(
            "calyx_http_request_duration_seconds_bucket{method=\"GET\",route=\"/v1/kernel\",le=\"10\"} 0"
        ));
        assert!(body.contains(
            "calyx_http_request_duration_seconds_bucket{method=\"GET\",route=\"/v1/kernel\",le=\"+Inf\"} 1"
        ));
    }

    #[test]
    fn parse_env_u64_defaults_when_unset_and_fails_loud_when_garbage() {
        // Unset → default (use a name no test sets).
        assert_eq!(
            parse_env_u64("CALYX_WEB_API_CACHE_TTL_SECS_UNSET_XYZ", 30).unwrap(),
            30
        );
        // Present-but-garbage → loud error, never silent default.
        // SAFETY: single-threaded test; var removed immediately after assert.
        unsafe { std::env::set_var("CALYX_WEB_API_TEST_BAD_INT", "not-a-number") };
        let err = parse_env_u64("CALYX_WEB_API_TEST_BAD_INT", 7).unwrap_err();
        unsafe { std::env::remove_var("CALYX_WEB_API_TEST_BAD_INT") };
        assert!(
            err.contains("non-negative integer"),
            "loud parse error: {err}"
        );
    }
}
