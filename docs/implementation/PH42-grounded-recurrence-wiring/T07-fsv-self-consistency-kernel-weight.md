# PH42 · T07 — FSV: recurring-agreeing → high self-consistency; recurring-differing → flaky; frequency → kernel weight

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-assay` / `calyx-lodestar` |
| **Files** | `crates/calyx-assay/src/tests.rs` (≤500) |
| **Depends on** | T06 (this phase) |
| **Axioms** | A29, A20, A10 |
| **PRD** | `dbprdplans/25 §4c`, `dbprdplans/07 §3b`, `dbprdplans/08 §2` |

## Goal

Write the formal FSV test suite that proves PH42's two exit-gate invariants on
aiwonder: (1) `oracle_self_consistency` correctly classifies a corpus of
recurring events with agreeing outcomes as high-consistency (≥ 0.90) and a
corpus with differing outcomes as flaky (≤ 0.60); (2) a high-frequency
constellation appears in the kernel graph node list with weight above a
low-frequency constellation when betweenness scores are equal — frequency raises
kernel candidacy.

## Build (checklist of concrete, code-level steps)

- [ ] `fsv_agreeing_outcomes_high_consistency`: construct a domain with 5 CxIds each recurring 4× (20 occurrences total). All outcome anchors agree across occurrences for all 5. Run `oracle_self_consistency(domain)`. Assert result ≥ 0.90.
- [ ] `fsv_differing_outcomes_flaky`: construct a domain with 5 CxIds each recurring 4×. For each CxId, outcomes alternate agree/disagree (2 agree, 2 disagree per CxId → `agreement_rate ≈ C(2,2)/C(4,2) = 1/6 ≈ 0.167`). Run `oracle_self_consistency`. Assert result ≤ 0.60.
- [ ] `fsv_mixed_domain_self_consistency`: combine above two domains into one (5 consistent + 5 flaky CxIds). Assert result is between 0.55 and 0.75 (mixed).
- [ ] `fsv_frequency_raises_kernel_weight`: construct two nodes A and B with equal betweenness score = 0.80. Ingest A 50 times (frequency=50), B once (frequency=1). Run `build_kernel` with frequency bonuses. Assert A's kernel score > B's kernel score. Assert A appears at a higher rank in the kernel node list.
- [ ] `fsv_surprise_never_inflates_bits`: ingest a singleton CxId into a domain with 99 recurring events. Compute `surprise_bits(singleton)`. Assert `surprise_bits ≈ 6.64` (as expected). Inspect singleton's stored CF bytes — assert no field in the CF row contains the value `6.64` or any float near it (surprise is NOT stored). Search CF with `xxd` and confirm.
- [ ] `fsv_temporal_lead_lag_directional`: create CxId-A recurs at [100,200,300,400,500] and CxId-B at [115,215,315,415,515]. Run `temporal_cross_term(A, B, window=30)`. Assert `lead_lag_secs ≈ 15.0`. Run `temporal_cross_term(B, A, window=30)`. Assert `lead_lag_secs ≈ -15.0` (sign flip).
- [ ] All tests: `FixedClock`, seeded, `#[cfg(test)]`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `fsv_agreeing_outcomes_high_consistency` passes (≥ 0.90)
- [ ] `fsv_differing_outcomes_flaky` passes (≤ 0.60)
- [ ] `fsv_mixed_domain_self_consistency` passes (0.55–0.75)
- [ ] `fsv_frequency_raises_kernel_weight` passes (A rank > B rank)
- [ ] `fsv_surprise_never_inflates_bits` passes (`xxd` scan confirms absence)
- [ ] `fsv_temporal_lead_lag_directional` passes (sign flip confirmed)
- [ ] proptest: `oracle_self_consistency ∈ [0.0, 1.0]` for all valid domain inputs
- [ ] edge: domain with all `Insufficient` CxIds (< 3 occurrences each) → `oracle_self_consistency = 1.0` (unknown → permissive)
- [ ] fail-closed: kernel build with all frequencies = 0 → all bonuses = 0.0; no panic; kernel still built from betweenness

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-assay tests` and `cargo test -p calyx-lodestar temporal_kernel::tests` on aiwonder
- **Readback:** `cargo test -p calyx-assay -- --nocapture 2>&1` and `cargo test -p calyx-lodestar -- --nocapture 2>&1`; paste full terminal output to PH42 GitHub issue
- **Prove:** all 9 tests pass; `fsv_agreeing_outcomes_high_consistency` output prints `oracle_self_consistency: <value ≥ 0.90>`; `fsv_differing_outcomes_flaky` prints `oracle_self_consistency: <value ≤ 0.60>`; `fsv_frequency_raises_kernel_weight` prints A and B kernel scores with A ranked higher; `fsv_surprise_never_inflates_bits` prints "surprise not found in CF bytes: OK"

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
