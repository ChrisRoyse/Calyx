# 16 - refusal driven expansion

- **Issue:** #883   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic before/after probe logs while #869 corpus ingest runs
- **Goal:** convert gate refusals and per-sensor deficits into ranked evidence/lens expansion actions, then verify whether a later run closed the refusal and produced new grounded hits.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue883-20260625T111607Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo"
scp issue883-20260625T111607Z.tar aiwonder:/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo.tar -C /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/refusal_expansion.rs`
- `crates/calyx-lodestar/tests/issue883_refusal_expansion_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z`
- Artifact: `/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/issue883_refusal_expansion_readback.json`
- Artifact bytes: `1620`
- Artifact SHA256: `f96e8235771b8d469713692057bc88000182e994ee23be99e8a640b86df21e53`
- Readback scalar leaves:
  - `schema_version=1`
  - `action_count=2`
  - `top_action_kind=AddLens`
  - `before_refusal_count=2`
  - `after_refusal_count=0`
  - `closed_refusal_count=2`
  - `new_grounded_count=1`
  - `closed=True`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- Refusals with deficits are turned into ranked actions.
- Lens/sensor deficit text maps to `AddLens`; evidence gaps map to evidence addition.
- Deficit floor filters low-value actions.
- Before/after verification requires both refusal reduction and new grounded hits.
- Refusal reduction without a new grounded hit does not close the expansion.
- Non-finite deficit parameters fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- The refusal-expansion planner now turns probe-matrix refusal rows into reusable, ranked expansion actions.
- The verifier produces a serializable before/after closure proof with refusal counts and new grounded hit IDs.
- The synthetic FSV proves the state-machine shape: two refusals planned, later zero refusals, one new grounded hit, `closed=True`.
- This is not yet the final #883 anchored-corpus acceptance. No real biomedical evidence has been added yet; the final issue requires a real refusal on the anchored corpus to become a grounded answer after targeted evidence addition.

## Conclusion & next step
The #883 planning and verification surface is ready. Keep #883 open until #869/#870/#871 produce the real corpus substrate, a real refusal is captured, targeted evidence/lens data is added, and the same verifier reads back a closed refusal with a new grounded answer.

## 2026-07-02 real refusal-driven expansion completion

### Root cause fixed during FSV
- The first real #883 regrounding run proved a probe-layer closure but exposed a false-success state: `calyx anchor` appended anchors while leaving the Base row `flags.ungrounded=true`.
- Fixed `AsterVault::anchor` and `AsterVault::anchor_with_ledger_entry` so appended anchors rewrite the Base row with `flags.ungrounded = anchors.is_empty()`.
- Added coverage in `crates/calyx-aster/tests/issue883_anchor_grounding_flag.rs`; focused aiwonder test readback passed.

### Real source-of-truth run
- FSV root: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z`
- Repo head: `7cb3341a81780f844481320b09e40f46b14a9d9b`
- Vault: `issue883-real-reground-20260702T091116Z`
- Vault dir: `/home/croyse/calyx/vaults/01KWH1HJ3BS09BY86RMFFB8W0R`
- Source data: local TREC-COVID parquet `/zfs/archive/calyx/datasets/trec_covid/corpus.parquet`
- Source parquet SHA256: `d76cea1b2304dbe67a1a54f7376a61de294976682a1d7d58d82de27141f3ba4a`
- Target source row: `ejv2xln0`, title `Surfactant protein-D and pulmonary host defense`
- Target CxId: `0a5307abb08f0e7c64845c93f60d9e74`
- Frontier: `Surfactant protein-D pulmonary host defense collectin SP-D`

### FSV readback
- Summary artifact: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/readback_summary.json`
- Summary SHA256: `d2abcbf2d347af555a4c5ce155d492bf66aff8b851bd5643bf349de95997c8bb`
- Before probe artifact: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/before_probe_matrix.json`
- Before probe SHA256: `a603f1f449725f1169eca2cc31736e27eec8558bee081611d3d8991d9ddb0d8a`
- After probe artifact: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/after_probe_matrix.json`
- After probe SHA256: `65598ca7acdbf3e17cf06ea1598dc99fcbc4ac50fe115a6fed3f18a17cdd98cc`
- Before readback: `status=refused`, `exit_code=2`, `accepted_hit_count=0`, `refusal_codes=[CALYX_PROBE_UNGROUNDED_HITS]`, target flags `ungrounded=true`, probe provenance `grounding:anchor_count=0 flags_ungrounded=true flags_degraded=false`.
- After readback: `status=ok`, `accepted_hit_count=5`, `refusal_count=0`, target flags `ungrounded=false`, probe provenance `grounding:anchor_count=2 flags_ungrounded=false flags_degraded=false`.
- Chain verification: before anchor `status=ok checked=4`; after anchor `status=ok checked=6`.
- Closure predicate in the summary read back as `closed=true`.

### Conclusion
#883 is complete. A real TREC-COVID biomedical evidence row first produced a persisted ungrounded-hit refusal, then the targeted grounding evidence append turned the same frontier into grounded probe hits with source metadata and clean Base flags.
