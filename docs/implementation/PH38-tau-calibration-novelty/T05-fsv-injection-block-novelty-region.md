# PH38 - T05 - FSV: injection corpus blocked >=99% at calibrated FAR + valid-novelty -> new region

| Field | Value |
|---|---|
| **Phase** | PH38 - tau Calibration (Conformal) + Novelty -> New Region |
| **Stage** | S8 - Ward Gtau Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/ph38_injection_fsv.rs` (<=500) |
| **Depends on** | T04 (PH38 core); blindspots T06/T07 are tracked separately |
| **Axioms** | A2, A12, A16 |
| **PRD** | `dbprdplans/09 S2`, `09 S3` |

**STATUS:** DONE / FSV-signed-off in #268; held-out split hardening signed off
in #352. Latest implementation commit: `210d995`. Evidence root:
`/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995`.

## Goal

Provide the PH38 exit-gate FSV harness: calibrate on the real prompt-injection
corpus `train` split on aiwonder, report the held-out `test` injection block
rate separately from calibration FAR and whole-corpus block rate, and assert the
held-out block rate is >=99%. Separately verify that a valid-novelty input
(outside all tau balls) fires `NoveltyAction::NewRegion` and the novel
constellation record is written and readable from the vault CF.

## Build (checklist of concrete, code-level steps)

- [x] Write ignored aiwonder FSV fixture
      `ph38_t05_fsv_fixture_writes_readback_artifacts`:
      - Load injection corpus from
        `/home/croyse/calyx/data/injection_corpus/vectors.jsonl` (each line has
        `id`, `split`, `row_idx`, `label`, `slot`, `text_sha256`, and `vec`).
        On aiwonder this file is a required prerequisite: if absent, the task is
        setup work and the FSV writes a clear missing-corpus error.
      - Read `/home/croyse/calyx/data/injection_corpus/manifest.json` and verify
        the pinned corpus/hash/model provenance before scoring.
      - Calibrate a `GuardProfile` in the fixture with `calibrate()` against the
        real corpus `train` split scores; the profile is not prebuilt or
        hand-edited.
      - For each held-out `test` injection vector, call
        `guard(profile, produced={content: vec},
        matched=train_benign_centroid, high_stakes=false)`.
      - Count `blocked = verdicts where overall_pass == false`.
      - `heldout_block_rate = blocked / heldout_injection_total`.
      - Print `heldout_injection_block_rate`.
      - Assert `heldout_block_rate >= 0.99`.
- [x] In the same FSV fixture verify valid novelty:
      - Construct a vector with cos = 0.30 to the known-good anchor.
      - `guard()` returns `overall_pass = false`.
      - `NoveltyHandler::handle()` with `NewRegion` policy returns
        `NoveltyRecord { status: AwaitingGrounding }`.
      - Write to a file-backed `VaultSink` under the durable FSV root; call
        `novel_regions(since=0)` and assert the record appears.
- [x] Write non-ignored edge/unit tests for deterministic novelty-vector
      construction, missing-corpus typed error, and file-backed novelty sink
      readback.

## Tests (synthetic, deterministic: known input -> known bytes/number)

- [x] FSV fixture: held-out injection corpus block rate asserts
      `heldout_block_rate >= 0.99`; writes held-out and whole-corpus block-rate
      JSON to the durable evidence root.
- [x] FSV fixture: valid novelty opens new region; asserts `AwaitingGrounding`,
      record in sink, `novel_id` UUID non-nil.
- [x] FSV fixture: calibration provenance complete; `estimator`, `target_far`,
      achieved `far`, `frr`, confidence, tau, profile JSON, and vectors SHA-256
      are written to durable JSON.
- [x] edge: injection corpus file absent on aiwonder -> fail with a typed
      missing-prerequisite error and record the missing path in the evidence
      root.

## FSV (read the bytes on aiwonder: the truth gate)

- **SoT:** durable aiwonder evidence root containing the captured cargo log,
  `split-readback.json`, `heldout-block-rate.json`,
  `whole-corpus-block-rate.json`, calibration provenance JSON, corpus readback
  JSON, novel-region vault/CF readback, missing-corpus edge JSON, and SHA-256
  manifest. Stdout is only one captured artifact, not the verdict.
- **Readback:**
  ```bash
  root=/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995
  test ! -e "$root"
  CALYX_WARD_PH38_T05_FSV_DIR="$root" \
    CALYX_WARD_INJECTION_CORPUS_DIR=/home/croyse/calyx/data/injection_corpus \
    cargo test -p calyx-ward --test ph38_injection_fsv \
      -- --ignored --nocapture ph38_t05_fsv_fixture_writes_readback_artifacts \
    2>&1 | tee "$root.ph38-fsv.log"
  xxd -g 1 "$root/heldout-block-rate.json" | head -32
  xxd -g 1 "$root/split-readback.json" | head -32
  sha256sum "$root"/* | sort
  ```
- **Prove:** `heldout_injection_block_rate=1.000000` on `test` with 60/60
  injection rows blocked; train calibration reports `calibration_far=0.009852`;
  whole-corpus block rate remains separate (`0.99239546`); novelty readback
  shows `"status": "AwaitingGrounding"` and a UUID `novel_id`; byte reads and
  hashes prove the durable JSON.

**Actual #268 readback:** `block_rate=0.99239546`, `blocked=261`,
`injection_total=263`, `tau=0.76665336`,
`estimator=conformal_quantile_v1`, `novel_status=AwaitingGrounding`,
`vectors_sha256=d8ec5f1b2bd117be8c4dd1a0915d75236629d12d22b11146692b1a395468dbad`.

**Actual #352 readback:** `calibration_split=train` (`343` benign, `203`
injection), `heldout_split=test` (`56` benign, `60` injection),
`heldout_block_rate=1.0`, `heldout_blocked=60`, `heldout_passed=0`,
`calibration_far=0.009852216579020023`, `whole_corpus_block_rate=0.99239546`,
`tau=0.76582265`,
`vectors_sha256=d8ec5f1b2bd117be8c4dd1a0915d75236629d12d22b11146692b1a395468dbad`.

Hashes:
- `case-summary.json`
  `4461402a6c63238d3d8596beae14fead775003ea5915269ea0f3e8838c3d4be5`
- `heldout-block-rate.json`
  `47a8a6d287ea19f3c77cc18a751348d59a85a7984785e66b7dcc1f84b59b6ca1`
- `split-readback.json`
  `e5c72c6b73bf70f67a1104d5026005a2061b118c24800efcb8c564c2ea7b0978`
- `calibration-provenance.json`
  `7d9fe1d868a1da4a60a22dc22b6def8f2ac48644d6bc44c6529ee10641c8ee9c`
- `whole-corpus-block-rate.json`
  `71efe2ead1333f12323bfc6567464db8a2d64a6cdbab6e82ce6b8c10f1016dea`
- log
  `a58682bcca8d72ea68a150473232dc56119818d4450c3e29e043d49334c151eb`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] file(s) <= 500 lines (line-count gate).
- [x] FSV evidence attached to the PH38 GitHub issue.
- [x] no anti-pattern (DOCTRINE S9): no flatten / no `C(N,2)` past DPI /
      nothing "trusted" without grounding / no frozen-lens mutation /
      no harness-as-FSV.
