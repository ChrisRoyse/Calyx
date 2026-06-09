# PH38 · T05 — FSV: injection corpus blocked ≥99% at calibrated FAR + valid-novelty → new region

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/calibrate_unit.rs` (≤500) |
| **Depends on** | T04 (this phase — all of PH38) |
| **Axioms** | A2, A12, A16 |
| **PRD** | `dbprdplans/09 §2`, `09 §3` |

## Goal

Provide the PH38 exit-gate FSV harness: run the real prompt-injection corpus
(on aiwonder at `/home/croyse/calyx/data/injection_corpus/`) through a
calibrated `GuardProfile` and assert ≥99% blocked; separately verify that a
valid-novelty input (outside all τ balls) fires `NoveltyAction::NewRegion` and
the novel constellation record is written and readable from the vault CF. Both
results are the evidence attached to the PH38 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [ ] Write `#[test] fn fsv_injection_corpus_block_rate`:
      - Load injection corpus from
        `/home/croyse/calyx/data/injection_corpus/vectors.jsonl` (each line:
        `{"slot": "content", "vec": [...]}`) — test skips with `eprintln!` if
        file absent (non-aiwonder environments)
      - Load a calibrated `GuardProfile` from
        `/home/croyse/calyx/data/injection_corpus/guard_profile.json`
        (pre-built by calibrate() against the corpus)
      - For each injection vector, call `guard(profile, produced={content: vec},
        matched=grounded_content_vec, high_stakes=false)`
      - Count `blocked = verdicts where overall_pass == false`
      - `block_rate = blocked as f32 / total as f32`
      - `println!("injection_block_rate: {:.4}", block_rate)`
      - `assert!(block_rate >= 0.99,
          "FAIL: injection block rate {:.4} < 0.99 required", block_rate)`
- [ ] Write `#[test] fn fsv_valid_novelty_opens_new_region`:
      - Construct a vector with cos = 0.30 to all known-good anchors (well
        outside τ ≈ 0.7); use seed=42 to generate
      - `guard()` returns `overall_pass = false`
      - `NoveltyHandler::handle()` with `NewRegion` policy → returns
        `NoveltyRecord { status: AwaitingGrounding }`
      - Print `NoveltyRecord` as JSON; assert `novel_id` non-nil;
        assert `action_taken == NewRegion`
      - Write to in-memory `VaultSink`; call `novel_regions(since=0)` →
        assert the record appears
      - `println!("novel_constellation: {}", serde_json::to_string_pretty(&record))`
- [ ] Write `#[test] fn fsv_calibration_provenance_complete`:
      - Run `calibrate()` on 200-sample synthetic data (seed=42);
        assert `CalibrationMeta` fields: `estimator == "conformal_quantile_v1"`,
        `far ≤ 0.01`, `confidence == 0.95`, `corpus_hash` is 32 non-zero bytes,
        `ts > 0`
      - Print `CalibrationMeta` as JSON

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `fsv_injection_corpus_block_rate` — asserts `block_rate ≥ 0.99`;
      prints rate to stdout; skips gracefully on non-aiwonder
- [ ] unit: `fsv_valid_novelty_opens_new_region` — asserts `AwaitingGrounding`,
      record in sink, `novel_id` UUID non-nil
- [ ] unit: `fsv_calibration_provenance_complete` — all 5 CalibrationMeta fields
      correct; JSON printed
- [ ] edge: injection corpus file absent → test prints
      `"SKIP: injection corpus not found at /home/croyse/..."` and exits with
      `return` (not panic; CI/dev machines pass)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue268-ph38-t05-<date>/` containing the
  captured cargo log, block-rate JSON, calibration provenance JSON,
  novel-region vault/CF readback, and SHA-256 manifest. Stdout is only one
  captured artifact, not the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue268-ph38-t05-<date>
  mkdir -p "$root"
  cargo test -p calyx-ward fsv -- --nocapture 2>&1 | tee "$root/ph38-fsv.log"
  grep -E "injection_block_rate|novel_constellation|estimator|AwaitingGrounding" "$root/ph38-fsv.log"
  xxd -g 1 "$root/block-rate.json" | head -32
  xxd -g 1 "$root/novel-region-readback.json" | head -32
  sha256sum "$root"/* | sort
  ```
- **Prove:** `injection_block_rate: 0.99xx` (≥ 0.99); `novel_constellation`
  JSON shows `"status": "AwaitingGrounding"` and a UUID `novel_id`;
  `estimator: "conformal_quantile_v1"`; all tests `ok`; `xxd` proves the
  durable JSON bytes; attach the root path, hashes, and readback excerpts to
  the PH38 GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
