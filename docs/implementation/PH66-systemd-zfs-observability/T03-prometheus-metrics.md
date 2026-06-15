# PH66 · T03 — Prometheus `/metrics` endpoint (all named metrics)

| Field | Value |
|---|---|
| **Phase** | PH66 — systemd + ZFS provisioning + Prometheus/Grafana |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/metrics.rs` (≤500) |
| **Depends on** | PH65 T05 (server.rs — metrics endpoint served on same loopback listener) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/16 §6` |

## Goal

Implement the Prometheus `/metrics` HTTP endpoint, served on the loopback
listener alongside the MCP surface. Register all required metric families:
ingest p95 latency, search p99 per strategy, recall tripwire, guard FAR/FRR,
n_eff, kernel recall ratio, Anneal A/B experiment counters, VRAM budget
utilization, and the full 25-hazard metric set from PH59. Metrics are correct
Prometheus format (text/plain; version 0.0.4), scrapeable by the existing
aiwonder Prometheus at `127.0.0.1:9090`.

## Build (checklist of concrete, code-level steps)

- [ ] `crates/calyxd/src/metrics.rs`: use `prometheus` crate (or `metrics` +
  `metrics-exporter-prometheus`); declare a `CalyxMetrics` struct holding all
  registered metric handles
- [ ] **Ingest metrics:**
  - `calyx_ingest_duration_seconds` histogram (p50/p95/p99), label: `vault`
  - `calyx_ingest_total` counter, labels: `vault`, `status` (`ok`/`err`)
- [ ] **Search metrics:**
  - `calyx_search_duration_seconds` histogram (p99), labels: `vault`, `strategy`
    (`single_lens`, `rrf`, `weighted_rrf`, `sparse`)
  - `calyx_search_recall_tripwire` gauge: 1.0 when recall ≥ threshold, 0.0 when
    tripped (triggers Alertmanager alert)
  - `calyx_search_total` counter, labels: `vault`, `strategy`, `status`
- [ ] **Guard metrics:**
  - `calyx_guard_far` gauge (false-accept rate per slot)
  - `calyx_guard_frr` gauge (false-reject rate per slot)
  - labels: `vault`, `slot`
- [ ] **DDA/bits metrics:**
  - `calyx_assay_n_eff` gauge, labels: `vault`, `panel`
  - `calyx_kernel_recall_ratio` gauge, labels: `vault`, `scope`
- [ ] **Anneal A/B metrics:**
  - `calyx_anneal_ab_variant_total` counter, labels: `experiment`, `variant`
  - `calyx_anneal_ab_improvement_ratio` gauge, labels: `experiment`
- [ ] **VRAM budget metric:**
  - `calyx_vram_budget_used_mib` gauge
  - `calyx_vram_budget_limit_mib` gauge
- [ ] **25-hazard metrics** (one gauge per hazard, from PH59 FSV register):
  `calyx_hazard_{hazard_id}` gauge, label: `hazard` — values 0.0 (ok) or 1.0
  (triggered); the full list of 25 hazard IDs is enumerated in the source as a
  constant array
- [ ] `/metrics` HTTP handler: on `GET /metrics`, collect all metrics from
  the global registry and write Prometheus text format to the response body;
  served from the existing loopback listener in `server.rs` as a special route
  (not MCP protocol — plain HTTP response)
- [ ] `CalyxMetrics::new() -> Self`: initializes and registers all metrics in
  the default prometheus registry; called once at daemon startup; stored in
  `Arc<CalyxMetrics>` shared across connection handlers

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `CalyxMetrics::new()` does not panic; all gauge/histogram/counter
  families are registered (assert `prometheus::gather()` returns ≥ 30
  MetricFamily entries after init)
- [ ] unit: record one ingest observation of 0.150s → `calyx_ingest_total`
  counter is 1; `calyx_ingest_duration_seconds` has a sample
- [ ] unit: record `calyx_search_recall_tripwire = 0.0` (tripped) →
  `prometheus::gather()` includes a metric with value 0.0 for that label
- [ ] unit: `calyx_vram_budget_used_mib` set to 4096, `limit` to 8192 →
  gathered text contains `calyx_vram_budget_used_mib 4096` and
  `calyx_vram_budget_limit_mib 8192` (exact string match in text output)
- [ ] unit: HTTP handler returns `Content-Type: text/plain; version=0.0.4`
  (required by Prometheus)
- [ ] edge: all 25 hazard gauges initialized to 0.0 → text output has 25 lines
  matching `calyx_hazard_` prefix
- [ ] edge: `/metrics` endpoint returns 200 OK when called from loopback, and
  the response body is parseable by `prometheus_parse` (or equivalent)
- [ ] fail-closed: a metric name collision (duplicate register) → panic at init
  time (better than silently overwriting a metric); assert `new()` panics if
  called twice in the same registry

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `curl -s http://127.0.0.1:7700/metrics` output; Prometheus target
  status
- **Readback:**
  ```bash
  # On aiwonder — scrape the endpoint directly:
  curl -s http://127.0.0.1:7700/metrics | grep -E "^calyx_"
  # Must list all metric families including calyx_hazard_ * 25

  # Via Prometheus:
  curl -s 'http://127.0.0.1:9090/api/v1/query?query=calyx_vram_budget_limit_mib' \
    | python3 -m json.tool | grep '"value"'
  ```
- **Prove:** `curl /metrics` output contains:
  - `calyx_ingest_duration_seconds` histogram
  - `calyx_search_recall_tripwire`
  - `calyx_guard_far`, `calyx_guard_frr`
  - `calyx_assay_n_eff`
  - `calyx_vram_budget_used_mib`
  - at least 25 lines matching `^calyx_hazard_`
  Full `/metrics` output and Prometheus query result attached to PH66 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH66 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
