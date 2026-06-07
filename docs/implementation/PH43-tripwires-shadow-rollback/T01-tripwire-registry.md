# PH43 · T01 — Tripwire registry (metrics + thresholds + hysteresis)

| Field | Value |
|---|---|
| **Phase** | PH43 — Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tripwire.rs` (≤500) |
| **Depends on** | — (first card; PH24 metrics consumed via trait) |
| **Axioms** | A14, A16 |
| **PRD** | `dbprdplans/12 §6`, `dbprdplans/27 §4` |

## Goal

Define the `TripwireRegistry` that watches four guarded metrics — `recall@k`,
guard `FAR/FRR`, search `p99`, ingest `p95` — and exposes `check(metric, value)
-> TripwireResult::Ok | TripwireResult::Crossed { metric, threshold, hysteresis
}`. Any Anneal action passes its post-change metric readings through this
registry; a `Crossed` result is the signal to auto-revert. Hysteresis prevents
oscillation: once a threshold is crossed, the metric must recover past
`threshold − hysteresis_band` before the guard clears.

## Build (checklist of concrete, code-level steps)

- [ ] Define `enum TripwireMetric { RecallAtK, GuardFAR, GuardFRR, SearchP99, IngestP95 }` and `struct TripwireThreshold { bound: f64, hysteresis: f64, direction: ThresholdDir }` (`ThresholdDir::Below` / `Above`).
- [ ] `struct TripwireRegistry` holds `HashMap<TripwireMetric, TripwireThreshold>` plus per-metric `state: ThresholdState { last_value: f64, crossed: bool }`.
- [ ] `fn check(&mut self, metric: TripwireMetric, value: f64) -> TripwireResult` — sets `crossed=true` when value crosses bound; clears only when value recovers past `bound − hysteresis` (for `Below`).
- [ ] `fn set_tripwire(&mut self, metric, bound, hysteresis)` — replaces an existing or inserts a new threshold; persist to a `tripwire.toml` in vault config dir.
- [ ] `fn status(&self) -> Vec<TripwireStatus>` — returns all metrics + current value + crossed flag.
- [ ] Default thresholds loaded from vault config; if absent use hardcoded safe defaults (`recall@k ≥ 0.90`, `FAR ≤ 0.01`, `FRR ≤ 0.05`, `search p99 ≤ 200ms`, `ingest p95 ≤ 500ms`).
- [ ] All fields `serde::{Serialize, Deserialize}` for persistence; no `SystemTime::now()` — accept a `clock: &dyn Clock` for any timestamping.
- [ ] Fail closed: if metric value is `NaN` or `Inf`, return `CALYX_TRIPWIRE_INVALID_METRIC`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: feed `recall@k = 0.85` against threshold `0.90` → `Crossed`; feed `0.95` → `Ok`; verify state transitions exactly.
- [ ] unit (hysteresis): cross threshold → then recover to `0.91` (inside hysteresis band `0.90 ± 0.05`) → still `Crossed`; recover to `0.96` → `Ok`.
- [ ] proptest: for any `(value, bound, hysteresis)` triple, `check` is monotone within hysteresis band (no oscillation).
- [ ] edge: `NaN` value → `CALYX_TRIPWIRE_INVALID_METRIC`; `Inf` value → `CALYX_TRIPWIRE_INVALID_METRIC`; zero hysteresis → behaves as simple threshold.
- [ ] fail-closed: attempting to set a threshold with `hysteresis > bound` (for a lower-bound metric) → `CALYX_TRIPWIRE_INVALID_CONFIG`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tripwire.toml` in vault config dir + in-memory `TripwireRegistry` state.
- **Readback:** `calyx readback config tripwire` (or `cat $CALYX_HOME/vault/.anneal/tripwire.toml`) — prints all thresholds with bounds and hysteresis.
- **Prove:** set `recall@k` threshold to `0.90`; feed value `0.85`; confirm `status()` returns `crossed=true` for `RecallAtK`; feed `0.91` (inside hysteresis); confirm still `crossed=true`; feed `0.97`; confirm `crossed=false`. The exact state sequence must be present in the readback.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH43 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
