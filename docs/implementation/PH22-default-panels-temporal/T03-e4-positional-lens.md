# PH22 · T03 — E4 Temporal-Positional lens (sequence order encoding)

| Field | Value |
|---|---|
| **Phase** | PH22 — Default panels + temporal lenses E2/E3/E4 |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/temporal/e4_positional.rs` (≤500) |
| **Depends on** | PH17 T02 (AlgorithmicLens pattern) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §2`, `dbprdplans/05 §7` |

## Goal

Implement `E4_Temporal_Positional` — a closed-form positional encoding that
scores the sequence order / position of an event within a series. No weights,
no external service. This answers "is this event early or late in a sequence?"
and "what is its relative position in a session?" Input: struct with
`position: u64` (0-indexed) and `total: u64` encoded as 16 bytes
(two u64 little-endian). Output: `SlotVector::Dense { dim: 4 }` encoding
the sinusoidal positional signal.

## Build (checklist of concrete, code-level steps)

- [x] `SequenceDirection` enum (from `25 §2`): `Forward`, `Backward`, `Both`.
- [x] `MultiAnchorMode` enum: `First`, `Last`, `All`.
- [x] `SequenceOptions` struct: `direction: SequenceDirection`,
  `multi_anchor: MultiAnchorMode`.
- [x] `E4PositionalConfig` struct: `options: SequenceOptions`.
- [x] `E4PositionalLens` implementing `calyx_core::Lens`:
  - `shape()` → `SlotShape::Dense(4)`.
  - `modality()` → `Modality::Structured`.
  - `measure(&self, input: &Input) -> Result<SlotVector>`:
    - parse `input.bytes`: first 8 bytes → `position: u64`, next 8 bytes →
      `total: u64`. If `total == 0` → treat as `total = 1` (avoid divide-by-zero).
    - `pos_ratio = position as f32 / total.max(1) as f32` — relative position
      in [0, 1].
    - `bwd_ratio = 1.0 − pos_ratio` — backward position.
    - Sinusoidal encoding:
      `[sin(pos_ratio * π), cos(pos_ratio * π),
        sin(bwd_ratio * π), cos(bwd_ratio * π)]`
    - This gives a 4-D unit-circle encoding of both forward and backward
      positions (well-defined at 0 and 1; smooth and differentiable).
    - if `direction = Forward` → zero out `data[2..4]` (backward dims = 0).
    - if `direction = Backward` → zero out `data[0..2]`.
    - if `direction = Both` → use all four (default).
    - **Do NOT L2-normalize** (temporal lenses are retrieval-only scalars,
      not cosine-space vectors; the AP-60 boost uses raw values, not cosine
      similarity). Document this explicitly.
    - return `SlotVector::Dense { dim: 4, data }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `position=0, total=10` → `pos_ratio=0.0`;
  `[sin(0)=0.0, cos(0)=1.0, sin(π)≈0.0, cos(π)=-1.0]`; assert to 1e-6.
- [x] unit: `position=5, total=10` → `pos_ratio=0.5`;
  `[sin(π/2)=1.0, cos(π/2)=0.0, sin(π/2)=1.0, cos(π/2)=0.0]`; assert to 1e-6.
- [x] unit: `position=10, total=10` → `pos_ratio=1.0`;
  `[sin(π)≈0.0, cos(π)=-1.0, sin(0)=0.0, cos(0)=1.0]`; assert to 1e-5.
- [x] unit: `direction=Forward` → `data[2..4] == [0.0, 0.0]`.
- [x] proptest: all 4 values finite for any `(position, total)` pair.
- [x] edge (≥3): (1) `total=0` → no panic (clamp to 1); (2) `position > total`
  → `pos_ratio` clamped to 1.0; (3) `position=u64::MAX, total=u64::MAX` → no
  overflow; ratio = 1.0.
- [x] fail-closed: `input.bytes.len() < 16` → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`
  with remediation "E4 expects 16 bytes: (u64 position ‖ u64 total) little-endian".

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** unit test output with pre-computed reference values
- **Readback:** `cargo test -p calyx-registry e4_positional -- --nocapture 2>&1`
- **Prove:** output shows the three reference cases with exact expected values;
  proptest passes; screenshot attached to PH22 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH22 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
