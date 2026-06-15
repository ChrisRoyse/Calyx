# PH27 · T05 — Blind-spot detector

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/blind_spot.rs` (≤500) |
| **Depends on** | T04 (agreement_graph, weave) · PH24 (ANN graph for k-nearest neighbors) |
| **Axioms** | A8, A16 |
| **PRD** | `dbprdplans/06 §5` |

## Goal

Implement the cross-lens blind-spot detector: a constellation that is
high-similarity in lens A but low-similarity in lens B vs its k-nearest
neighbors is a cross-lens anomaly. The detector fires a `BlindSpotAlert`
exposing the (cx_id, slot_a, slot_b, agreement_cx, agreement_nbr) tuple —
the grounded signal that a lens disagreement is real, not noise. Absorbed from
ContextGraph `search_cross_embedder_anomalies`.

## Build (checklist of concrete, code-level steps)

- [x] Define `BlindSpotAlert`: `{ cx_id: CxId, slot_a: SlotId, slot_b: SlotId, agreement_self: f32, agreement_neighborhood_mean: f32, delta: f32, severity: Low|Medium|High }`
  - `severity`: `delta > 0.5` → High; `delta > 0.3` → Medium; else Low
- [x] Implement `blind_spot_detector(cx_id, k: usize, vault, sextant, forge, clock) -> Result<Vec<BlindSpotAlert>, CalyxError>`:
  - for each active pair `(a,b)` in cx: compute `agreement_self = cos(v_a, v_b)` for this constellation
  - retrieve the `k` nearest neighbors of `cx_id` from Sextant's ANN index (use slot_a for the neighbor query)
  - for each neighbor `cx_nbr`: compute `agreement_nbr = cos(v_a_nbr, v_b_nbr)`
  - compute `agreement_neighborhood_mean` = mean over neighbors
  - if `agreement_self > 0.7` and `agreement_neighborhood_mean < 0.3` → emit `BlindSpotAlert`
  - if `agreement_self < 0.3` and `agreement_neighborhood_mean > 0.7` → emit `BlindSpotAlert` (reversed)
- [x] Implement `blind_spots(cx_id | query)` public API: takes either a known `CxId` or a query to retrieve the `CxId` first; returns `Vec<BlindSpotAlert>`; empty vec is fine (no alert = no anomaly)
- [x] Thresholds (`0.7`, `0.3`, `k=10`) configurable via `BlindSpotConfig`; default values must be exactly as stated (No-Compress List)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: plant a constellation `cx0` with `cos(v_a,v_b)=0.9`; give it 5 neighbors all with `cos(v_a,v_b)=0.1`; `blind_spot_detector(cx0, k=5)` must fire a `BlindSpotAlert` for pair `(a,b)` with `severity: High`
- [x] unit: plant a constellation `cx1` where all neighbors also have `cos(v_a,v_b)=0.9`; `blind_spot_detector(cx1)` returns empty vec (no alert)
- [x] proptest: alerts are deterministic — calling `blind_spot_detector` twice on the same inputs returns identical `Vec<BlindSpotAlert>` (all RNG seeded)
- [x] edge: fewer than `k` neighbors available → use all available; 0 neighbors → empty alerts, no panic; constellation with single active slot → empty alerts
- [x] fail-closed: unknown `CxId` → `CALYX_ASTER_NOT_FOUND`; Sextant ANN lookup failure → `CALYX_SEXTANT_INDEX_ERROR` propagated (not swallowed)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** a planted cross-lens disagreement: `cx0` has `cos(v_text, v_code) = 0.9`; its 5 nearest text-space neighbors all have `cos(v_text, v_code) ≈ 0.05`
- **Readback:**
  ```
  cargo test blind_spot_planted_disagreement -- --nocapture
  ```
  Output must include `BlindSpotAlert { cx_id: cx0, slot_a: text, slot_b: code, delta: ≈0.85, severity: High }`.
- **Prove:** run the test on aiwonder; confirm the alert fires and the delta is ≥ 0.5 (High severity). Confirm that when the planted disagreement is removed (neighbors also at 0.9), zero alerts fire.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
