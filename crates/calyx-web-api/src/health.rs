use super::*;
use crate::blocking::run_blocking;

/// Stateless liveness of the web-API process itself (used by the scaffold
/// builders, which have no loaded vault). The deployed origin serves
/// [`health_full`] instead.
pub(super) async fn health() -> impl IntoResponse {
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
pub(super) async fn probe_hhem_faithfulness() -> &'static str {
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

pub(super) async fn health_full(State(ctx): State<Arc<MeasureCtx>>) -> impl IntoResponse {
    let work_ctx = Arc::clone(&ctx);
    let gpu_ready = run_blocking("health_embedder", move || {
        measure_query_vectors(&work_ctx.state, "health")
            .map(|measured| {
                measured
                    .iter()
                    .any(|(_, vector)| vector.as_dense().is_some())
            })
            .map_err(|error| {
                tracing::warn!(error = ?error, "CALYX_WEB_API_HEALTH_EMBEDDER_PROBE_FAILED");
                ApiError::of(ErrorCode::Internal)
            })
    })
    .await
    .unwrap_or(false);
    let gpu = if gpu_ready { "ok" } else { "degraded" };
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
pub(super) async fn not_implemented() -> ApiError {
    ApiError::of(ErrorCode::NotImplemented)
}
