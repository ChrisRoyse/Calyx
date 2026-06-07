# PH40 · T04 — Causal confidence gate (×1.10 / ×0.85)

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/causal_gate.rs` (≤500) |
| **Depends on** | T03 (this phase) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §3` |

## Goal

Implement the causal confidence gate that multiplies a hit's boosted score by
×1.10 when the hit's causal confidence is `High` and by ×0.85 when it is `Low`.
The gate is applied after `apply_temporal_boost` as a final post-processing pass.
High-confidence hits are those whose causal anchor (if present) has been
corroborated by the Ledger or the Ward guard; low-confidence hits are those whose
causal anchor is unverified or contested.

## Build (checklist of concrete, code-level steps)

- [ ] Define `CausalConfidence` enum: `High | Neutral | Low | Absent` — `Absent` means no causal anchor present; treated as `Neutral` in gate (mult = 1.0)
- [ ] Implement `causal_gate_mult(conf: CausalConfidence, cfg: &BoostConfig) -> f32`: `High` → `cfg.causal_high_mult` (default 1.10); `Low` → `cfg.causal_low_mult` (default 0.85); `Neutral | Absent` → 1.0
- [ ] Implement `apply_causal_gate(hits: Vec<Hit>, cfg: &BoostConfig) -> Vec<Hit>`: for each hit, multiply score by `causal_gate_mult(hit.causal_confidence, cfg)`; re-sort descending; attach `CausalConfidence` to hit for explain
- [ ] `CausalConfidence` derives from the `Hit`'s anchor provenance: expose a `fn derive_causal_confidence(hit: &Hit) -> CausalConfidence` that inspects `hit.anchor_ledger_ref` — if corroborated ledger entry present → `High`; if contested → `Low`; if missing → `Absent`
- [ ] The final combined pipeline function: `temporal_search_pipeline(hits, policy, query_time, tz_offset, clock) -> Vec<Hit>` that chains: window-filter → `apply_temporal_boost` → `apply_causal_gate`
- [ ] All multipliers stay within `[0.0, 10.0]` — validate `BoostConfig` at construction

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `causal_gate_mult(High, default_cfg)` → 1.10 exactly
- [ ] unit: `causal_gate_mult(Low, default_cfg)` → 0.85 exactly
- [ ] unit: `causal_gate_mult(Neutral, default_cfg)` → 1.0 exactly
- [ ] unit: `causal_gate_mult(Absent, default_cfg)` → 1.0 exactly
- [ ] unit: `apply_causal_gate` on three hits with scores [0.9 High, 0.8 Neutral, 0.7 Low] → final scores [0.99, 0.80, 0.595]; re-ranked [0.99, 0.80, 0.595]
- [ ] unit: `temporal_search_pipeline` on a 3-hit synthetic set (High/Neutral/Low causal confidence, two in time window) → correct 2-hit window-filtered, boosted, gated result
- [ ] proptest: `apply_causal_gate` is a permutation of input hit IDs (no hits added or removed)
- [ ] edge: empty hit list → empty result without panic
- [ ] fail-closed: `BoostConfig { causal_high_mult: -0.5 }` → `CALYX_TEMPORAL_INVALID_BOOST_CONFIG`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** final ranked hit list from `temporal_search_pipeline` with explain output showing per-hit `CausalConfidence` and multiplier applied
- **Readback:** `calyx readback temporal_search --explain --clock-fixed 1_000_000` on a synthetic vault with three constellations with known anchor corroboration states; print final ranked scores
- **Prove:** high-confidence hit score = `(content_score + temporal_boost) * 1.10` exactly (verify arithmetic to 4 decimal places); low-confidence hit score = `(content_score + temporal_boost) * 0.85` exactly; explain output contains `causal_confidence` field

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH40 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
