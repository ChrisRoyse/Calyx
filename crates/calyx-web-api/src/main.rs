#![deny(warnings)]

//! calyx-web-api — the thin, read-only HTTP surface in front of `calyxd`.
//!
//! Binds `127.0.0.1:8121` (loopback ONLY; external exposure is the reverse
//! proxy's job, never this process's) and exposes only the website's read
//! endpoints. No write or mutating route is compiled in: `measure`/`search`/
//! `guard` are idempotent query POSTs (a body-carrying read), `kernel`/
//! `provenance`/`health` are GETs. Every data route is a fail-loud
//! `501 Not Implemented` carrying a structured `{code,message,remediation}`
//! envelope until it is wired to `calyxd` on `127.0.0.1:8120` — a caller can
//! never mistake an unwired route for an empty/successful result.

use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::json;

/// Loopback bind address. Loopback by construction; asserted at startup.
const BIND_ADDR: &str = "127.0.0.1:8121";
/// The `calyxd` daemon this read API will query (wired by later endpoint work).
const UPSTREAM_CALYXD: &str = "127.0.0.1:8120";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    let addr: SocketAddr = BIND_ADDR
        .parse()
        .expect("BIND_ADDR is a compile-time-constant loopback socket address");
    assert!(
        addr.ip().is_loopback(),
        "calyx-web-api refuses a non-loopback bind address: {addr}"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("CALYX_WEB_API_BIND_FAILED: cannot bind {addr}: {e}"));

    tracing::info!(
        "calyx-web-api listening on http://{addr} (read-only, upstream {UPSTREAM_CALYXD})"
    );

    axum::serve(listener, router())
        .await
        .unwrap_or_else(|e| panic!("CALYX_WEB_API_SERVE_FAILED: {e}"));
}

/// The complete read-only route surface, built in one place so it is auditable
/// at a glance and unit-testable without binding a socket.
fn router() -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/measure", post(not_implemented))
        .route("/v1/search", post(not_implemented))
        .route("/v1/guard", post(not_implemented))
        .route("/v1/kernel", get(not_implemented))
        .route("/v1/provenance/{id}", get(provenance))
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

/// `/v1/provenance/{id}` echoes the requested id into the fail-loud 501 so the
/// unwired route is unambiguous in logs.
async fn provenance(Path(id): Path<String>) -> impl IntoResponse {
    not_implemented_detail(&format!("/v1/provenance/{id}"))
}

/// Fail-loud placeholder for a scaffolded-but-unwired endpoint.
async fn not_implemented() -> impl IntoResponse {
    not_implemented_detail("this endpoint")
}

fn not_implemented_detail(which: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "code": "CALYX_WEB_API_NOT_IMPLEMENTED",
            "message": format!("{which} is scaffolded but not yet wired to calyxd"),
            "remediation": "wire this route to its calyxd query before calling it",
        })),
    )
}
