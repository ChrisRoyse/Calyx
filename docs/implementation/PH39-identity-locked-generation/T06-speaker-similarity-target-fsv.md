# PH39 · T06 — Speaker similarity target FSV (0.961 mean WavLM cos)

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/identity_fsv.rs` facade + `tests/identity_fsv/speaker_similarity.rs` + `tests/identity_fsv/speaker_similarity/support.rs` (each ≤500) |
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

- [x] Write `#[test] fn fsv_stage8_speaker_similarity_target_writes_readbacks`:
      - Load the speaker `IdentityProfile` from
        `/home/croyse/calyx/data/identity_fsv/speaker_tts_espeak_ng_20260609_v2/speaker_profile.json`
        (calibrated; τ_speaker on the speaker slot)
      - Load matched speaker audio from documented f32-le fixture bytes and
        embed it with the pinned WavLM model
      - Load N ≥ 20 in-region TTS audio files from
        `/home/croyse/calyx/data/identity_fsv/speaker_tts_espeak_ng_20260609_v2/tts_samples/`
        (little-endian f32 PCM, 22.05 kHz, generated from deterministic eSpeak
        WAVs with SHA-256 manifest readback)
      - Missing directory or fewer than 20 valid samples is setup failure, not a
        passing skip
      - For each sample:
        - `embed_speaker(audio_pcm)` via `SpeakerLens`
        - Compute `cos_k = cosine(produced_speaker_vec, matched_speaker_vec)`
        - Append to `cos_scores`
        - Call `guard_generate()` and assert `Accepted` (all in-region)
      - `mean_cos = cos_scores.iter().sum::<f32>() / n`
      - `println!("mean_wavlm_speaker_similarity: {:.4}", mean_cos)`
      - `assert!(mean_cos >= 0.961,
          "FAIL: mean speaker sim {:.4} < 0.961 target (09 §5b)", mean_cos)`
- [x] Write cross-speaker reject proof in the same FSV test:
      - Load N ≥ 5 cross-speaker audio files from
        `/home/croyse/calyx/data/identity_fsv/cross_speaker_samples/`
      - For each: assert `guard()` on the speaker slot returns `overall_pass == false`
        (different speaker is outside τ)
      - Print per-slot `(cos, tau, pass)` for each
- [x] Write Stage 8 exit summary JSON:
      - Runs the key assertions from PH37+PH38+PH39 in a single summary test:
        1. Average-passing/slot-failing → rejected (PH37 gate)
        2. Calibrated FAR ≤ 0.01 on held-out data (PH38 gate)
        3. Mean speaker sim ≥ 0.961 (PH39 gate)
        4. Style injection → quarantined (PH39 gate)
      - Prints a summary table to stdout:
        ```
        PH37 no-flatten gate:     PASS
        PH38 injection block:     0.9952 >= 0.99 PASS
        PH39 speaker sim:         0.9882728457450867 >= 0.9610000252723694 PASS
        PH39 style quarantine:    PASS
        Stage 8 Ward exit:        PASS
        ```

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `fsv_stage8_speaker_similarity_target_writes_readbacks` — asserts mean_cos ≥ 0.961 on
      aiwonder TTS samples; prints
      `mean_wavlm_speaker_similarity: 0.9882728457450867`
- [x] unit: cross-speaker cos < τ;
      all assert `overall_pass == false`; print per-slot verdicts
- [x] unit: Stage 8 summary readback — all 4 checks
      `PASS`; exit code 0
- [x] edge: TTS samples directory has 0 files → fail closed with a setup error
      before writing a PASS summary
- [x] edge: malformed f32-le bytes → fail closed before embedding
- [x] edge: one sample in the batch has NaN embedding (bad audio) → fail closed
      or quarantine the sample explicitly; never exclude it silently from the
      mean used for the 0.961 claim

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue274-ph39-t06-<date>/` containing the
  captured cargo log, per-sample speaker verdict JSON, mean-similarity summary
  JSON, cross-speaker rejection readback JSON, Stage 8 summary JSON, and
  SHA-256 manifest. Stdout is only one captured artifact, not the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue274-ph39-t06-<date>
  mkdir -p "$root"
  cargo test -p calyx-ward fsv_stage8 -- --nocapture 2>&1 | tee "$root/ph39-speaker-fsv.log"
  grep -E "mean_wavlm|Stage 8 Ward exit|speaker sim|PASS|FAIL" "$root/ph39-speaker-fsv.log"
  xxd -g 1 "$root/mean-speaker-sim-readback.json" | head -32
  xxd -g 1 "$root/stage8-summary-readback.json" | head -32
  sha256sum "$root"/* | sort
  ```
- **Prove:** `mean_wavlm_speaker_similarity: 0.9882728457450867` ≥
  `0.9610000252723694`; `Stage 8 Ward exit: PASS`; all 4 per-phase checks
  `PASS`; cross-speaker all `overall_pass: false`; attach the root path,
  hashes, and durable JSON readback excerpts to PH39 and the Stage 8 exit issue
  as evidence

## FSV evidence (2026-06-09)

- Root:
  `/home/croyse/calyx/data/fsv-issue274-ph39-t06-20260609-8e29b51-v2-cpu-ort126`
- Fixture:
  `/home/croyse/calyx/data/identity_fsv/speaker_tts_espeak_ng_20260609_v2`
  (`SHA256SUMS.txt` verified; profile SHA-256
  `7f8844852cb681ac8b16af5f5fea963bf71af19e65896ed7b4b28e39e9e06686`)
- Provider/model: `cpu_explicit,no_cuda` over pinned WavLM model SHA-256
  `22a38bdd854a11db171357cb997156511697d2f2c621d1262c82ba91b873d08b`.
  #270 remains the CPU↔CUDA parity proof on the WavLM golden set; this T06
  target proof uses CPU explicit for long deterministic TTS samples.
- Readback:
  mean speaker similarity `0.9882728457450867` ≥ target
  `0.9610000252723694`; in-region min/max
  `0.9850643873214722` / `0.990755558013916`; cross-speaker cos range
  `0.9233567714691162`–`0.9288811087608337`; five cross-speaker records read
  back as `Rejected`; Stage 8 summary readback has PH37/PH38/PH39 speaker/PH39
  style all `pass: true`.
- Edge readbacks: empty TTS dir, malformed f32-le bytes, and NaN f32-le sample
  all fail closed and are recorded in `edge-failclosed-readback.json`.
- Full manifest SHA-256:
  `0166503a36486b37a0422c84befd77aeed58ce168ac9083aad87b141a640e589`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the WavLM embedding golden set (#270)
- [x] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
