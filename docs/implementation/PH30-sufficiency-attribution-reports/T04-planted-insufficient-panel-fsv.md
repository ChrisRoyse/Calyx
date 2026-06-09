# PH30 · T04 — Planted-insufficient panel FSV + trusted/provisional tagging

> **Status: DONE in Stage 5 core.** The planted sufficiency, attribution, trust,
> and abundance-report paths are covered by `stage5_full_stack_fsv`; byte
> readback is the JSON artifact under the Stage 5 FSV root. Human CLI commands
> are deferred to PH62.

| Field | Value |
|---|---|
| **Phase** | PH30 — Panel sufficiency + attribution + reports |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/tests.rs` (≤500) |
| **Depends on** | T01, T02, T03 (all sufficiency + report implementations) |
| **Axioms** | A2, A8, A16 |
| **PRD** | `dbprdplans/07 §4`, `07 §7`, `15_STAGE5_LOOM_ASSAY.md` PH30 FSV gate |

## Goal

Write the planted-insufficient panel FSV tests that close PH30 and provide
byte-level proof on aiwonder: (1) a known-insufficient panel (`I ≪ H`) is
flagged with per-slot deficit; (2) trusted bits are only when grounded (`A2`);
(3) `abundance_report` shows the four honest numbers. These tests read real
bytes from the assay CF and the CLI output, not harness assertions only.

## Build (checklist of concrete, code-level steps)

- [x] Implement `test_panel_insufficiency_planted`:
  - create a test vault with N=5 slots; slot vectors are random (independent of anchor); 300 labeled samples (binary anchor, balanced, seed=42)
  - known: `H(anchor) ≈ 1.0 bit`, `I(panel; anchor) ≈ 0.0–0.1 bits`
  - call `panel_sufficiency(anchor, panel, vault, forge, clock)`
  - assert `verdict: Insufficient { deficit_bits > 0.8 }`
  - read the assay CF: `calyx readback --cf assay --panel <id> --anchor grounded` → confirm `deficit_bits > 0.8`
- [x] Implement `test_per_slot_deficit_identified`:
  - create a panel where slot_a has MI=0.3 bits, slot_b has MI=0.0 bits (random noise), slot_c has MI=0.0 bits
  - call `bits_report(panel, anchor, ...)`
  - assert `slot_b.marginal_bits ≈ 0.0` and `slot_c.marginal_bits ≈ 0.0`; `slot_a.marginal_bits ≈ 0.3`
  - the report identifies slot_b and slot_c as the deficit slots
- [x] Implement `test_bits_trust_grounded_vs_provisional`:
  - grounded anchor (`AnchorKind::Binary { source: Grounded }`) → `MiEstimate { trust: Trusted }`
  - provisional anchor (`AnchorKind::Binary { source: AutoLabeled }`) → `MiEstimate { trust: Provisional }`
  - read back both from assay CF; confirm the `trust` field byte is different
- [x] Implement `test_abundance_report_four_honest_numbers`:
  - ingest 50 constellations into a test vault with N=5 lenses, grounded anchor
  - run `stage5_full_stack_fsv` and read `stage5-readback.json`
  - assert all five fields present: N=5, C(N,2)=10, materialized count, n_eff (Computed), DPI ceiling (Computed)
  - assert `[provisional]` does NOT appear in the output
- [x] All tests: seeded RNG, injected `FixedClock`, no `Instant::now()`, no `thread_rng()`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] test_panel_insufficiency_planted → `deficit_bits > 0.8` (assertion in test + CF readback)
- [x] test_per_slot_deficit_identified → slot_b and slot_c `marginal_bits < 0.02`; slot_a `marginal_bits > 0.25`
- [x] test_bits_trust_grounded_vs_provisional → `Trusted` and `Provisional` tags present in CF rows
- [x] test_abundance_report_four_honest_numbers → all four fields non-provisional in stdout
- [x] regression: all four tests are deterministic across 3 consecutive runs on aiwonder

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the assay JSON readback and `stage5-readback.json` for the
  planted-insufficient panel case.
- **Readback:**
  ```
  cargo test test_panel_insufficiency_planted -- --nocapture
  cargo test test_per_slot_deficit_identified -- --nocapture
  cargo test test_bits_trust_grounded_vs_provisional -- --nocapture
  cargo test test_abundance_report_four_honest_numbers -- --nocapture
  calyx readback --cf assay --panel <id> --anchor grounded
  cat /home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final/stage5-readback.json
  ```
- **Prove:**
  - `panel_insufficiency` CF row shows `deficit_bits > 0.8`
  - `bits_report` CF row identifies slot_b and slot_c as deficit slots
  - `trust: Trusted` in CF row for grounded anchor; `trust: Provisional` for auto-labeled
  - `stage5-readback.json` contains all four honest numbers without fabricated values
  - All tests pass deterministically on 3 consecutive runs
  - Evidence (terminal screenshots + CF readback) posted to PH30 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH30 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
