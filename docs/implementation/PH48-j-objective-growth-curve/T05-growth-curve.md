# PH48 · T05 — Growth curve (J-over-time, rise check, persistence)

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/j/growth_curve.rs` (≤500) |
| **Depends on** | T04 (IntelligenceReport — each sample is a report snapshot) |
| **Axioms** | A32 |
| **PRD** | `dbprdplans/27 §8` |

## Goal

Implement `GrowthCurve`: a time-series of `J` values sampled periodically as the
vault processes data. Persisted to the `anneal_growth` CF. Exposes `is_rising()`,
which checks whether the curve has a positive slope over a configurable recent
window (not a hard invariant, but a health signal). The `calyx anneal
growth-curve` CLI command queries and plots this series. This is the headline
FSV metric for A32: "is Calyx getting more intelligent, and how fast?"

## Build (checklist of concrete, code-level steps)

- [ ] `struct GrowthSample { ts: LogicalTime, j: f64, delta_j: f64, n_queries_since_last: u64, actions_taken: Vec<String> }`.
- [ ] `struct GrowthCurve { samples: VecDeque<GrowthSample>, max_samples: usize, cf: GrowthCf, clock: Arc<dyn Clock> }` — `max_samples` default `10_000`; oldest evicted when full.
- [ ] `fn record_sample(&mut self, report: &IntelligenceReport, n_queries: u64, actions: Vec<String>)` — appends `GrowthSample { ts, j: report.j, delta_j: report.j - last_j, n_queries_since_last, actions_taken }`; persists to CF.
- [ ] `fn is_rising(&self, window: usize) -> bool` — computes linear regression slope over the last `window` samples; returns `true` if slope > `0.0`; returns `false` if fewer than 2 samples.
- [ ] `fn curve_summary(&self) -> GrowthSummary { samples_count, j_first, j_last, j_max, slope_recent, is_rising }`.
- [ ] `fn plot_ascii(&self, width: usize, height: usize) -> String` — ASCII sparkline of `J` over time; used by `calyx anneal growth-curve`.
- [ ] `fn load_from_cf(cf: GrowthCf) -> GrowthCurve` — loads persisted samples on restart; handles empty CF gracefully.
- [ ] Clock-injected; no `SystemTime::now()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: record 5 samples with `j` values `[1.0, 1.5, 2.0, 2.3, 2.8]`; `is_rising(5)=true`; `j_last=2.8`.
- [ ] unit: record `[1.0, 2.0, 1.5, 1.8, 1.6]` (dip at index 2); `is_rising(5)=false` (negative slope); `is_rising(2)=false` (last two: `1.8→1.6`, falling).
- [ ] unit: `plot_ascii(40, 5)` for 5 samples returns a non-empty string of width ≤40 with `*` characters.
- [ ] proptest: for any non-empty samples list, `curve_summary().j_max ≥ curve_summary().j_last`.
- [ ] edge: single sample → `is_rising(10) = false` (need ≥2); `max_samples=1` → only last sample retained; `record_sample` after CF failure → in-memory sample added, error propagated but curve continues.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_growth` CF rows + `curve_summary` output.
- **Readback:** `calyx anneal growth-curve --last 20` — prints ASCII sparkline of J over time + `j_first`, `j_last`, `slope_recent`, `is_rising`.
- **Prove:** ingest a real-corpus batch (10k documents) under the autotune + mistake-closure loop; take growth curve samples every 1000 ingests; `growth-curve --last 10` shows `is_rising=true`; `j_last > j_first`. Attach `growth-curve` output to PH48 GitHub issue.

## Implementation Notes

- `crates/calyx-anneal/src/j/growth_curve.rs` stores `GrowthSample` rows in the
  Aster `anneal_growth` CF using timestamp+sequence big-endian keys.
- `calyx anneal growth-curve --vault <dir> [--last <n>]` reads the CF back,
  prints a `GrowthSummary`, and includes an ASCII plot for manual inspection.
- The ignored FSV test writes durable evidence under
  `/home/croyse/calyx/data/fsv-issue427-*` and separately reads CF rows to prove
  the bytes exist after the trigger.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH48 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
