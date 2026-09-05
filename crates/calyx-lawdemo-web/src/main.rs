//! Binary entrypoint for `calyx-lawdemo-web`.
//!
//! Load `LAWDEMO_WEB_*` config (fail-fast panic on any missing/invalid required
//! var), assert the bind is loopback, bind the socket, and serve the app.

#[cfg(unix)]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use calyx_lawdemo_web::{AppState, Config, app, health};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    // Fail loud at startup on any missing/invalid required configuration — the
    // service never boots into a half-configured state.
    let config =
        Config::from_env().unwrap_or_else(|error| panic!("CALYX_LAWDEMO_CONFIG_INVALID: {error}"));

    // Defence in depth: Config::from_map already refuses a non-loopback bind, but
    // re-assert here so no future refactor can open a public socket by accident.
    assert!(
        config.bind_addr.ip().is_loopback(),
        "calyx-lawdemo-web refuses a non-loopback bind address: {}",
        config.bind_addr
    );
    let corpus = health::verify_canonical_corpus(&config)
        .unwrap_or_else(|error| panic!("CALYX_LAWDEMO_CORPUS_INVALID: {error}"));

    let addr = config.bind_addr;
    // The abandoned-episode reaper (#2061). Started before the listener so no episode
    // can ever be opened while nothing is watching for its client to disappear.
    let _reaper = calyx_lawdemo_web::spawn_episode_reaper(&config);
    let state = AppState {
        config: Arc::new(config),
    };

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("CALYX_LAWDEMO_BIND_FAILED: cannot bind {addr}: {e}"));

    tracing::info!(
        physical_opinions = corpus["physical_opinions"].as_u64().unwrap_or(0),
        content_lenses = corpus["content_lenses"].as_u64().unwrap_or(0),
        manifest_sha256 = corpus["manifest_sha256"].as_str().unwrap_or(""),
        "calyx-lawdemo-web listening on http://{addr} (loopback only; Research Console at / and /console)"
    );

    axum::serve(listener, app(state))
        .await
        .unwrap_or_else(|e| panic!("CALYX_LAWDEMO_SERVE_FAILED: {e}"));
}
