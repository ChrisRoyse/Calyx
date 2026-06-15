# PH52 · T03 — Total correlation `n_eff` (TC + interaction information)

| Field | Value |
|---|---|
| **Phase** | PH52 — Advanced math |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/total_correlation.rs` (≤500) |
| **Depends on** | PH28 (KSG MI / entropy estimators — same estimator reused), PH29 (`n_eff` pairwise baseline — TC replaces/extends) |
| **Axioms** | A7, A9, A16, A2 |
| **PRD** | `dbprdplans/26 §5` |

## Goal

Implement `total_correlation(panel_slots) -> TCResult` where
`TC(Φ) = Σ_k H(slot_k) − H(Φ)` — the multivariate mutual information (multi-information)
— and `interaction_information(triple) -> f32` for 3-way synergy detection.
This gives a principled `n_eff` that catches multi-way redundancy the pairwise ≤0.6 rule
misses (`26 §5`). Also implements `n_eff_from_tc` as the primary source for effective rank
going forward (the pairwise graph remains the fast first-pass gate). Returns CI + sample
count; fail-closed below quorum.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn total_correlation(slots: &[SlotVectors], clock: &dyn Clock) -> Result<TCResult, AssayError>` where `SlotVectors = &[Vec<f32>]` (N slots, each with M samples)
- [ ] `TC(Φ) = Σ_k H(slot_k) − H(Φ)`: estimate each marginal entropy `H(slot_k)` via KSG (PH28); estimate joint entropy `H(Φ)` via KSG on the concatenated slot matrix; `TC = Σ H_k − H_joint`
- [ ] `n_eff_from_tc`: `n_eff = N · (1 − TC / Σ H_k)` normalized to `[1, N]`; `1` = fully redundant panel, `N` = fully independent
- [ ] `struct TCResult { tc: f32, n_eff: f32, ci_95: (f32, f32), n_samples: usize, provisional: bool }` — `provisional = n_samples < MIN_QUORUM_TC` where `MIN_QUORUM_TC = 50 * N` (A16: "fail closed below quorum, `50 × N` samples")
- [ ] `pub fn interaction_information(slot_a: &[f32], slot_b: &[f32], slot_c: &[f32], clock: &dyn Clock) -> Result<IIResult, AssayError>` — `II(A;B;C) = I(A;B) − I(A;B|C)` = positive for redundancy, negative for synergy; reuses KSG pairwise + conditional MI
- [ ] `struct IIResult { ii: f32, sign: IISign, ci_95: (f32, f32), provisional: bool }` where `IISign { Redundant | Synergistic | Unclear }`
- [ ] CI via bootstrap (500 resamples, seeded RNG seed from `Clock`)
- [ ] Keep the pairwise ≤0.6 gate in `differentiation_contract.rs` as the first-pass filter; `TC` is the thorough audit pass invoked on demand (do not replace the pairwise gate)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 3 slots with planted multi-way redundancy (slots 2 and 3 = slot 1 + noise); `TC > 0`; `n_eff < 3.0 ± 0.3`
- [ ] unit: 3 fully independent Gaussian slots (seeded); `TC ≈ 0 ± CI`; `n_eff ≈ 3.0 ± 0.3`
- [ ] unit: `interaction_information` on a planted redundant triple (two slots highly correlated with a third) → `II > 0`, `IISign::Redundant`
- [ ] unit: `interaction_information` on a planted synergistic triple (XOR-type relationship) → `II < 0`, `IISign::Synergistic`
- [ ] proptest: `n_eff ∈ [1.0, N]` always; `TC ≥ 0` always (TC is non-negative by definition)
- [ ] edge (≥3): single slot → `TC = 0`, `n_eff = 1.0`; all slots identical → `TC = max`, `n_eff ≈ 1.0`; below `MIN_QUORUM_TC` samples → `provisional = true`
- [ ] fail-closed: below `MIN_QUORUM_TC` → `CALYX_TC_INSUFFICIENT_SAMPLES` with sample count in the error; CI width > `tc` value → `IISign::Unclear`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `TCResult` JSON; `IIResult` JSON from test output
- **Readback:**
  ```
  cargo test -p calyx-assay -- total_correlation --nocapture 2>&1 | tee /tmp/ph52_tc.log
  grep "n_eff\|TC\|IISign" /tmp/ph52_tc.log
  # Planted redundant: n_eff < N; TC > 0
  # Planted independent: n_eff ≈ N; TC ≈ 0
  # Planted synergistic triple: IISign = Synergistic
  ```
- **Prove:** `n_eff < 3.0` for planted-redundant panel; `n_eff ≈ 3.0` for independent panel; `IISign::Synergistic` for XOR triple; CI bounds in log confirm non-degenerate estimates

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH52 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
