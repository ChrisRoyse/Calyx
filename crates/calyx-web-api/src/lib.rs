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
use calyx_core::{
    AnchorKind, CxId, Input, Modality, Result as CalyxResult, Slot, SlotId, SlotShape, SlotState,
    VaultId, VaultStore,
};
use calyx_ledger::{
    LedgerCfStore, LedgerEntry, QuarantineLookup, VerifyResult, get_answer_trace, verify_chain,
};
use calyx_lodestar::{
    KernelParams, RecallTestParams, measured_kernel_with_contributions_from_vault_allow_partial,
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

mod guardrails;
use guardrails::routes_base;
pub use guardrails::{Guardrails, app, build_app, guardrails};

mod blocking;

mod error;
pub use error::{ApiError, ErrorCode};

mod auth;
pub use auth::{AuthCtx, require_bearer};

mod health;
pub use health::probe_hhem_faithfulness_at;
use health::{health, health_full, not_implemented};

mod cache;
pub use cache::ResponseCache;
#[cfg(test)]
use cache::parse_env_u64;

mod measure;
pub use measure::MeasureCtx;
use measure::{assay_bits_handler, guard_handler, kernel_handler, measure, search};

mod provenance;
pub use provenance::ProvenanceCtx;
use provenance::provenance_wired;

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
        .route("/v1/assay/bits", get(not_implemented))
        .merge(prov_route)
        .fallback(fallback_404)
        .method_not_allowed_fallback(fallback_405)
        .layer(middleware::from_fn_with_state(limiter, guardrails))
        .layer(panic_catch_layer())
}

/// Build the app with the vault-backed routes (`/v1/health` full, `/v1/measure`,
/// `/v1/search`, `/v1/guard`, `/v1/kernel`, `/v1/assay/bits`) wired
/// (provenance still scaffolded).
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
        .route("/v1/assay/bits", get(assay_bits_handler))
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

mod metrics;
pub use metrics::{HttpMetrics, MetricsCtx};
#[cfg(test)]
use metrics::{MetricsSnapshot, render_metrics};
use metrics::{metrics_handler, track_metrics};

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
        .route("/v1/assay/bits", get(assay_bits_handler))
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
mod cache_tests;
