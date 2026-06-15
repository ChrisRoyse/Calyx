# PH22 · T02 — E3 Temporal-Periodic lens (hour-of-day, day-of-week)

| Field | Value |
|---|---|
| **Phase** | PH22 — Default panels + temporal lenses E2/E3/E4 |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/temporal/e3_periodic.rs` (≤500) |
| **Depends on** | PH17 T02 (AlgorithmicLens pattern) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §2`, `dbprdplans/05 §7` |

## Goal

Implement `E3_Temporal_Periodic` — a closed-form lens that scores how well an
event's timestamp matches a target periodic rhythm (hour-of-day 0–23 and/or
day-of-week 0–6). This is the recurring-pattern engine (`25 §2`). No weights,
no external service. Input: 8-byte little-endian i64 UTC Unix timestamp.
Output: `SlotVector::Dense { dim: 2, data: [hour_score, dow_score] }`.

## Build (checklist of concrete, code-level steps)

- [x] `PeriodicOptions` struct (from `25 §2`):
  `target_hour: Option<u8>` (0–23),
  `target_day_of_week: Option<u8>` (0–6, Mon=0),
  `use_now: bool` (if true, use the injected `reference_time` as the current
  moment and score how close the event time's hour/dow is to that moment's
  hour/dow — for "is this event typical for this time of day?" queries).
- [x] `E3PeriodicConfig` struct: `options: PeriodicOptions`, `reference_time: i64`.
- [x] `E3PeriodicLens` implementing `calyx_core::Lens`:
  - `shape()` → `SlotShape::Dense(2)`.
  - `modality()` → `Modality::Structured`.
  - `measure(&self, input: &Input) -> Result<SlotVector>`:
    - parse event timestamp from `input.bytes` (same protocol as E2).
    - extract UTC hour (0–23) and day-of-week (0–6) from timestamp using
      integer arithmetic (no external chrono dependency if avoidable; or use
      `chrono` — add as dep).
    - `hour_score`:
      - if `target_hour = Some(h)`: `1.0 − min_circular_dist(event_hour, h) / 12.0`
        where `min_circular_dist(a, b) = min(|a-b|, 24-|a-b|)` (circular
        distance on 24-hour clock, max=12, so score ∈ [0,1]).
      - if `use_now`: compare event hour to reference_time's hour same way.
      - if neither: `1.0` (no constraint).
    - `dow_score`: analogous with circular distance on 7-day week, max=3.5.
    - return `SlotVector::Dense { dim: 2, data: [hour_score, dow_score] }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `target_hour=14`, event at 14:00 → `hour_score = 1.0`.
- [x] unit: `target_hour=14`, event at 02:00 → circular dist = min(12,12) = 12
  → `hour_score = 0.0`.
- [x] unit: `target_hour=14`, event at 08:00 → circular dist = 6 →
  `hour_score = 0.5`.
- [x] unit: `target_day_of_week=1` (Tuesday), event on a Tuesday → `dow_score = 1.0`.
- [x] unit: `target_day_of_week=1`, event on a Friday (4) → circular dist =
  min(3,4) = 3 → `dow_score = 1.0 − 3/3.5 ≈ 0.143`.
- [x] proptest: both scores ∈ [0.0, 1.0] for any timestamp.
- [x] edge (≥3): (1) Unix timestamp 0 (1970-01-01 00:00 UTC Thu=3) → scores
  match expected; (2) no `target_hour` and no `use_now` → `hour_score = 1.0`;
  (3) `input.bytes.len() < 8` → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`.
- [x] fail-closed: bad input bytes → exact error code.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** unit test output with hand-computed reference values
- **Readback:** `cargo test -p calyx-registry e3_periodic -- --nocapture 2>&1`
- **Prove:** output shows all reference cases matching expected values to 3
  decimal places; screenshot attached to PH22 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH22 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
