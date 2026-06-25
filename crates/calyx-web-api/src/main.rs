#![deny(warnings)]

//! Binary entrypoint for `calyx-web-api`: bind the loopback socket and serve
//! the read-only app (see the crate lib for the route surface + error envelope).

use std::net::SocketAddr;

use calyx_web_api::{BIND_ADDR, app};

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

    tracing::info!("calyx-web-api listening on http://{addr} (read-only)");

    axum::serve(listener, app())
        .await
        .unwrap_or_else(|e| panic!("CALYX_WEB_API_SERVE_FAILED: {e}"));
}
