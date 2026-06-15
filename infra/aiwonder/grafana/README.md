# Calyx Grafana dashboard (PH66 T05)

`calyx-dashboard.json` — the "Calyx — Association Engine" dashboard
(uid `calyx-main`, Grafana 10.x / schemaVersion 39). Repo-owned; the operator
imports it once into the existing `ops.leapable.ai` Grafana.

## Panels (13)

Ingest p95 + rate · Search p99 by strategy + recall tripwire · Guard FAR/FRR ·
DDA `n_eff` + kernel recall ratio · Anneal A/B exposures + improvement ratio ·
VRAM budget + hotpool free · 25-hazard summary table. Each panel's
`description` documents the metric and its normal operating range.

The hotpool panel reads `zfs_pool_free_bytes{pool="hotpool"}` (ZFS exporter);
every other panel reads a `calyx_*` metric from the calyxd `/metrics` endpoint
(PH66 T03). Latency panels use `histogram_quantile(...)` over the
`*_duration_seconds_bucket` series — a literal raw-metric expr cannot compute a
percentile, so the panels query the real PromQL.

## Import (operator)

```bash
# [OPERATOR] UI: Dashboards → New → Import → upload calyx-dashboard.json,
# then pick the Prometheus datasource for the DS_PROMETHEUS variable.
# Or via API (token in $GRAFANA_TOKEN):
curl -s -X POST https://ops.leapable.ai/api/dashboards/db \
  -H "Authorization: Bearer $GRAFANA_TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"dashboard\": $(cat calyx-dashboard.json), \"overwrite\": true}"
```

## FSV (screenshot + AI-vision — post-deploy)

`read_text` cannot capture rendered chart values (`16 §6`). After calyxd is
installed (T01) and scraped (T04) and has served traffic, open the dashboard in
the main Chrome window, screenshot it, and verify with AI-vision: recall
tripwire stat green (1.0), VRAM gauge > 0 MiB, at least one non-flat time
series, and ≥1 hazard-table row. Structural validity (uid, title, 13 panels,
calyx_ exprs, 25 hazard rows, recall thresholds) is checked at PR time.
