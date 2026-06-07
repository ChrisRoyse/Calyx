# PH39 · T04 — `guard_generate()` integration loop + provenance tag

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/generate.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH37 T03 · PH38 T03 |
| **Axioms** | A12, A15 |
| **PRD** | `dbprdplans/09 §5`, `09 §5b`, `09 §8` |

## Goal

Implement `guard_generate()` — the generation-time integration loop from
`09 §5`: model produces a candidate, Forge embeds per required lens, Ward gates
per required slot, on `PASS` writes "guarded:pass" provenance, on `FAIL` routes
to `NoveltyHandler`. This is the database primitive that makes identity-locked
generation work: every AI output is checked against the grounded constellation
before being accepted.

## Build (checklist of concrete, code-level steps)

- [ ] Define `GenerateInput` struct:
      `candidate_audio: Option<Vec<f32>>` (for speaker),
      `candidate_text: Option<String>` (for style/content),
      `sample_rate: u32`,
      `matched_cx_id: ConstellationId` (the grounded anchor to gate against)
- [ ] Define `GenerateOutput` enum:
      `Accepted { verdict: GuardVerdict, provenance_tag: String }` |
      `Novel { record: NoveltyRecord }` |
      `Rejected { verdict: GuardVerdict }` (for `RejectClosed`)
- [ ] Implement `guard_generate(identity_profile: &IdentityProfile,
      input: &GenerateInput, speaker_lens: &dyn Lens,
      style_lens: &dyn Lens, novelty_handler: &NoveltyHandler,
      high_stakes: bool) -> Result<GenerateOutput, WardError>`:
      - Embed `candidate_audio` via `speaker_lens.embed()` → `produced["speaker"]`
      - Embed `candidate_text` via `style_lens.embed()` → `produced["style"]`
      - Retrieve `matched_slot_cache` from `identity_profile` (pre-computed;
        no re-embed of the grounded constellation)
      - Call `guard(identity_profile.guard_profile, produced, matched, high_stakes)`
      - On `Ok(verdict)` where `overall_pass == true`:
        - Write provenance tag `"guarded:pass"` (Ledger stub until PH35)
        - Return `Ok(GenerateOutput::Accepted { verdict, provenance_tag: "guarded:pass".into() })`
      - On `Ok(verdict)` where `overall_pass == false` (can happen with non-high-stakes
        uncalibrated profile per PH38 T02 path):
        - Call `novelty_handler.handle()`; return `Novel` or `Rejected`
      - On `Err(WardError::Ood { .. })`: call `novelty_handler.handle()`; return
        `Novel { record }` or `Rejected { verdict: from error }`
      - On `Err(WardError::Provisional)`: propagate as-is (fail closed)
- [ ] `guard_generate` must never call `guard()` with a flattened multi-slot
      vector; each slot embedded separately by its own lens
- [ ] Add `/// CALYX_GUARD_OOD` doc on the OOD path; `/// guarded:pass` on
      the accept path

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: mock lenses returning in-region vecs (cos=0.85 on both slots);
      calibrated profile; `guard_generate()` returns
      `Accepted { provenance_tag: "guarded:pass" }`
- [ ] unit: mock lenses returning out-of-region vecs (cos=0.40 on style slot);
      `NewRegion` policy; returns `Novel { record }` with
      `status: AwaitingGrounding`
- [ ] unit: `RejectClosed` policy + out-of-region → returns `Rejected { .. }`;
      `WardError::Ood` embedded
- [ ] unit: uncalibrated profile + `high_stakes=true` → `Err(Provisional)`;
      no lens embeddings computed (early return)
- [ ] proptest: for any in-region input (cos ≥ τ on all slots), `guard_generate`
      always returns `Accepted`; for any out-of-region (cos < τ on any required
      slot), never returns `Accepted`
- [ ] edge: `candidate_audio = None` when speaker slot is required → lenses
      called with `LensInput::None` → `WardError::MissingSlot` from guard
- [ ] fail-closed: `novelty_handler.handle()` fails (vault write error) →
      error propagated; `Accepted` not returned for a failing candidate

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `GenerateOutput` printed as `{:?}` in test stdout
- **Readback:**
  `cargo test -p calyx-ward guard_generate -- --nocapture 2>&1 | grep -E "Accepted|Novel|Rejected|guarded:pass"`
- **Prove:** `Accepted { provenance_tag: "guarded:pass" }` in in-region test;
  `Novel { .. AwaitingGrounding .. }` in out-of-region `NewRegion` test;
  `Err(Provisional)` in high-stakes uncalibrated test; all tests `ok`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 (lens embed is Forge-touching)
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
