# PH43 T01 - Tripwire registry (metrics + thresholds + hysteresis)

| Field | Value |
|---|---|
| Phase | PH43 - Tripwires + Shadow-First + Reversible/Rollback |
| Stage | S10 - Anneal + Intelligence Objective J |
| Crate | `calyx-anneal` |
| Files | `crates/calyx-anneal/src/tripwire.rs` (<=500) |
| Depends on | none (first card; PH24 metrics consumed via trait) |
| Axioms | A14, A16 |
| PRD | `dbprdplans/12 Section 6`, `dbprdplans/27 Section 4` |

## Goal

Define the `TripwireRegistry` that watches guarded metrics: `recall@k`,
guard `FAR/FRR`, search `p99`, and ingest `p95`. Every Anneal action passes its
post-change metric readings through this registry; a crossed tripwire is the
signal for future PH43 shadow/rollback cards to auto-revert the candidate.
Hysteresis prevents oscillation after a metric first crosses its threshold.

## Build

- [x] Define `TripwireMetric::{RecallAtK, GuardFAR, GuardFRR, SearchP99, IngestP95}`.
- [x] Define `TripwireThreshold { bound, hysteresis, direction }` with `ThresholdDir::{Below, Above}`.
- [x] Store thresholds and per-metric `ThresholdState { last_value, crossed }` in `TripwireRegistry`.
- [x] Expose `check(metric, value) -> Result<TripwireResult>` with `Ok` and `Crossed { metric, threshold, hysteresis }`.
- [x] Implement lower-bound and upper-bound hysteresis clearing without oscillation.
- [x] Implement `set_tripwire(metric, bound, hysteresis)` and persist to `<vault>/.anneal/tripwire.toml`.
- [x] Implement `status() -> Vec<TripwireStatus>`.
- [x] Load defaults from vault config; create hardcoded safe defaults if absent.
- [x] Add serde persistence for config/status/result types.
- [x] Fail closed for non-finite metric values with `CALYX_TRIPWIRE_INVALID_METRIC`.
- [x] Fail closed for invalid config with `CALYX_TRIPWIRE_INVALID_CONFIG`.
- [x] Add `calyx readback config tripwire --vault <dir>` to print the TOML file's parsed thresholds plus BLAKE3.

## Defaults

| Metric | Direction | Bound | Hysteresis |
|---|---:|---:|---:|
| `recall_at_k` | below | `0.90` | `0.045` |
| `guard_far` | above | `0.01` | `0.0005` |
| `guard_frr` | above | `0.05` | `0.0025` |
| `search_p99` | above | `200.0` | `10.0` |
| `ingest_p95` | above | `500.0` | `25.0` |

## Tests

- [x] Unit: `recall@k = 0.85` against `0.90` crosses; recovery to `0.95` clears.
- [x] Unit: after crossing, `0.91` remains crossed inside the `0.90 + 0.05` band; `0.96` clears.
- [x] Proptest: lower-bound hysteresis remains crossed inside the recovery band.
- [x] Edge: `NaN` and `Inf` return `CALYX_TRIPWIRE_INVALID_METRIC`.
- [x] Edge: zero hysteresis behaves as a simple threshold.
- [x] Fail-closed: lower-bound `hysteresis > bound` returns `CALYX_TRIPWIRE_INVALID_CONFIG`.

## FSV

- Source of truth: `<vault>/.anneal/tripwire.toml` plus in-memory `TripwireRegistry` state.
- Readback: `calyx readback config tripwire --vault <vault>`.
- aiwonder evidence root: `/home/croyse/calyx/data/fsv-issue394-tripwire-20260610-2220`.
- Manual proof sequence:
  - Persist `recall_at_k` bound `0.90`, hysteresis `0.05`.
  - Feed `0.85`; read status and confirm `crossed=true`.
  - Feed `0.91`; read status and confirm it remains crossed inside hysteresis.
  - Feed `0.97`; read status and confirm `crossed=false`.
  - Read `tripwire.toml` bytes and BLAKE3 manifest from the aiwonder FSV root.

## Done when

- [x] `cargo check` + `clippy -D warnings` + tests green on aiwonder.
- [x] File line-count gate passes.
- [x] FSV evidence is attached to issue #394.
- [x] No DOCTRINE Section 9 anti-pattern is introduced.
