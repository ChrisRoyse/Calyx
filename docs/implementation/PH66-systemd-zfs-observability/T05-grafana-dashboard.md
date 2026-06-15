# PH66 · T05 — Grafana dashboard JSON + screenshot FSV

| Field | Value |
|---|---|
| **Phase** | PH66 — systemd + ZFS provisioning + Prometheus/Grafana |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `infra` (no Rust crate) |
| **Files** | `infra/aiwonder/grafana/calyx-dashboard.json` |
| **Depends on** | T03 (metrics exist), T04 (Prometheus target up) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/16 §6` |

## Goal

Build the Grafana Calyx dashboard JSON with panels for ingest latency p95,
search p99 per strategy, recall tripwire gauge, guard FAR/FRR, n_eff, kernel
recall ratio, Anneal A/B experiment results, VRAM budget utilization, and the
25-hazard summary. The dashboard is imported into the existing
`ops.leapable.ai` Grafana instance (auto-authed via Cloudflare Access). FSV
for this card is a **screenshot + AI-vision** — an agent opens
`ops.leapable.ai` in the one main Chrome window, navigates to the Calyx
dashboard, takes a screenshot, and uses AI-vision to verify the panels show
non-zero metric values. `read_text` cannot capture rendered chart values.

> **[OPERATOR] step:** importing the dashboard JSON into Grafana requires
> operator action (Grafana UI → Dashboards → Import → paste JSON, or via the
> Grafana API). The dashboard JSON is repo-owned; the import is one-time.

## Build (checklist of concrete, code-level steps)

- [ ] `infra/aiwonder/grafana/calyx-dashboard.json`: a valid Grafana dashboard
  JSON (Grafana 10.x format) with the following panels:
  - **Row 1 — Ingest:** `calyx_ingest_duration_seconds` p95 time series;
    `calyx_ingest_total` rate counter
  - **Row 2 — Search:** `calyx_search_duration_seconds` p99 by strategy (one
    line per strategy label); `calyx_search_recall_tripwire` stat panel (green
    `1.0` = ok, red `0.0` = tripped)
  - **Row 3 — Guard:** `calyx_guard_far` and `calyx_guard_frr` time series by
    slot
  - **Row 4 — DDA/Bits:** `calyx_assay_n_eff` gauge; `calyx_kernel_recall_ratio`
    time series
  - **Row 5 — Anneal:** `calyx_anneal_ab_variant_total` bar chart;
    `calyx_anneal_ab_improvement_ratio` time series
  - **Row 6 — Resources:** `calyx_vram_budget_used_mib` / `calyx_vram_budget_limit_mib`
    gauge (fill color changes at 80%); hotpool free space from
    `zfs_pool_free_bytes{pool="hotpool"}`
  - **Row 7 — Hazard summary:** 25-hazard table: one row per `calyx_hazard_*`
    metric; value column 0/1 with threshold coloring (red when 1)
- [ ] Dashboard JSON must have `"uid": "calyx-main"` (stable UID for linking)
  and `"title": "Calyx — Association Engine"`
- [ ] `infra/aiwonder/grafana/README.md` (≤50 lines): operator steps to import
  (UI path and API curl command), note that FSV requires screenshot + AI-vision
  (`16 §6`: "charts that `read_text` can't capture")
- [ ] Each panel's `description` field explains the metric's meaning and its
  normal operating range — serves as in-dashboard documentation

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] JSON validity: `python3 -c "import json; json.load(open('infra/aiwonder/grafana/calyx-dashboard.json'))"` exits 0
- [ ] unit: dashboard JSON has `"uid": "calyx-main"` — assert exact key/value
- [ ] unit: dashboard JSON has `"title": "Calyx — Association Engine"` — assert
- [ ] unit: count panels: dashboard has ≥ 12 panels (one per bulleted panel
  above + hazard table) — parse JSON and count `panels` array length
- [ ] unit: every panel references a `calyx_` metric in its `targets[0].expr`
  — parse JSON, extract all expressions, assert all start with `calyx_` or
  `zfs_pool_free_bytes`
- [ ] unit: the `calyx_search_recall_tripwire` panel has threshold settings:
  value 0 → red, value 1 → green — assert in the JSON
- [ ] edge: hazard table panel has exactly 25 rows configured (via repeat or
  explicit target list) — count in JSON

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** rendered Grafana dashboard at `ops.leapable.ai` showing non-zero
  metric values after calyxd has been running and serving traffic
- **Readback:**
  The FSV for this card requires **screenshot + AI-vision** (not `read_text`):
  ```
  1. Agent opens ops.leapable.ai in the main Chrome window (Cloudflare Access
     auto-authed, `16 §6`).
  2. Navigate to Dashboards → Calyx — Association Engine.
  3. Take a screenshot of the full dashboard (Synapse capture_screenshot or
     Playwright browser_take_screenshot).
  4. AI-vision reads the screenshot and verifies:
     - Each row/panel is visible and labeled correctly
     - At least one time-series panel shows a non-flat line (non-zero values)
     - The recall tripwire stat panel is green (value 1.0)
     - The VRAM gauge shows a value > 0 MiB used
     - The hazard table shows at least one row
  5. Screenshot + AI-vision verdict attached to PH66 issue.
  ```
- **Prove:** screenshot shows rendered panels with non-zero values; AI-vision
  confirms recall tripwire green + VRAM used > 0. `read_text` alone is
  explicitly insufficient for chart values (`16 §6`).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: screenshot + AI-vision verdict attached to the PH66 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
