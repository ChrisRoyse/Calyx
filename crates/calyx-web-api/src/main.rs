#![deny(warnings)]

//! Binary entrypoint for `calyx-web-api`: load the vault from the environment,
//! bind the loopback socket, and serve the read-only app (see the crate lib for
//! the route surface + error envelope).

use std::net::SocketAddr;
use std::sync::Arc;

use calyx_web_api::{
    AuthCtx, BIND_ADDR, MeasureCtx, ProvenanceCtx, app_with_measure_and_provenance,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    // Fail loud at startup if the vault is not configured/openable — the service
    // does not serve a degraded surface with an unusable measure endpoint.
    let ctx = MeasureCtx::from_env()
        .unwrap_or_else(|error| panic!("CALYX_WEB_API_VAULT_LOAD_FAILED: {error}"));

    // Fail loud at startup if the ledger is not configured/readable — the
    // provenance endpoint is never served over an unreadable ledger.
    let prov = ProvenanceCtx::from_env()
        .unwrap_or_else(|error| panic!("CALYX_WEB_API_LEDGER_LOAD_FAILED: {error}"));

    // Fail loud at startup if the bearer secret is unset — the origin is never
    // anonymous (#1906/#587); every request must present the shared-secret bearer.
    let auth = AuthCtx::from_env()
        .unwrap_or_else(|error| panic!("CALYX_WEB_API_AUTH_LOAD_FAILED: {error}"));

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
        "calyx-web-api listening on http://{addr} (read-only, bearer-locked, measure + provenance wired)"
    );

    axum::serve(
        listener,
        app_with_measure_and_provenance(Arc::new(ctx), Arc::new(prov), Arc::new(auth)),
    )
    .await
    .unwrap_or_else(|e| panic!("CALYX_WEB_API_SERVE_FAILED: {e}"));
}
