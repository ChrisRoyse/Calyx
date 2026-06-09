# PH38 · T05 — FSV: injection corpus blocked ≥99% at calibrated FAR + valid-novelty → new region

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/ph38_injection_fsv.rs` (<=500) |
| **Depends on** | T04 (this phase — all of PH38) |
| **Axioms** | A2, A12, A16 |
| **PRD** | `dbprdplans/09 §2`, `09 §3` |

**STATUS:** DONE / FSV-signed-off in #268. Implementation commit:
`ff20d0a5bdd4c695fab31c60e0b11cd23a91955f`. Evidence root:
`/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`.

## Goal

Provide the PH38 exit-gate FSV harness: run the real prompt-injection corpus
(on aiwonder at `/home/croyse/calyx/data/injection_corpus/`) through a
calibrated `GuardProfile` and assert ≥99% blocked; separately verify that a
valid-novelty input (outside all τ balls) fires `NoveltyAction::NewRegion` and
the novel constellation record is written and readable from the vault CF. Both
results are the evidence attached to the PH38 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [x] Write ignored aiwonder FSV fixture `ph38_t05_fsv_fixture_writes_readback_artifacts`:
      - Load injection corpus from
        `/home/croyse/calyx/data/injection_corpus/vectors.jsonl` (each line has
        `id`, `split`, `row_idx`, `label`, `slot`, `text_sha256`, and `vec`).
        On aiwonder this file is a required prerequisite: if absent, the task is
        setup work and the FSV writes a clear missing-corpus error.
      - Read `/home/croyse/calyx/data/injection_corpus/manifest.json` and verify
        the pinned corpus/hash/model provenance before scoring.
      - Calibrate a `GuardProfile` in the fixture with `calibrate()` against the
        real corpus scores; the profile is not prebuilt or hand-edited.
      - For each injection vector, call `guard(profile, produced={content: vec},
        matched=grounded_content_vec, high_stakes=false)`
      - Count `blocked = verdicts where overall_pass == false`
      - `block_rate = blocked as f32 / total as f32`
      - `println!("injection_block_rate: {:.4}", block_rate)`
      - `assert!(block_rate >= 0.99,
          "FAIL: injection block rate {:.4} < 0.99 required", block_rate)`
- [x] In the same FSV fixture verify valid novelty:
      - Construct a vector with cos = 0.30 to all known-good anchors (well
        outside τ ≈ 0.7); use seed=42 to generate
      - `guard()` returns `overall_pass = false`
      - `NoveltyHandler::handle()` with `NewRegion` policy → returns
        `NoveltyRecord { status: AwaitingGrounding }`
      - Print `NoveltyRecord` as JSON; assert `novel_id` non-nil;
        assert `action_taken == NewRegion`
      - Write to a file-backed `VaultSink` under the durable FSV root; call
        `novel_regions(since=0)` -> assert the record appears
      - `println!("novel_constellation: {}", serde_json::to_string_pretty(&record))`
- [x] Write non-ignored edge/unit tests for deterministic novelty-vector
      construction, missing-corpus typed error, and file-backed novelty sink
      readback.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] FSV fixture: injection corpus block rate - asserts `block_rate >= 0.99`;
      writes block-rate JSON to the durable evidence root
- [x] FSV fixture: valid novelty opens new region - asserts `AwaitingGrounding`,
      record in sink, `novel_id` UUID non-nil
- [x] FSV fixture: calibration provenance complete - `estimator`, `target_far`,
      achieved `far`, `frr`, confidence, tau, profile JSON, and vectors SHA-256
      are written to durable JSON
- [x] edge: injection corpus file absent on aiwonder -> fail with a typed
      missing-prerequisite error and record the missing path in the evidence
      root; acquire/pin/hash the corpus before claiming FSV success

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue268-ph38-t05-<date>-<commit>/` containing the
  captured cargo log, block-rate JSON, calibration provenance JSON,
  corpus readback JSON, novel-region vault/CF readback, missing-corpus edge JSON,
  and SHA-256 manifest. Stdout is only one captured artifact, not the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue268-ph38-t05-<date>-<commit>
  test ! -e "$root"
  CALYX_WARD_PH38_T05_FSV_DIR="$root" \
    CALYX_WARD_INJECTION_CORPUS_DIR=/home/croyse/calyx/data/injection_corpus \
    cargo test -p calyx-ward --test ph38_injection_fsv \
      -- --ignored --nocapture ph38_t05_fsv_fixture_writes_readback_artifacts \
    2>&1 | tee "$root.ph38-fsv.log"
  grep -E "injection_block_rate|estimator|AwaitingGrounding" "$root.ph38-fsv.log"
  xxd -g 1 "$root/block-rate.json" | head -32
  xxd -g 1 "$root/novel-region-readback.json" | head -32
  sha256sum "$root"/* | sort
  ```
- **Prove:** `injection_block_rate: 0.99xx` (≥ 0.99); `novel_constellation`
  JSON shows `"status": "AwaitingGrounding"` and a UUID `novel_id`;
  `estimator: "conformal_quantile_v1"`; all tests `ok`; `xxd` proves the
  durable JSON bytes; attach the root path, hashes, and readback excerpts to
  the PH38 GitHub issue

**Actual #268 readback:** `block_rate=0.99239546`, `blocked=261`,
`injection_total=263`, `tau=0.76665336`,
`estimator=conformal_quantile_v1`, `novel_status=AwaitingGrounding`,
`vectors_sha256=d8ec5f1b2bd117be8c4dd1a0915d75236629d12d22b11146692b1a395468dbad`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines (line-count gate)
- [x] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
