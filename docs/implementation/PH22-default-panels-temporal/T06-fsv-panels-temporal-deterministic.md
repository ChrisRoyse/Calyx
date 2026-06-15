# PH22 · T06 — FSV: panels instantiate + E2/E3/E4 deterministic hand-verified

| Field | Value |
|---|---|
| **Phase** | PH22 — Default panels + temporal lenses E2/E3/E4 |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/tests/panels_temporal_fsv.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05 (this phase) |
| **Axioms** | A27 |
| **PRD** | `13_STAGE3_REGISTRY.md §PH22 FSV gate`, `dbprdplans/25 §2` |

## Goal

End-to-end FSV integration test on aiwonder that proves all three PH22 exit
gate requirements in one file:
1. All four default panels instantiate with correct slot counts.
2. E2, E3, E4 produce deterministic closed-form scores verified against
   hand-computed reference values.
3. All temporal lens specs carry `retrieval_only=true, excluded_from_dedup=true`.

This is the gate that closes Stage 3 (the Registry / Lenses stage).

## Build (checklist of concrete, code-level steps)

- [x] Test `all_default_panels_instantiate`:
  - for each of `text_default()`, `code_default()`, `civic_default()`,
    `media_default()`: call `instantiate_panel` with a mock registry+store.
  - print `"{panel_name}: {slot_count} slots"`.
  - assert `text_default` has ≥ 8 slots; code ≥ 15; civic ≥ 24; media ≥ 10.
  - assert all four panels contain at least one slot with each of `E2_recency`,
    `E3_periodic`, `E4_positional` names.
- [x] Test `e2_hand_computed_linear`:
  - reference time = 1_000_000 (Unix secs), event time = 900_000, max_age = 200_000.
  - expected: `age = 100_000`, `score = 1.0 - 100000/200000 = 0.5`. Assert `score == 0.5`.
- [x] Test `e2_hand_computed_exponential`:
  - reference = 86400, event = 0, half_life = 86400.
  - expected: `score = exp(-0.693147) ≈ 0.5000`. Assert within 1e-4.
- [x] Test `e3_hand_computed_hour`:
  - timestamp representing 14:30 UTC; `target_hour=14`.
  - expected: `hour_score = 1.0 - 0/12 = 1.0` (exact match). Assert `hour_score == 1.0`.
- [x] Test `e3_hand_computed_dow`:
  - timestamp for a Monday (dow=0); `target_day_of_week=3` (Thursday).
  - circular dist = min(3, 4) = 3; `dow_score = 1.0 - 3/3.5 ≈ 0.1429`. Assert within 1e-4.
- [x] Test `e4_hand_computed_midpoint`:
  - `position=50, total=100` → `pos_ratio=0.5` →
    `[sin(π/2)=1.0, cos(π/2)=0.0, sin(π/2)=1.0, cos(π/2)=0.0]`. Assert each within 1e-6.
- [x] Test `temporal_flags_on_all_three`:
  - assert all three temporal lens specs in the text_default panel have
    `retrieval_only=true` and `excluded_from_dedup=true`.
- [x] Test `determinism_all_temporal`:
  - call `determinism_probe` (PH17 T04) on each of E2, E3, E4 with the
    canonical probe input → assert `Ok(())` for each.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] All seven sub-tests above are non-`#[ignore]` (no network needed).
- [x] Each reference value is hard-coded as a constant in the test with a
  comment showing the hand computation.
- [x] proptest: run all three temporal lenses on 100 seeded random timestamps
  → all outputs finite, all within declared ranges.
- [x] edge: `e4_hand_computed` at `position=0` and `position=total` (boundary
  values) produce the exact expected sinusoidal values.
- [x] fail-closed: if any sub-test panics, the panic message includes the lens
  name and the failing value.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-registry/tests/panels_temporal_fsv.rs` output on
  aiwonder; all tests green
- **Readback:**
  `cargo test -p calyx-registry panels_temporal_fsv -- --nocapture 2>&1`
- **Prove:** output shows:
  `text-default: 8 slots ✓`;
  `code-default: 15 slots ✓`;
  `civic-default: 24 slots ✓`;
  `media-default: 10 slots ✓`;
  `E2 linear score=0.5000 ✓`;
  `E2 exp score=0.5000±1e-4 ✓`;
  `E3 hour_score=1.0000 ✓`;
  `E3 dow_score=0.1429±1e-4 ✓`;
  `E4 midpoint=[1.0,0.0,1.0,0.0]±1e-6 ✓`;
  `temporal flags: retrieval_only=true excluded_from_dedup=true for all 3 ✓`;
  all determinism probes OK;
  screenshot of terminal output attached to PH22 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH22 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
