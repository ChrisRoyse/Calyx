# PH39 · T06 — Speaker similarity target FSV (0.961 mean WavLM cos)

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/identity_fsv.rs` (≤500) |
| **Depends on** | T05 (this phase) · T02 (WavLM adapter) |
| **Axioms** | A12 |
| **PRD** | `dbprdplans/09 §5b` |

## Goal

Prove the 0.961 mean WavLM speaker-similarity target: run a set of in-region
TTS outputs (matched speaker) through the `SpeakerLens`, gate them via
`guard_generate()` with the calibrated speaker `IdentityProfile`, and assert
that the mean cosine similarity to the target speaker constellation is ≥ 0.961.
This is the paper's measured value (`09 §5b`): "a reproduced voice at 0.961
mean WavLM speaker-similarity (encoder-matched)."

## Build (checklist of concrete, code-level steps)

- [ ] Write `#[test] fn fsv_speaker_similarity_target`:
      - Load the speaker `IdentityProfile` from
        `/home/croyse/calyx/data/identity_fsv/speaker_profile.json`
        (calibrated; τ_speaker on the speaker slot)
      - Load matched speaker vec from
        `/home/croyse/calyx/data/identity_fsv/matched_speaker.npy`
        (the target speaker's grounded embedding from the vault)
      - Load N ≥ 20 in-region TTS audio files from
        `/home/croyse/calyx/data/identity_fsv/tts_samples/` (wav, 16kHz)
      - Skip gracefully if directory absent
      - For each sample:
        - `embed_speaker(audio_pcm)` via `SpeakerLens`
        - Compute `cos_k = cosine(produced_speaker_vec, matched_speaker_vec)`
        - Append to `cos_scores`
        - Call `guard_generate()` and assert `Accepted` (all in-region)
      - `mean_cos = cos_scores.iter().sum::<f32>() / n`
      - `println!("mean_wavlm_speaker_similarity: {:.4}", mean_cos)`
      - `assert!(mean_cos >= 0.961,
          "FAIL: mean speaker sim {:.4} < 0.961 target (09 §5b)", mean_cos)`
- [ ] Write `#[test] fn fsv_speaker_out_of_region_rejected`:
      - Load N ≥ 5 cross-speaker audio files from
        `/home/croyse/calyx/data/identity_fsv/cross_speaker_samples/`
      - For each: assert `guard()` on the speaker slot returns `overall_pass == false`
        (different speaker is outside τ)
      - Print per-slot `(cos, tau, pass)` for each
- [ ] Write `#[test] fn fsv_stage8_exit_summary`:
      - Runs the key assertions from PH37+PH38+PH39 in a single summary test:
        1. Average-passing/slot-failing → rejected (PH37 gate)
        2. Calibrated FAR ≤ 0.01 on held-out data (PH38 gate)
        3. Mean speaker sim ≥ 0.961 (PH39 gate)
        4. Style injection → quarantined (PH39 gate)
      - Prints a summary table to stdout:
        ```
        PH37 no-flatten gate:     PASS
        PH38 injection block:     0.9952 >= 0.99 PASS
        PH39 speaker sim:         0.9634 >= 0.961 PASS
        PH39 style quarantine:    PASS
        Stage 8 Ward exit:        PASS
        ```

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `fsv_speaker_similarity_target` — asserts mean_cos ≥ 0.961 on
      aiwonder TTS samples; prints `mean_wavlm_speaker_similarity: 0.9xxx`
- [ ] unit: `fsv_speaker_out_of_region_rejected` — cross-speaker cos < τ;
      all assert `overall_pass == false`; print per-slot verdicts
- [ ] unit: `fsv_stage8_exit_summary` — prints full summary table; all 4 checks
      `PASS`; exit code 0
- [ ] edge: TTS samples directory has 0 files → test prints
      `"SKIP: no TTS samples at ..."` and returns (not panic or assertion fail)
- [ ] edge: one sample in the batch has NaN embedding (bad audio) → excluded
      from mean; warning printed; rest of batch processed

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-ward fsv_speaker -- --nocapture 2>&1`
  on aiwonder with real TTS samples and WavLM model loaded
- **Readback:**
  ```
  cargo test -p calyx-ward fsv_stage8 -- --nocapture 2>&1 | tee /tmp/ph39_speaker_fsv.txt
  grep -E "mean_wavlm|Stage 8 Ward exit|speaker sim|PASS|FAIL" /tmp/ph39_speaker_fsv.txt
  ```
- **Prove:** `mean_wavlm_speaker_similarity: 0.9xxx` ≥ 0.961; `Stage 8 Ward
  exit: PASS`; all 4 per-phase checks `PASS`; cross-speaker all `overall_pass:
  false`; attach `/tmp/ph39_speaker_fsv.txt` to PH39 GitHub issue as Stage 8
  exit evidence

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the WavLM embedding golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
