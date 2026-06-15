# PH22 · T01 — E2 Temporal-Recent lens (Linear/Exponential/Step decay)

| Field | Value |
|---|---|
| **Phase** | PH22 — Default panels + temporal lenses E2/E3/E4 |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/temporal/e2_recency.rs` (≤500), `crates/calyx-registry/src/temporal/mod.rs` (≤500) |
| **Depends on** | PH17 T02 (AlgorithmicLens pattern) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §2`, `dbprdplans/05 §7` |

## Goal

Implement `E2_Temporal_Recent` as a closed-form algorithmic lens that scores
recency/freshness. No trained weights, no network calls, no randomness.
Input: `input.bytes` = i64 Unix timestamp (little-endian 8 bytes) of the
event. Config: `DecayFunction` (Linear, Exponential with half-life,
or Step). Output: `SlotVector::Dense { dim: 1, data: [score] }` where
`score ∈ [0.0, 1.0]`.

## Build (checklist of concrete, code-level steps)

- [x] `DecayFunction` enum:
  - `Linear { max_age_secs: i64 }` → `score = 1.0 − age / max_age`; clamp to [0,1].
  - `Exponential { half_life_secs: i64 }` → `score = exp(−age * 0.693 / half_life)`.
  - `Step` → `score = if age < 3600 { 0.8 } else if age < 86400 { 0.5 } else { 0.1 }`.
    (Exact thresholds from `25 §2`: `<1h: 0.8`, `<1d: 0.5`, `≥1d: 0.1`.)
- [x] `E2RecencyConfig` struct: `decay: DecayFunction`, `reference_time: i64`
  (the "now" timestamp for computing age — injected, never read from system
  clock in `measure`; this is how DOCTRINE `Clock` trait is respected for
  algorithmic lenses).
- [x] `E2RecencyLens` struct implementing `calyx_core::Lens`:
  - `id()` → `compute_lens_id` from a canonical spec for E2.
  - `shape()` → `SlotShape::Dense(1)`.
  - `modality()` → `Modality::Structured`.
  - `measure(&self, input: &Input) -> Result<SlotVector>`:
    - parse `input.bytes` as `i64::from_le_bytes(bytes[..8].try_into()?)`.
    - `age = self.config.reference_time − event_timestamp` (clamp to 0 if
      negative — future events score as maximally fresh).
    - apply `self.config.decay` formula.
    - return `SlotVector::Dense { dim: 1, data: vec![score] }`.
- [x] Input error: `input.bytes.len() < 8` → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`
  with remediation "E2 expects 8-byte little-endian i64 timestamp".

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit `linear_decay`: `reference=1000, event=0, max_age=1000` →
  `age=1000`, `score = 1.0 − 1000/1000 = 0.0`. Assert exact.
- [x] unit `linear_decay_recent`: `reference=1000, event=900, max_age=1000` →
  `score = 0.1`. Assert exact.
- [x] unit `exponential_decay`: `reference=86400, event=0, half_life=86400` →
  `score = exp(−0.693) ≈ 0.500`. Assert within 1e-4.
- [x] unit `step_decay`: `age=1800` (30 min) → `score = 0.8` exactly.
  `age=43200` (12 h) → `score = 0.5`. `age=172800` (2 d) → `score = 0.1`.
- [x] proptest: `score ∈ [0.0, 1.0]` for any non-negative age and any
  `DecayFunction`.
- [x] edge (≥3): (1) future event (`age < 0`) → clamped to `age = 0`, score
  = max fresh; (2) `max_age=0` → score = 0.0 (clamp); (3) `half_life=0` →
  score = 0.0 (avoid divide-by-zero; document as degenerate).
- [x] fail-closed: `input.bytes.len() < 8` → exact
  `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** unit test output; hand-computed reference values listed in test
- **Readback:** `cargo test -p calyx-registry e2_recency -- --nocapture 2>&1`
- **Prove:** test output shows all three decay modes producing the exact
  hand-computed reference scores; proptest green; screenshot attached to PH22
  GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH22 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
