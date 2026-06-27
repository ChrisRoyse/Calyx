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

use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::manifest::is_vault_seq_quarantined;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Input, Modality, Result as CalyxResult, VaultId, VaultStore};
use calyx_ledger::{
    LedgerCfStore, LedgerEntry, QuarantineLookup, VerifyResult, get_answer_trace, verify_chain,
};
use calyx_lodestar::{KernelParams, RecallTestParams, measured_kernel_from_vault};
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
    /// An unhandled internal fault (including a caught panic). Never leaks detail.
    Internal,
}

impl ErrorCode {
    /// The complete closed catalog (for documentation + invariant tests).
    pub const ALL: [ErrorCode; 8] = [
        Self::NotImplemented,
        Self::NotFound,
        Self::MethodNotAllowed,
        Self::PayloadTooLarge,
        Self::RateLimited,
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
    let status = if gpu == "ok" { "ok" } else { "degraded" };
    (
        StatusCode::OK,
        Json(json!({
            "status": status,
            "service": "calyx-web-api",
            "readOnly": true,
            "gpu": gpu,
            "vault": vault,
            "panelVersion": u64::from(ctx.state.panel.version),
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
    /// The vault directory — needed by `/v1/search` to open the persisted
    /// search indexes (`idx/search/*`) under it.
    vault_dir: PathBuf,
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
    let fusion = match req.fusion.as_deref() {
        None | Some("rrf") => FusionChoice::Rrf,
        Some("weighted-rrf") => FusionChoice::WeightedRrf,
        Some("single-lens") => FusionChoice::SingleLens,
        Some("kernel-first") => FusionChoice::KernelFirst,
        Some("pipeline") => FusionChoice::Pipeline,
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
    let guard = if req.guard.unwrap_or(false) {
        GuardChoice::InRegion
    } else {
        GuardChoice::Off
    };
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
            (StatusCode::OK, Json(body)).into_response()
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
    let per_slot: Vec<Value> = verdict
        .per_slot
        .iter()
        .map(|slot| {
            json!({
                "slot": slot.slot.get(),
                "cosine": slot.cos,
                "tau": slot.tau,
                "pass": slot.pass,
            })
        })
        .collect();
    let far = profile.calibration.as_ref().map(|meta| meta.far);
    let body = json!({
        "verdict": verdict_str,
        "overallPass": verdict.overall_pass,
        "provisional": verdict.provisional,
        "highStakes": high_stakes,
        "far": far,
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
    let measured = match measured_kernel_from_vault(
        &ctx.vault,
        content_slot,
        &kernel_params,
        &recall_params,
        8,
        0.5,
    ) {
        Ok(measured) => measured,
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
    let members: Vec<Value> = measured
        .kernel
        .members
        .iter()
        .map(|cx_id| {
            json!({
                "id": cx_id.to_string(),
                "trust": if unanchored.contains(cx_id) { "provisional" } else { "anchored" },
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
        "ts": now_ms(),
    });
    (StatusCode::OK, Json(body)).into_response()
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
    (StatusCode::OK, Json(body)).into_response()
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
pub fn build_app_with_search(limiter: Arc<Guardrails>, measure_ctx: Arc<MeasureCtx>) -> Router {
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
        .layer(panic_catch_layer())
}

/// Build the production app with BOTH `/v1/measure` (vault) and
/// `/v1/provenance/{id}` (ledger) wired. Each stateful route is its own
/// `with_state` sub-router merged onto the shared base, avoiding route overlap.
pub fn build_app_with_measure_and_provenance(
    limiter: Arc<Guardrails>,
    measure_ctx: Arc<MeasureCtx>,
    prov: Arc<ProvenanceCtx>,
) -> Router {
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
        .merge(measure_route)
        .merge(prov_route)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(panic_catch_layer())
}

/// Production app with measure + provenance wired (used by the binary).
pub fn app_with_measure_and_provenance(
    measure_ctx: Arc<MeasureCtx>,
    prov: Arc<ProvenanceCtx>,
) -> Router {
    build_app_with_measure_and_provenance(Arc::new(Guardrails::production()), measure_ctx, prov)
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
