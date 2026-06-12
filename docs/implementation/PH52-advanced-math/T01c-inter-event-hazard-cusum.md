# PH52 T01c - Inter-event-time Hazard + CUSUM Rate Change-point

| Field | Value |
|---|---|
| Phase | PH52 - Advanced math |
| Issue | #585 |
| Crate | `calyx-assay` |
| Files | `crates/calyx-assay/src/recurrence_hazard.rs`, `crates/calyx-assay/src/special_fn.rs`, `crates/calyx-assay/tests/recurrence_hazard_fsv.rs` |
| PRD | `docs/dbprdplans/26_ADVANCED_MATH_FRONTIERS.md` section 4 rows 3-4; `17 §8`; `25 §4b` |

## Purpose

Doc 26 §4 marks the recurrence-temporal-math block `Build`-tier. The
periodicity row (T01b / #584) was owned, but rows 3-4 — the **inter-event-time
hazard** ("expected recurrence didn't happen" anomaly) and the **CUSUM
change-point on the recurrence rate** (drift/regime-change alarm) — were owned
by no card. These are first-class capabilities the Oracle and Ward act on:
the overdue-recurrence anomaly (`25 §4b`) and the drift alarm (`17 §8`).

## Implementation

`src/recurrence_hazard.rs` provides two deterministic, fail-closed primitives
over a recurrence point process:

- **`inter_event_hazard`** — fits the inter-event gaps as a two-parameter
  **Gamma renewal process** by method of moments (`k = μ²/σ²`, `θ = σ²/μ`; the
  canonical renewal model, Corral 2004 — the hazard function uniquely defines
  the inter-event law). The survival `S(d) = Q(k, d/θ)` at elapsed
  `d = now − t_last` is the probability the next event is not yet observed;
  `S(d) ≤ α` flags **overdue**. A perfectly regular series (CV ≈ 0) collapses to
  the deterministic renewal `S(d) = 1[d < μ]` — the correct model for that
  input, not a fallback. Reports the modelled survival/hazard, an independent
  empirical-survival cross-check, the mean-cadence next-occurrence estimate, and
  the elapsed at which survival crosses `α`.
- **`recurrence_rate_cusum`** — Page's two-sided tabular CUSUM (Page 1954;
  Montgomery SPC) over the standardised gap series, reference value `k = 0.5σ`,
  decision interval `h = 5σ`. An upward run flags a **slow-down** (rate ↓), a
  downward run a **speed-up** (rate ↑); the change-point is localised to the
  last index the triggering cumulative left zero (standard onset rule). A
  perfectly regular baseline (σ = 0) is floored to keep the standardisation
  finite, so a regular rhythm that suddenly shifts is still detected.

`src/special_fn.rs` holds the shared deterministic special functions — the
regularised incomplete gamma integrals (`gammp`/`gammq`, series + continued
fraction) and `ln Γ` (Lanczos g = 7) — validated against exact closed-form
values (`Q(1, x) = e^{-x}`, `P(2, 2) = 1 − 3e^{-2}`, `Γ(5) = 24`, `Γ(½) = √π`).

`_from_series` adapters bridge directly to the on-disk source of truth: a vault
`RecurrenceSeries` read back from the Aster Recurrence CF. All input validation
fails closed with the existing `CALYX_ASSAY_INSUFFICIENT_SAMPLES` catalog code;
both modules stay under the 500-line Rust gate.

## FSV Evidence

Synthetic known-I/O (the `2+2=4` rule); vault-backed tests write occurrences to
the Aster Recurrence CF and read them back before running the detectors.

- **Overdue (deterministic renewal):** 11 occurrences at a 100s cadence,
  last at `t=2000`. Probe `now=2350` (elapsed 350) → `overdue=true`,
  `survival=0.0`, `expected_next=2100`, `overdue_threshold_secs=100`. Probe
  `now=2050` (elapsed 50) → `overdue=false`, `survival=1.0`.
- **Overdue (Gamma path):** jittered ~100s cadence (CV > 0) → the overdue flag
  flips exactly across `overdue_threshold_secs` (survival monotone-decreasing).
- **CUSUM speed-up:** 20 gaps of 100 then 20 of 20 (5× rate jump) from
  `t=1000` → change-point at `gap_index=20`, `change_time=3000`,
  `direction=speed_up`; baseline σ = 0 floored to 0.1.
- **CUSUM slow-down / no-change:** opposite-direction shift detected; a steady
  cadence fires no change-point (no false alarm).
- **Edge cases (fail-closed):** too few occurrences, `now` before the last
  occurrence, NaN occurrence, non-monotonic times, invalid `alpha`, and an
  oversized CUSUM baseline — each returns `CALYX_ASSAY_INSUFFICIENT_SAMPLES`
  with a precise message.

### aiwonder byte-readback (binding)

Run on aiwonder (RTX 5090 sm_120), `cargo check` + `clippy -D warnings` +
`cargo test -p calyx-assay` all green:

```
CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue585-recurrence-hazard-<ts> \
  cargo test -p calyx-assay --test recurrence_hazard_fsv -- --ignored --nocapture
```

- Root: `/home/croyse/calyx/data/fsv-issue585-recurrence-hazard-20260612T162550Z`
- `issue585_hazard.json` SHA256: `ee84df90841719c31da3db5b2d90265e18482e60c1b993eabab54e0c4e9d53ca`
- `issue585_cusum.json` SHA256: `98ef614d12c11736cc44d8f5654dbe1fa0ae2d2cfd65d6662cfd7864092ca380`
- `issue585_edges.json` SHA256: `ba04ae3c62abd311a35e5345348ea6abe5e0173f2b1c55482e805a88fe792896`

Read-back values (independent `cat` of the SoT):

- Hazard: planted 100s cadence, last `t=2000`; `now=2350` → `overdue=true`,
  `overdue_survival=0.0`, `expected_next=2100`, `overdue_threshold_secs=100`;
  `now=2050` → `overdue=false`, `fresh_survival=1.0`.
- CUSUM: planted 5× speed-up → `detected_gap_index=20`,
  `detected_change_time=3000`, `detected_direction=speed_up`,
  `baseline_mean_gap=100`, `baseline_sigma=0.1` (floored from 0).
- Edges: all four fail closed with `CALYX_ASSAY_INSUFFICIENT_SAMPLES` and the
  exact diagnostic message (too few occurrences, now-before-last, NaN
  occurrence, oversized CUSUM baseline).
