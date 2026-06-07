# PH52 · T02 — Transfer entropy on recurrence streams (reuse KSG)

| Field | Value |
|---|---|
| **Phase** | PH52 — Advanced math |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/transfer_entropy.rs` (≤500) |
| **Depends on** | PH28 (KSG MI estimator — the building block), PH42 (grounded recurrence streams — the input data), PH46 (Anneal — lag parameter autotune) |
| **Axioms** | A29, A2, A16 |
| **PRD** | `dbprdplans/26 §4` |

## Goal

Implement `transfer_entropy(stream_a, stream_b, lag) -> TEResult` where
`T(A→B) = I(B_future; A_past | B_past)` — the information-theoretic directional causal
influence of event-stream A on event-stream B. This is the rigorous causal-discovery
primitive for the Oracle (`21`) — "the provable version of temporal co-occurrence =
causality, and it's your fifth element (information) made directional" (`26 §4`).
Reuses the Assay KSG estimator (PH28) applied to time-lagged recurrence streams.
Returns `provisional` when the series is too short. Lag is Anneal-autotuned over
`[1, 2, 4, 8]` time units; reports the max-TE lag.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn transfer_entropy(stream_a: &RecurrenceStream, stream_b: &RecurrenceStream, lag: usize, clock: &dyn Clock) -> Result<TEResult, AssayError>` where `RecurrenceStream = &[(Timestamp, f32)]` (time, event indicator)
- [ ] Construct the joint vector `(B_future, A_past, B_past)` at each time point `t`: `B_future = b[t+lag]`, `A_past = a[t-lag..t]`, `B_past = b[t-lag..t]` (windowed; configurable window size `W`)
- [ ] `T(A→B) = I(B_future; [A_past, B_past]) − I(B_future; B_past)` — both MIs estimated via KSG (PH28 `ksg_mi` function)
- [ ] Return `TEResult { t_a_to_b: f32, t_b_to_a: f32, dominant_direction: Direction, ci_95: (f32, f32), lag: usize, provisional: bool, n_samples: usize }`
- [ ] `provisional = true` if `n_samples < MIN_QUORUM` (default 30); tag in result (A16)
- [ ] `dominant_direction: Direction` = `A_to_B` if `t_a_to_b > t_b_to_a`, `B_to_A` if reversed, `Unclear` if within CI overlap
- [ ] `pub fn transfer_entropy_sweep(a: &RecurrenceStream, b: &RecurrenceStream, lags: &[usize], clock: &dyn Clock) -> Vec<TEResult>` — sweep over lags; report max-TE lag (Anneal can consume this for autotuning)
- [ ] CI via bootstrap (500 resamples, seeded); `ci_95 = (p2.5, p97.5)` of bootstrap distribution
- [ ] Fail-closed below quorum: `provisional = true` + `CALYX_TE_INSUFFICIENT_SAMPLES`; do not suppress the result entirely — return it with flag

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: planted causal A→B with fixed lag=2 (A fires, then B fires 2 steps later; 100 pairs; seeded); `t_a_to_b > t_b_to_a + 0.1` ± CI; `dominant_direction = A_to_B`
- [ ] unit: independent streams (A and B sampled independently) → `t_a_to_b ≈ 0` and `t_b_to_a ≈ 0` within CI; `dominant_direction = Unclear`
- [ ] unit: `t_b_to_a` on the planted A→B dataset is significantly lower than `t_a_to_b` (asymmetry; `26 §4`)
- [ ] proptest: `TEResult.provisional = (n_samples < 30)` always holds; `ci_95.0 ≤ t_a_to_b ≤ ci_95.1`
- [ ] edge (≥3): 0-length stream → `CALYX_TE_INSUFFICIENT_SAMPLES`; single event → `provisional = true`; simultaneous events (lag = 0) → handled without division-by-zero
- [ ] fail-closed: below `MIN_QUORUM` → `provisional = true` + `CALYX_TE_INSUFFICIENT_SAMPLES` code; never silently zero-fills a non-provisional result

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `TEResult` JSON from `calyx readback transfer_entropy --stream_a <a_id> --stream_b <b_id> --lag 2`
- **Readback:**
  ```
  cargo test -p calyx-assay -- transfer_entropy --nocapture 2>&1 | tee /tmp/ph52_te.log
  grep "t_a_to_b\|t_b_to_a\|dominant_direction" /tmp/ph52_te.log
  # Planted A→B: dominant_direction = "A_to_B"; t_a_to_b > t_b_to_a
  ```
- **Prove:** `t_a_to_b > t_b_to_a` for planted dataset; `dominant_direction = "A_to_B"`; CI does not overlap zero for `t_a_to_b`; independent streams show both near 0

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH52 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
