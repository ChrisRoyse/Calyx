# PH47 · T02 — Candidate lens synthesis (`CandidateLens` + commission spec)

| Field | Value |
|---|---|
| **Phase** | PH47 — Lens Proposal (Sufficiency Deficit) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/propose/candidate_synth.rs` (≤500) |
| **Depends on** | T01 (DeficitMap — drives synthesis target) |
| **Axioms** | A7 |
| **PRD** | `dbprdplans/12 §5` |

## Goal

Given a `DeficitMap`, produce a `CandidateLens`: either an algorithmic lens
constructed from the corpus (PCA, time-lag, frequency-domain decomposition for
temporal deficits) or a commission spec for an external embedding (TEI endpoint
or ONNX model targeting the under-represented modality). Algorithmic synthesis
is always attempted first (fast, no external dep); commission-on-corpus only if
algorithmic synthesis cannot close the modality gap.

## Build (checklist of concrete, code-level steps)

- [ ] `enum CandidateLens { Algorithmic { kind: AlgorithmicKind, params: AlgParams }, Commission { spec: CommissionSpec } }` where `AlgorithmicKind { PCA, TimeLag, FrequencyBand, TFIDF }` and `CommissionSpec { target_modality: ModalityId, endpoint: Option<Url>, model_id: Option<String>, description: String }`.
- [ ] `fn synthesize_algorithmic(deficit: &DeficitMap, corpus_sample: &[Constellation]) -> Option<CandidateLens>` — tries each `AlgorithmicKind` in order of cheapness; returns the first that is computable on `corpus_sample` and targets the top gap; returns `None` if none applicable.
- [ ] `fn build_commission_spec(deficit: &DeficitMap) -> CandidateLens` — constructs a `Commission` spec targeting the top `underrepresented_modality`; does NOT call the external endpoint (that happens in the Registry capability card, T03); just specifies what to commission.
- [ ] `fn synthesize(deficit: &DeficitMap, corpus_sample: &[Constellation]) -> CandidateLens` — calls `synthesize_algorithmic`; if `None`, calls `build_commission_spec`.
- [ ] `fn describe(candidate: &CandidateLens) -> String` — human-readable summary for Ledger entry.
- [ ] Corpus sample size capped at 1000 constellations for algorithmic synthesis (budget-aware).
- [ ] Algorithmic lens construction is deterministic given the same corpus sample + seed.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `DeficitMap` with `top_gap` in temporal domain → `synthesize_algorithmic` returns `AlgorithmicKind::TimeLag` candidate.
- [ ] unit: `DeficitMap` with gap in audio modality (no algorithmic kind applicable) → `synthesize` returns `Commission { target_modality: Audio }`.
- [ ] proptest: for any `DeficitMap`, `synthesize` returns a `CandidateLens` (never panics); `Algorithmic` kind is one of the defined enum variants.
- [ ] edge: empty `corpus_sample` → `synthesize_algorithmic` returns `None`; falls back to commission spec; `DeficitMap` with no underrepresented modality → `synthesize` returns a PCA candidate (default fallback).
- [ ] fail-closed: `corpus_sample` read fails → `CALYX_ASTER_CF_UNAVAILABLE`; `synthesize` returns the error (no silent synthetic-corpus generation).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `CandidateLens` returned by `synthesize` for a given `DeficitMap`.
- **Readback:** `calyx anneal propose-preview --anchor <anchor_id>` — prints the `CandidateLens` description that would be synthesized for the current deficit.
- **Prove:** set up a synthetic corpus with known temporal structure; create a deficit for a temporal anchor class; `propose-preview` shows `AlgorithmicKind::TimeLag` candidate with non-empty params; separately test a modality-gap corpus → `propose-preview` shows `Commission` spec.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH47 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
