use super::health::probe_hhem_faithfulness;
use super::*;
use crate::blocking::run_blocking;

// ---------------------------------------------------------------------------
// Calyx-native Prometheus metrics surface (#1249 G11, #597)
// ---------------------------------------------------------------------------

/// State for the `/metrics` collector: the SAME loaded vault panel
/// ([`MeasureCtx`]) and on-disk ledger ([`ProvenanceCtx`]) the `/v1` data
/// endpoints serve, so every gauge reflects the EXACT state the origin answers
/// from — never a separate, drift-prone health view.
pub struct MetricsCtx {
    pub(super) measure: Arc<MeasureCtx>,
    pub(super) prov: Arc<ProvenanceCtx>,
    /// Per-route request RED metrics (rate/errors/duration), accumulated by the
    /// [`track_metrics`] middleware and rendered alongside the engine gauges.
    pub(super) http: Arc<HttpMetrics>,
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
    pub(super) fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            durations: Mutex::new(HashMap::new()),
        }
    }

    /// Record one completed request. Mutex poisoning is a hard, fail-loud bug
    /// (a panic while holding the lock) — we surface it rather than mask it.
    pub(super) fn record(&self, method: &str, route: &str, code: u16, latency_secs: f64) {
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
    pub(super) fn render(&self) -> String {
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
pub(super) async fn track_metrics(
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
pub(super) struct MetricsSnapshot {
    /// 1 iff the measure panel/vault loaded and answered a probe query.
    pub(super) vault_ready: i64,
    /// 1 iff the embedder produced a dense query vector (GPU path live).
    pub(super) gpu_ready: i64,
    /// 1 iff the HHEM faithfulness backend probe returned ok.
    pub(super) faithfulness_ready: i64,
    /// The loaded panel version (monotonic; bumps on vault rebuild).
    pub(super) panel_version: u64,
    /// 1 iff the on-disk ledger scanned without error.
    pub(super) scan_ok: i64,
    /// Number of entries in the append-only ledger.
    pub(super) ledger_rows: u64,
    /// 1 iff `verify_chain` returned `Intact` over the whole ledger.
    pub(super) chain_intact: i64,
    /// The seq of the first broken/corrupt entry, or `-1` when intact/unknown.
    pub(super) chain_broken_seq: i64,
    /// How long gathering this snapshot took (collector self-instrumentation).
    pub(super) scrape_duration_seconds: f64,
}

/// Render a [`MetricsSnapshot`] as Prometheus text exposition format (v0.0.4).
///
/// PURE: no I/O, deterministic for a given snapshot. Metric names are
/// `calyx_`-prefixed per the Prometheus exporter naming convention; all are
/// gauges (state snapshots that can go down). `calyx_origin_healthy` is the
/// single roll-up the breaker/alerts gate on: vault + gpu + faithfulness +
/// chain all green.
pub(super) fn render_metrics(s: &MetricsSnapshot) -> String {
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
pub(super) async fn metrics_handler(State(ctx): State<Arc<MetricsCtx>>) -> Response {
    let started = Instant::now();

    let measure_ctx = Arc::clone(&ctx.measure);
    let gpu_probe = run_blocking("metrics_embedder", move || {
        match measure_query_vectors(&measure_ctx.state, "health") {
            Ok(measured) => {
                let dense = measured
                    .iter()
                    .any(|(_, vector)| vector.as_dense().is_some());
                Ok((i64::from(dense), 1))
            }
            Err(error) => {
                tracing::warn!(error = ?error, "CALYX_WEB_API_METRICS_EMBEDDER_PROBE_FAILED");
                Ok((0, 0))
            }
        }
    });
    let panel_version = u64::from(ctx.measure.state.panel.version);

    // Source-of-truth: scan the on-disk ledger and verify the hash chain on
    // every scrape (the ledger is small; #1898 caches the per-answer path, but
    // the chain verdict must be live so a tamper is observable within one
    // scrape interval).
    let prov_ctx = Arc::clone(&ctx.prov);
    let ledger_probe = run_blocking("metrics_ledger", move || {
        Ok(match prov_ctx.store.scan() {
            Ok(entries) => {
                let rows = entries.len() as u64;
                match verify_chain(&prov_ctx.store, 0..rows) {
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
        })
    });
    let (gpu_result, ledger_result, faithfulness) =
        tokio::join!(gpu_probe, ledger_probe, probe_hhem_faithfulness());
    let (gpu_ready, vault_ready) = gpu_result.unwrap_or((0, 0));
    let (scan_ok, ledger_rows, chain_intact, chain_broken_seq) =
        ledger_result.unwrap_or((0, 0, 0, -1));
    let faithfulness_ready = i64::from(faithfulness == "ok");

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
